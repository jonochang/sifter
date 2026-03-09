use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use blake3::Hasher;
use globset::{Glob, GlobSetBuilder};
use ignore::WalkBuilder;
use rusqlite::{Connection, params};
use serde::Serialize;
use sifter_codeintel::PluginRegistry;
use sifter_codeintel_rust::RustPlugin;
use sifter_core::config::Config;

#[derive(Debug, Clone, Serialize)]
pub struct IndexedFile {
    pub docid: String,
    pub collection: String,
    pub path: String,
    pub virtual_path: String,
    pub kind: String,
    pub title: String,
    pub language: Option<String>,
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
    pub score: f64,
    pub snippet: String,
    pub line_start: usize,
    pub line_end: usize,
    pub language: Option<String>,
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
        transaction.execute("DELETE FROM files_fts", [])?;
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

                transaction.execute(
                    "INSERT INTO files (
                        docid, collection, path, virtual_path, kind, title, language, content, line_start, line_end
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        docid,
                        name,
                        absolute_path.to_string_lossy(),
                        virtual_path,
                        kind,
                        title,
                        language,
                        content,
                        1usize,
                        line_end,
                    ],
                )?;

                transaction.execute(
                    "INSERT INTO files_fts (docid, title, content) VALUES (?, ?, ?)",
                    params![docid, title, content],
                )?;

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

    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let mut statement = self.connection.prepare(
            "SELECT files.docid, files.path, files.collection, files.kind, files.title, files.line_start, files.line_end, files.language, snippet(files_fts, 2, '[', ']', ' … ', 12), bm25(files_fts)
             FROM files_fts
             JOIN files ON files.docid = files_fts.docid
             WHERE files_fts MATCH ?
             ORDER BY bm25(files_fts)
             LIMIT 20",
        )?;

        let hits = statement
            .query_map([query], |row| {
                Ok(SearchHit {
                    docid: row.get(0)?,
                    file: row.get(1)?,
                    collection: row.get(2)?,
                    kind: row.get(3)?,
                    title: row.get(4)?,
                    line_start: row.get(5)?,
                    line_end: row.get(6)?,
                    language: row.get(7)?,
                    snippet: row.get(8)?,
                    score: -row.get::<_, f64>(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(hits)
    }

    pub fn get(&self, reference: &str) -> Result<Option<IndexedFile>> {
        let mut statement = if reference.starts_with('#') {
            self.connection.prepare(
                "SELECT docid, collection, path, virtual_path, kind, title, language, content, line_start, line_end
                 FROM files WHERE docid = ? LIMIT 1",
            )?
        } else if reference.starts_with("sifter://") {
            self.connection.prepare(
                "SELECT docid, collection, path, virtual_path, kind, title, language, content, line_start, line_end
                 FROM files WHERE virtual_path = ? LIMIT 1",
            )?
        } else {
            self.connection.prepare(
                "SELECT docid, collection, path, virtual_path, kind, title, language, content, line_start, line_end
                 FROM files WHERE path = ? LIMIT 1",
            )?
        };

        let key = if reference.starts_with('#') {
            reference.trim_start_matches('#')
        } else {
            reference
        };
        let mut rows = statement.query([key])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(IndexedFile {
                docid: row.get(0)?,
                collection: row.get(1)?,
                path: row.get(2)?,
                virtual_path: row.get(3)?,
                kind: row.get(4)?,
                title: row.get(5)?,
                language: row.get(6)?,
                content: row.get(7)?,
                line_start: row.get(8)?,
                line_end: row.get(9)?,
            }));
        }
        Ok(None)
    }

    pub fn multi_get(&self, references: &str) -> Result<Vec<IndexedFile>> {
        let parts = references
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let mut results = Vec::new();

        for part in parts {
            if part.contains('*') || part.contains('?') {
                let mut statement = self.connection.prepare(
                    "SELECT docid, collection, path, virtual_path, kind, title, language, content, line_start, line_end
                     FROM files WHERE path GLOB ? ORDER BY path",
                )?;
                let rows = statement.query_map([part], |row| {
                    Ok(IndexedFile {
                        docid: row.get(0)?,
                        collection: row.get(1)?,
                        path: row.get(2)?,
                        virtual_path: row.get(3)?,
                        kind: row.get(4)?,
                        title: row.get(5)?,
                        language: row.get(6)?,
                        content: row.get(7)?,
                        line_start: row.get(8)?,
                        line_end: row.get(9)?,
                    })
                })?;
                results.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
            } else if let Some(file) = self.get(part)? {
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
        let target = match self.get(reference)? {
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
            CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
                docid UNINDEXED,
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

fn docid_for(collection: &str, relative_path: &Path, content: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(collection.as_bytes());
    hasher.update(relative_path.to_string_lossy().as_bytes());
    hasher.update(content.as_bytes());
    let digest = hasher.finalize().to_hex().to_string();
    digest[..12].to_string()
}
