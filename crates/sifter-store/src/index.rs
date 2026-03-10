use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use blake3::Hasher;
use globset::{Glob, GlobSetBuilder};
use ignore::WalkBuilder;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sifter_codeintel::PluginRegistry;
use sifter_codeintel_rust::RustPlugin;
use sifter_core::config::{Config, matching_contexts};
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query as TantivyQuery, QueryParser, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TEXT, TantivyDocument, Value,
};
use tantivy::{Index, ReloadPolicy, Term, doc};

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
    pub match_type: String,
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
pub enum SymbolMode {
    Definitions,
    References,
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

struct LexicalIndex {
    index_dir: PathBuf,
    index: Index,
    chunk_docid: Field,
    title: Field,
    content: Field,
    kind: Field,
}

pub struct Store {
    connection: Connection,
    plugins: PluginRegistry,
    lexical: LexicalIndex,
}

#[derive(Debug)]
struct RelatedAccumulator {
    file: String,
    collection: String,
    score: usize,
    shared_symbols: BTreeSet<String>,
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
            lexical: LexicalIndex::open(&lexical_index_dir(path)?)?,
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn rebuild(&mut self, config: &Config) -> Result<usize> {
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM files", [])?;
        transaction.execute("DELETE FROM chunks", [])?;
        transaction.execute("DELETE FROM symbols", [])?;
        transaction.execute("DELETE FROM relations", [])?;

        self.lexical = LexicalIndex::reset(&self.lexical.index_dir)?;
        let mut writer = self
            .lexical
            .index
            .writer(50_000_000)
            .context("failed to create Tantivy writer")?;

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

                    writer.add_document(doc!(
                        self.lexical.chunk_docid => chunk.chunk_docid,
                        self.lexical.title => chunk.title,
                        self.lexical.content => chunk.content,
                        self.lexical.kind => kind.clone(),
                    ))?;
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

                    for relation in plugin.extract_relations(&content, &absolute_path) {
                        transaction.execute(
                            "INSERT INTO relations (
                                docid, name, kind, line_start, line_end
                            ) VALUES (?, ?, ?, ?, ?)",
                            params![
                                docid,
                                relation.name,
                                relation.kind.as_str(),
                                relation.line_start,
                                relation.line_end,
                            ],
                        )?;
                    }
                }

                count += 1;
            }
        }

        transaction.commit()?;
        writer.commit().context("failed to commit Tantivy index")?;
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
        let reader = self
            .lexical
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("failed to open Tantivy reader")?;
        let searcher = reader.searcher();

        let parser = QueryParser::for_index(
            &self.lexical.index,
            vec![self.lexical.title, self.lexical.content],
        );
        let parsed = parser
            .parse_query(query)
            .with_context(|| format!("failed to parse query '{query}'"))?;

        let boxed_query: Box<dyn TantivyQuery> = if let Some(kind) = options.kind {
            let kind_term = match kind {
                SearchKind::Doc => "doc",
                SearchKind::Code => "code",
            };
            Box::new(BooleanQuery::new(vec![
                (Occur::Must, parsed),
                (
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(self.lexical.kind, kind_term),
                        IndexRecordOption::Basic,
                    )),
                ),
            ]))
        } else {
            parsed
        };

        let top_docs = searcher.search(&boxed_query, &TopDocs::with_limit(20))?;
        let mut hits = Vec::new();
        let mut statement = self.connection.prepare(
            "SELECT files.docid, files.path, files.collection, files.kind, chunks.title, files.context, chunks.line_start, chunks.line_end, files.language, chunks.content
             FROM chunks
             JOIN files ON files.docid = chunks.docid
             WHERE chunks.chunk_docid = ? LIMIT 1",
        )?;

        for (score, address) in top_docs {
            let document: TantivyDocument = searcher.doc(address)?;
            let Some(chunk_docid) = document
                .get_first(self.lexical.chunk_docid)
                .and_then(|value| value.as_str())
            else {
                continue;
            };

            let chunk_hit = statement.query_row([chunk_docid], |row| {
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
                    snippet: generate_snippet(&row.get::<_, String>(9)?, query),
                    score: score.into(),
                })
            })?;
            hits.push(chunk_hit);
        }

        Ok(hits)
    }

    pub fn get(&self, reference: &str, slice: Option<LineSlice>) -> Result<Option<IndexedFile>> {
        let parsed = ParsedReference::parse(reference, slice);
        let key = normalize_lookup_reference(&parsed.reference)?;
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

        let mut rows = statement.query([key.as_str()])?;
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
                self.extend_glob_matches(&mut results, &parsed)?;
            } else if let Some(file) = self.get(reference, slice)? {
                results.push(file);
            }
        }

        Ok(results)
    }

    pub fn symbol(&self, query: &str, mode: SymbolMode) -> Result<Vec<SymbolHit>> {
        let like = format!("{query}%");
        let (sql, match_type) = match mode {
            SymbolMode::Definitions => (
                "SELECT symbols.name, symbols.kind, files.path, files.collection, symbols.line_start, symbols.line_end, files.language
                 FROM symbols
                 JOIN files ON files.docid = symbols.docid
                 WHERE symbols.name = ? OR symbols.name LIKE ?
                 ORDER BY symbols.name, files.path, symbols.line_start",
                "definition",
            ),
            SymbolMode::References => (
                "SELECT DISTINCT relations.name, relations.kind, files.path, files.collection, relations.line_start, relations.line_end, files.language
                 FROM relations
                 JOIN files ON files.docid = relations.docid
                 WHERE relations.name = ? OR relations.name LIKE ?
                 ORDER BY relations.name, files.path, relations.line_start",
                "reference",
            ),
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(params![query, like], |row| {
            Ok(SymbolHit {
                name: row.get(0)?,
                kind: row.get(1)?,
                match_type: match_type.to_string(),
                file: row.get(2)?,
                collection: row.get(3)?,
                line_start: row.get(4)?,
                line_end: row.get(5)?,
                language: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn extend_glob_matches(
        &self,
        results: &mut Vec<IndexedFile>,
        parsed: &ParsedReference,
    ) -> Result<()> {
        let (query, order_by) = if parsed.reference.starts_with("sifter://") {
            ("virtual_path", "virtual_path")
        } else {
            ("path", "path")
        };
        let mut statement = self.connection.prepare(&format!(
            "SELECT docid, collection, path, virtual_path, kind, title, language, context, content, line_start, line_end
             FROM files WHERE {query} GLOB ? ORDER BY {order_by}"
        ))?;
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
        Ok(())
    }

    pub fn related(&self, reference: &str) -> Result<Vec<RelatedHit>> {
        let target = match self.get(reference, None)? {
            Some(target) => target,
            None => return Ok(Vec::new()),
        };
        let target_symbols = self
            .symbols_for_docid(&target.docid)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        if target_symbols.is_empty() {
            return Ok(Vec::new());
        }

        let mut statement = self.connection.prepare(
            "SELECT relations.docid, files.path, files.collection, relations.name, relations.kind
             FROM relations
             JOIN files ON files.docid = relations.docid
             WHERE files.docid != ?
             ORDER BY files.path",
        )?;
        let rows = statement.query_map([target.docid.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut related = HashMap::<String, RelatedAccumulator>::new();
        for row in rows {
            let (docid, path, collection, name, kind) = row?;
            if !target_symbols.contains(&name) {
                continue;
            }

            let entry = related.entry(docid).or_insert_with(|| RelatedAccumulator {
                file: path,
                collection,
                score: 0,
                shared_symbols: BTreeSet::new(),
            });
            entry.score += relation_weight(&kind);
            entry.shared_symbols.insert(name);
        }

        let mut results = related
            .into_values()
            .map(|item| RelatedHit {
                file: item.file,
                collection: item.collection,
                score: item.score,
                shared_symbols: item.shared_symbols.into_iter().collect(),
            })
            .collect::<Vec<_>>();
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
            CREATE TABLE IF NOT EXISTS relations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docid TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER NOT NULL
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

impl LexicalIndex {
    fn open(index_dir: &Path) -> Result<Self> {
        fs::create_dir_all(index_dir)
            .with_context(|| format!("failed to create {}", index_dir.display()))?;
        let schema = lexical_schema();
        let index = if index_dir.join("meta.json").exists() {
            Index::open_in_dir(index_dir).context("failed to open Tantivy index")?
        } else {
            Index::create_in_dir(index_dir, schema.clone())
                .context("failed to create Tantivy index")?
        };
        Ok(Self::from_index(index_dir.to_path_buf(), index, schema))
    }

    fn reset(index_dir: &Path) -> Result<Self> {
        if index_dir.exists() {
            fs::remove_dir_all(index_dir)
                .with_context(|| format!("failed to clear {}", index_dir.display()))?;
        }
        fs::create_dir_all(index_dir)
            .with_context(|| format!("failed to create {}", index_dir.display()))?;
        let schema = lexical_schema();
        let index = Index::create_in_dir(index_dir, schema.clone())
            .context("failed to create Tantivy index")?;
        Ok(Self::from_index(index_dir.to_path_buf(), index, schema))
    }

    fn from_index(index_dir: PathBuf, index: Index, schema: Schema) -> Self {
        let chunk_docid = schema.get_field("chunk_docid").expect("chunk_docid field");
        let title = schema.get_field("title").expect("title field");
        let content = schema.get_field("content").expect("content field");
        let kind = schema.get_field("kind").expect("kind field");
        Self {
            index_dir,
            index,
            chunk_docid,
            title,
            content,
            kind,
        }
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

fn lexical_index_dir(db_path: &Path) -> Result<PathBuf> {
    let stem = db_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("invalid database path for lexical index"))?;
    let parent = db_path
        .parent()
        .ok_or_else(|| anyhow!("database path has no parent"))?;
    Ok(parent.join(format!("{stem}.tantivy")))
}

fn lexical_schema() -> Schema {
    let mut builder = Schema::builder();
    builder.add_text_field("chunk_docid", STRING | STORED);
    builder.add_text_field("title", TEXT | STORED);
    builder.add_text_field("content", TEXT);
    builder.add_text_field("kind", STRING | STORED);
    builder.build()
}

fn relation_weight(kind: &str) -> usize {
    match kind {
        "import" => 3,
        "mention" => 1,
        _ => 1,
    }
}

fn normalize_lookup_reference(reference: &str) -> Result<String> {
    if reference.starts_with('#') {
        return Ok(reference.trim_start_matches('#').to_string());
    }

    if reference.starts_with("sifter://") {
        return Ok(reference.to_string());
    }

    let path = Path::new(reference);
    if path.exists() {
        return fs::canonicalize(path)
            .with_context(|| format!("failed to canonicalize {}", path.display()))
            .map(|path| path.to_string_lossy().to_string());
    }

    Ok(reference.to_string())
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

fn generate_snippet(content: &str, query: &str) -> String {
    let lowered_content = content.to_lowercase();
    let lowered_query = query.to_lowercase();
    if let Some(index) = lowered_content.find(&lowered_query) {
        let start = index.saturating_sub(40);
        let end = (index + query.len() + 80).min(content.len());
        let snippet = content[start..end].trim();
        return snippet.replacen(query, &format!("[{query}]"), 1);
    }
    content.lines().take(4).collect::<Vec<_>>().join("\n")
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
