use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use blake3::Hasher;
use globset::{Glob, GlobSetBuilder};
use ignore::WalkBuilder;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sifter_codeintel::PluginRegistry;
use sifter_codeintel_rust::RustPlugin;
use sifter_core::config::{Config, matching_contexts};

#[derive(Debug, Clone, Serialize)]
pub struct IndexedFile {
    pub docid: String,
    pub collection: String,
    pub path: String,
    pub virtual_path: String,
    pub kind: String,
    pub title: String,
    pub language: Option<String>,
    pub context: Option<String>,
    pub content: String,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub docid: String,
    pub file: String,
    pub collection: String,
    pub kind: String,
    pub title: String,
    pub context: Option<String>,
    pub score: f64,
    pub snippet: String,
    pub line_start: usize,
    pub line_end: usize,
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_content: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolHit {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub collection: String,
    pub line_start: usize,
    pub line_end: usize,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedHit {
    pub file: String,
    pub collection: String,
    pub score: usize,
    pub shared_symbols: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub indexed_files: usize,
    pub indexed_docs: usize,
    pub indexed_code: usize,
    pub collections: usize,
    pub has_vector_index: bool,
    pub vector_runtime: String,
}

#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    pub kind: Option<SearchKind>,
    pub include_full_content: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Doc,
    Code,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSlice {
    pub start: usize,
    pub max_lines: Option<usize>,
    pub line_numbers: bool,
}

#[derive(Debug, Clone)]
struct ChunkRecord {
    chunk_docid: String,
    title: String,
    content: String,
    line_start: usize,
    line_end: usize,
}

pub struct Store {
    connection: Connection,
    plugins: PluginRegistry,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let connection =
            Connection::open(path).with_context(|| format!("failed to open {}", path.display()))?;
        let mut plugins = PluginRegistry::new();
        plugins.register(RustPlugin);
        let store = Self {
            connection,
            plugins,
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn rebuild(&mut self, config: &Config) -> Result<usize> {
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM files", [])?;
        transaction.execute("DELETE FROM chunks", [])?;
        transaction.execute("DELETE FROM chunks_fts", [])?;
        transaction.execute("DELETE FROM symbols", [])?;

        let mut count = 0usize;
        for (name, collection) in &config.collections {
            let glob = compile_glob(&collection.pattern)?;
            let mut walker = WalkBuilder::new(&collection.path);
            walker.hidden(false).git_ignore(true).git_exclude(true);

            for entry in walker.build() {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => continue,
                };
                if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                    continue;
                }

                let absolute_path = entry.path().to_path_buf();
                let relative_path = match absolute_path.strip_prefix(&collection.path) {
                    Ok(path) => path.to_path_buf(),
                    Err(_) => continue,
                };
                if !glob.is_match(&relative_path) {
                    continue;
                }

                let Ok(content) = fs::read_to_string(&absolute_path) else {
                    continue;
                };
                let virtual_path = format!("sifter://{name}/{}", relative_path.to_string_lossy());
                let kind = classify_kind(&absolute_path).to_string();
                let plugin = self.plugins.plugin_for_path(&absolute_path);
                let language = plugin.map(|item| item.language_name().to_string());
                let title = infer_title(&absolute_path, &content);
                let line_end = content.lines().count().max(1);
                let docid = docid_for(name, &relative_path, &content);
                let context = matching_contexts(config, &virtual_path)
                    .into_iter()
                    .next()
                    .map(|item| item.value);

                transaction.execute(
                    "INSERT INTO files (
                        docid, collection, path, virtual_path, kind, title, language, context, content, line_start, line_end
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        docid,
                        name,
                        absolute_path.to_string_lossy(),
                        virtual_path,
                        kind,
                        title,
                        language,
                        context,
                        content,
                        1usize,
                        line_end,
                    ],
                )?;

                for chunk in chunk_file(name, &relative_path, &absolute_path, &content)? {
                    transaction.execute(
                        "INSERT INTO chunks (
                            chunk_docid, docid, title, content, line_start, line_end
                        ) VALUES (?, ?, ?, ?, ?, ?)",
                        params![
                            chunk.chunk_docid,
                            docid,
                            chunk.title,
                            chunk.content,
                            chunk.line_start,
                            chunk.line_end,
                        ],
                    )?;
                    transaction.execute(
                        "INSERT INTO chunks_fts (chunk_docid, title, content) VALUES (?, ?, ?)",
                        params![chunk.chunk_docid, chunk.title, chunk.content],
                    )?;
                }

                if let Some(plugin) = plugin {
                    for symbol in plugin.extract_symbols(&content, &absolute_path) {
                        transaction.execute(
                            "INSERT INTO symbols (
                                docid, name, kind, line_start, line_end, scope
                            ) VALUES (?, ?, ?, ?, ?, ?)",
                            params![
                                docid,
                                symbol.name,
                                symbol.kind.as_str(),
                                symbol.line_start,
                                symbol.line_end,
                                symbol.scope,
                            ],
                        )?;
                    }
                }

                count += 1;
            }
        }

        transaction.commit()?;
        Ok(count)
    }

    pub fn status(&self, config: &Config) -> Result<Status> {
        let indexed_files = self
            .connection
            .query_row("SELECT COUNT(*) FROM files", [], |row| {
                row.get::<_, usize>(0)
            })?;
        let indexed_docs = self.connection.query_row(
            "SELECT COUNT(*) FROM files WHERE kind = 'doc'",
            [],
            |row| row.get::<_, usize>(0),
        )?;
        let indexed_code = self.connection.query_row(
            "SELECT COUNT(*) FROM files WHERE kind = 'code'",
            [],
            |row| row.get::<_, usize>(0),
        )?;

        Ok(Status {
            indexed_files,
            indexed_docs,
            indexed_code,
            collections: config.collections.len(),
            has_vector_index: false,
            vector_runtime: "pending".to_string(),
        })
    }

    pub fn search(&self, query: &str, options: &SearchOptions) -> Result<Vec<SearchHit>> {
        let kind_filter = match options.kind {
            Some(SearchKind::Doc) => Some("doc"),
            Some(SearchKind::Code) => Some("code"),
            None => None,
        };

        let mut statement = self.connection.prepare(
            "SELECT files.docid, files.path, files.collection, files.kind, chunks.title, files.context, chunks.line_start, chunks.line_end, files.language, chunks.content,
                    snippet(chunks_fts, 2, '[', ']', ' … ', 12), bm25(chunks_fts)
             FROM chunks_fts
             JOIN chunks ON chunks.chunk_docid = chunks_fts.chunk_docid
             JOIN files ON files.docid = chunks.docid
             WHERE chunks_fts MATCH ? AND (? IS NULL OR files.kind = ?)
             ORDER BY bm25(chunks_fts)
             LIMIT 20",
        )?;

        let rows = statement.query_map(params![query, kind_filter, kind_filter], |row| {
            Ok(SearchHit {
                docid: row.get(0)?,
                file: row.get(1)?,
                collection: row.get(2)?,
                kind: row.get(3)?,
                title: row.get(4)?,
                context: row.get(5)?,
                line_start: row.get(6)?,
                line_end: row.get(7)?,
                language: row.get(8)?,
                full_content: if options.include_full_content {
                    Some(row.get(9)?)
                } else {
                    None
                },
                snippet: row.get(10)?,
                score: -row.get::<_, f64>(11)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get(&self, reference: &str, slice: Option<LineSlice>) -> Result<Option<IndexedFile>> {
        let parsed = ParsedReference::parse(reference, slice);
        let mut statement = if parsed.reference.starts_with('#') {
            self.connection.prepare(
                "SELECT docid, collection, path, virtual_path, kind, title, language, context, content, line_start, line_end
                 FROM files WHERE docid = ? LIMIT 1",
            )?
        } else if parsed.reference.starts_with("sifter://") {
            self.connection.prepare(
                "SELECT docid, collection, path, virtual_path, kind, title, language, context, content, line_start, line_end
                 FROM files WHERE virtual_path = ? LIMIT 1",
            )?
        } else {
            self.connection.prepare(
                "SELECT docid, collection, path, virtual_path, kind, title, language, context, content, line_start, line_end
                 FROM files WHERE path = ? LIMIT 1",
            )?
        };

        let key = parsed.reference.trim_start_matches('#');
        let mut rows = statement.query([key])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };

        let mut file = IndexedFile {
            docid: row.get(0)?,
            collection: row.get(1)?,
            path: row.get(2)?,
            virtual_path: row.get(3)?,
            kind: row.get(4)?,
            title: row.get(5)?,
            language: row.get(6)?,
            context: row.get(7)?,
            content: row.get(8)?,
            line_start: row.get(9)?,
            line_end: row.get(10)?,
        };

        if let Some(slice) = parsed.slice {
            apply_line_slice(&mut file, slice)?;
        }

        Ok(Some(file))
    }

    pub fn multi_get(
        &self,
        references: &[String],
        slice: Option<LineSlice>,
    ) -> Result<Vec<IndexedFile>> {
        let mut results = Vec::new();

        for reference in references {
            let parsed = ParsedReference::parse(reference, slice);
            if parsed.reference.contains('*') || parsed.reference.contains('?') {
                let mut statement = self.connection.prepare(
                    "SELECT docid, collection, path, virtual_path, kind, title, language, context, content, line_start, line_end
                     FROM files WHERE path GLOB ? ORDER BY path",
                )?;
                let rows = statement.query_map([parsed.reference.as_str()], |row| {
                    Ok(IndexedFile {
                        docid: row.get(0)?,
                        collection: row.get(1)?,
                        path: row.get(2)?,
                        virtual_path: row.get(3)?,
                        kind: row.get(4)?,
                        title: row.get(5)?,
                        language: row.get(6)?,
                        context: row.get(7)?,
                        content: row.get(8)?,
                        line_start: row.get(9)?,
                        line_end: row.get(10)?,
                    })
                })?;

                for row in rows {
                    let mut file = row?;
                    if let Some(slice) = parsed.slice {
                        apply_line_slice(&mut file, slice)?;
                    }
                    results.push(file);
                }
            } else if let Some(file) = self.get(reference, slice)? {
                results.push(file);
            }
        }

        Ok(results)
    }

    pub fn symbol(&self, query: &str) -> Result<Vec<SymbolHit>> {
        let mut statement = self.connection.prepare(
            "SELECT symbols.name, symbols.kind, files.path, files.collection, symbols.line_start, symbols.line_end, files.language
             FROM symbols
             JOIN files ON files.docid = symbols.docid
             WHERE symbols.name = ? OR symbols.name LIKE ?
             ORDER BY symbols.name, files.path",
        )?;
        let like = format!("{query}%");
        let rows = statement.query_map(params![query, like], |row| {
            Ok(SymbolHit {
                name: row.get(0)?,
                kind: row.get(1)?,
                file: row.get(2)?,
                collection: row.get(3)?,
                line_start: row.get(4)?,
                line_end: row.get(5)?,
                language: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn related(&self, reference: &str) -> Result<Vec<RelatedHit>> {
        let target = match self.get(reference, None)? {
            Some(target) => target,
            None => return Ok(Vec::new()),
        };
        let target_symbols = self.symbols_for_docid(&target.docid)?;
        if target_symbols.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        let mut statement = self.connection.prepare(
            "SELECT docid, path, collection, content FROM files WHERE docid != ? ORDER BY path",
        )?;
        let rows = statement.query_map([target.docid.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;

        for row in rows {
            let (docid, path, collection, content) = row?;
            let shared_symbols = target_symbols
                .iter()
                .filter(|name| content.contains(name.as_str()) || self.doc_has_symbol(&docid, name))
                .cloned()
                .collect::<Vec<_>>();
            if shared_symbols.is_empty() {
                continue;
            }

            results.push(RelatedHit {
                file: path,
                collection,
                score: shared_symbols.len(),
                shared_symbols,
            });
        }

        results.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then(left.file.cmp(&right.file))
        });
        Ok(results)
    }

    fn migrate(&self) -> Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                docid TEXT PRIMARY KEY,
                collection TEXT NOT NULL,
                path TEXT NOT NULL,
                virtual_path TEXT NOT NULL UNIQUE,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                language TEXT,
                context TEXT,
                content TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS chunks (
                chunk_docid TEXT PRIMARY KEY,
                docid TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docid TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL,
                scope TEXT
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
                chunk_docid UNINDEXED,
                title,
                content
            );",
        )?;
        Ok(())
    }

    fn symbols_for_docid(&self, docid: &str) -> Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT name FROM symbols WHERE docid = ? ORDER BY name")?;
        let rows = statement.query_map([docid], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn doc_has_symbol(&self, docid: &str, name: &str) -> bool {
        self.connection
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE docid = ? AND name = ?",
                params![docid, name],
                |row| row.get::<_, usize>(0),
            )
            .map(|count| count > 0)
            .unwrap_or(false)
    }

    pub fn docid_for_path(&self, path: &str) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT docid FROM files WHERE path = ? LIMIT 1",
                [path],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }
}

#[derive(Debug, Clone)]
struct ParsedReference {
    reference: String,
    slice: Option<LineSlice>,
}

impl ParsedReference {
    fn parse(input: &str, fallback: Option<LineSlice>) -> Self {
        let (reference, line_start) = parse_line_suffix(input);
        let slice = line_start
            .map(|start| LineSlice {
                start,
                max_lines: fallback.and_then(|item| item.max_lines),
                line_numbers: fallback.is_some_and(|item| item.line_numbers),
            })
            .or(fallback);

        Self { reference, slice }
    }
}

fn compile_glob(pattern: &str) -> Result<globset::GlobSet> {
    let mut builder = GlobSetBuilder::new();
    builder.add(Glob::new(pattern)?);
    builder.build().context("failed to build collection glob")
}

fn classify_kind(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("md" | "mdx" | "txt" | "rst") => "doc",
        _ => "code",
    }
}

fn infer_title(path: &Path, content: &str) -> String {
    if matches!(classify_kind(path), "doc")
        && let Some(heading) = content
            .lines()
            .find(|line| line.starts_with('#'))
            .map(|line| line.trim_start_matches('#').trim())
            .filter(|line| !line.is_empty())
    {
        return heading.to_string();
    }

    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("untitled")
        .to_string()
}

fn chunk_file(
    collection: &str,
    relative_path: &Path,
    absolute_path: &Path,
    content: &str,
) -> Result<Vec<ChunkRecord>> {
    let path_title = infer_title(absolute_path, content);
    if classify_kind(absolute_path) == "doc" {
        return Ok(chunk_markdown(
            collection,
            relative_path,
            content,
            &path_title,
        ));
    }

    Ok(vec![ChunkRecord {
        chunk_docid: chunk_docid_for(collection, relative_path, 0, content),
        title: path_title,
        content: content.to_string(),
        line_start: 1,
        line_end: content.lines().count().max(1),
    }])
}

fn chunk_markdown(
    collection: &str,
    relative_path: &Path,
    content: &str,
    fallback_title: &str,
) -> Vec<ChunkRecord> {
    let mut chunks = Vec::new();
    let mut current_title = fallback_title.to_string();
    let mut current_lines = Vec::new();
    let mut current_start = 1usize;
    let mut line_no = 0usize;
    let mut in_code_fence = false;

    for line in content.lines() {
        line_no += 1;
        if line.trim_start().starts_with("```") {
            in_code_fence = !in_code_fence;
        }

        if !in_code_fence && line.starts_with('#') {
            if !current_lines.is_empty() {
                chunks.push(build_chunk(
                    collection,
                    relative_path,
                    chunks.len(),
                    &current_title,
                    &current_lines,
                    current_start,
                ));
                current_lines.clear();
            }
            current_title = line.trim_start_matches('#').trim().to_string();
            current_start = line_no;
        }

        current_lines.push(line.to_string());
    }

    if !current_lines.is_empty() {
        chunks.push(build_chunk(
            collection,
            relative_path,
            chunks.len(),
            &current_title,
            &current_lines,
            current_start,
        ));
    }

    chunks
}

fn build_chunk(
    collection: &str,
    relative_path: &Path,
    index: usize,
    title: &str,
    lines: &[String],
    start_line: usize,
) -> ChunkRecord {
    let content = lines.join("\n");
    let end_line = start_line + lines.len().saturating_sub(1);
    ChunkRecord {
        chunk_docid: chunk_docid_for(collection, relative_path, index, &content),
        title: title.to_string(),
        content,
        line_start: start_line,
        line_end: end_line.max(start_line),
    }
}

fn apply_line_slice(file: &mut IndexedFile, slice: LineSlice) -> Result<()> {
    if slice.start == 0 {
        return Err(anyhow!("line numbers are 1-based"));
    }

    let all_lines = file.content.lines().map(str::to_string).collect::<Vec<_>>();
    if all_lines.is_empty() {
        return Ok(());
    }
    if slice.start > all_lines.len() {
        return Err(anyhow!("line {} is out of range", slice.start));
    }

    let zero_based_start = slice.start - 1;
    let end = match slice.max_lines {
        Some(max_lines) => zero_based_start
            .saturating_add(max_lines)
            .min(all_lines.len()),
        None => all_lines.len(),
    };
    let selected = &all_lines[zero_based_start..end];
    file.content = if slice.line_numbers {
        selected
            .iter()
            .enumerate()
            .map(|(offset, line)| format!("{:>4}: {}", slice.start + offset, line))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        selected.join("\n")
    };
    file.line_start = slice.start;
    file.line_end = slice.start + selected.len().saturating_sub(1);
    Ok(())
}

fn parse_line_suffix(input: &str) -> (String, Option<usize>) {
    let Some((reference, suffix)) = input.rsplit_once(':') else {
        return (input.to_string(), None);
    };

    if suffix.chars().all(|character| character.is_ascii_digit()) {
        return (reference.to_string(), suffix.parse::<usize>().ok());
    }

    (input.to_string(), None)
}

fn docid_for(collection: &str, relative_path: &Path, content: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(collection.as_bytes());
    hasher.update(relative_path.to_string_lossy().as_bytes());
    hasher.update(content.as_bytes());
    let digest = hasher.finalize().to_hex().to_string();
    digest[..12].to_string()
}

fn chunk_docid_for(collection: &str, relative_path: &Path, index: usize, content: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(collection.as_bytes());
    hasher.update(relative_path.to_string_lossy().as_bytes());
    hasher.update(index.to_string().as_bytes());
    hasher.update(content.as_bytes());
    let digest = hasher.finalize().to_hex().to_string();
    digest[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_chunking_splits_on_headings_outside_code_fences() {
        let content = "# One\nalpha\n```rust\n# not a heading\n```\n## Two\nbeta\n";
        let chunks = chunk_markdown("repo", Path::new("docs/test.md"), content, "test");
        let summary = chunks
            .iter()
            .map(|chunk| (chunk.title.clone(), chunk.line_start, chunk.line_end))
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            vec![("One".to_string(), 1, 5), ("Two".to_string(), 6, 7)]
        );
    }

    #[test]
    fn parse_line_suffix_extracts_reference_and_line() {
        assert_eq!(
            parse_line_suffix("sifter://repo/docs/brief.md:12"),
            ("sifter://repo/docs/brief.md".to_string(), Some(12))
        );
    }
}
