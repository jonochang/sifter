# Sifter — Product Brief

**A Rust port of QMD, extended with code intelligence.**

QMD is an excellent local search engine for documents and knowledge bases. Sifter takes the same core design — BM25, vector search, hybrid retrieval with reranking, smart chunking, collections, context annotations — and rebuilds it in Rust as a single static binary, then adds Tree-sitter-backed code indexing so the same tool covers both documentation and source code.

---

## Goals

### 1. Port QMD's document search to Rust

QMD's architecture is well-proven: SQLite FTS5 for BM25, sqlite-vec for vector similarity, GGUF models via node-llama-cpp for embeddings and reranking, smart markdown chunking with heading-aware boundaries, and RRF fusion for hybrid results. The problem is the runtime: QMD requires Node.js >= 22 or Bun, plus native SQLite extensions, plus ~2GB of downloaded models. That's a lot of moving parts for a tool that wants to be "just there" in a developer's workflow.

Sifter rebuilds this in Rust with the goal of a single binary that embeds everything it needs. The retrieval pipeline stays the same — BM25 + vector + optional reranking with RRF fusion — but the implementation shifts to Tantivy (or SQLite FTS5 via rusqlite) for lexical search, and a Rust GGUF runtime (llama.cpp bindings or candle) for local embeddings and reranking.

### 2. Add code intelligence as a first-class index type

QMD indexes markdown, meeting notes, and knowledge bases. It has no concept of source code as structured content. Sifter adds a code indexing pipeline that uses Tree-sitter to extract symbols (functions, structs, types, imports, constants), their positions, enclosing scopes, and cross-references. This means a search for "retry budget" can return not just the ADR section that discusses it, but also the `RetryPolicy` struct, the test that exercises budget exhaustion, and the comment in `client.rs` that references the concept.

### 3. Preserve QMD's agent ergonomics

QMD already does several things right for agent workflows: JSON output, structured result fields (docid, score, path, context, snippet), collection-scoped search, min-score filtering, and the `--all --files` pattern for dumping relevant file lists. Sifter keeps all of this and adds code-specific structured output (symbol kind, scope, references, line ranges).

### 4. Ship as a single binary

No Node.js, no Bun, no Python, no Docker. One binary, one `sift` command. Models are either bundled or downloaded on first use to a cache directory, but the tool itself is a single static Rust executable.

---

## Scenarios

### Scenario 1 — Drop-in QMD replacement for docs

A developer currently uses QMD to search their knowledge base: meeting notes, ADRs, runbooks, design docs. They switch to Sifter and the experience is equivalent:

```
sift collection add ~/notes --name notes
sift context add sift://notes "Personal notes and project decisions"
sift embed
sift search "quarterly planning"        # BM25 keyword search
sift vsearch "how do we handle deploys" # vector semantic search
sift query "authentication flow"        # hybrid with reranking
```

Same three search modes as QMD. Same collection and context model. Same output formats.

### Scenario 2 — Agent context acquisition across code and docs

A Claude Code-style agent is tasked with modifying retry behavior. It needs the design rationale, the current implementation, and the test coverage.

```
sift query "retry budget"
```

Sifter returns results from both indexes:
- An ADR section in `docs/adr/007-retry-strategy.md` under "Budget exhaustion" (doc result)
- The `RetryPolicy` struct definition in `src/net/retry.rs` (code result, symbol: struct)
- A test `test_retry_budget_exceeded` in `src/net/retry_test.rs` (code result, symbol: function)
- A comment in `src/client.rs` referencing budget limits (code result, lexical match)

The agent gets a focused working set spanning both documentation and code from a single query.

### Scenario 3 — Symbol exploration

An engineer wants to understand how `RequestExecutor` is used across the codebase:

```
sift symbol RequestExecutor
```

Sifter returns the struct definition, its methods, files that import it, and any doc sections that mention it by name. This is structural retrieval — not grep, not semantic search, but Tree-sitter-extracted symbol relationships.

### Scenario 4 — Related context expansion

An agent has identified `src/net/retry.rs` as relevant and wants to expand outward:

```
sift related src/net/retry.rs
```

Sifter walks the cross-reference graph to find files sharing symbols, import chains, or doc mentions with the target file. Progressive context expansion without path guessing.

### Scenario 5 — Scoped collection search

A developer has separate collections for their monorepo's docs, their personal notes, and a vendor SDK:

```
sift search "rate limiting" -c docs         # only project docs
sift vsearch "how to authenticate" -c sdk   # only vendor SDK
sift query "deployment checklist" --all --files --min-score 0.4
```

Same collection-scoping model as QMD, with the same `--all --files --min-score` pattern for agent-friendly bulk retrieval.

---

## Constraints

1. **Single Rust binary.** No runtime dependencies. The install story is: download one file, run it.
2. **Local-only.** No network calls at search time. Models are cached locally after first download. Index lives on the filesystem.
3. **QMD-compatible mental model.** Anyone who knows QMD should immediately understand Sifter. Same collection/context concepts, same three search tiers (search/vsearch/query), same output format options.
4. **Fast enough to be interactive.** Target sub-100ms for BM25 search, sub-500ms for vector search, sub-2s for full hybrid query with reranking — on repos up to 500K lines of code and 10K doc files.
5. **Structured output by default.** JSON for non-TTY (agent consumption), human-readable for TTY. Same as QMD's approach.
6. **Tree-sitter for code structure.** Grammars compiled into the binary for a fixed set of languages. No LSP, no type resolution — just fast structural extraction.

---

## Out of Scope

These are explicitly not part of Sifter:

- **IDE integration.** No LSP server, no VS Code extension. CLI-first, agent-first.
- **Remote or cloud indexing.** Everything is local filesystem.
- **Write operations.** Sifter reads, indexes, and retrieves. It doesn't modify files.
- **Deep code analysis.** No type resolution, call graph analysis, or semantic code understanding beyond what Tree-sitter structure provides.
- **Custom model training.** QMD has a fine-tuning pipeline for query expansion models. Sifter will use pre-trained GGUF models. A fine-tuning pipeline is a separate concern.
- **MCP server (v1).** QMD has a built-in MCP server with stdio and HTTP transport. Sifter may add this later, but v1 is CLI-only. The tool should work well when called directly by agents without requiring a long-running server.

---

## API

Sifter's CLI is the API. It mirrors QMD's command structure where applicable and adds code-specific commands.

### Collection and context management (ported from QMD)

```bash
sift collection add ~/project --name myproject
sift collection add ~/notes --name notes --mask "**/*.md"
sift collection list
sift collection remove myproject
sift collection rename myproject my-project
sift ls notes
sift ls notes/subfolder

sift context add sift://notes "Personal notes and ideas"
sift context add sift://docs/api "API documentation"
sift context list
sift context check
sift context rm sift://notes/old
```

Collections are stored in YAML config (following QMD's move away from SQLite for config). Context annotations are returned alongside search results to give agents richer signals about what they're looking at.

### Indexing

```bash
sift update                # re-index all collections
sift update --pull         # git pull before re-indexing
sift embed                 # generate/update vector embeddings
sift embed -f              # force re-embed everything
sift status                # index health, collection info, model status
```

### Search — three tiers (mirroring QMD)

```bash
# BM25 keyword search (fast, exact)
sift search "retry budget"
sift search "retry budget" -c myproject -n 10

# Vector semantic search
sift vsearch "how do we handle retries"
sift vsearch "how do we handle retries" --min-score 0.3

# Hybrid: BM25 + vector + query expansion + RRF + reranking (best quality)
sift query "retry budget"
sift query "retry budget" --all --files --min-score 0.4
```

### Search — code-specific

```bash
# Symbol lookup (Tree-sitter extracted)
sift symbol RetryPolicy
sift symbol RetryPolicy --defs     # definitions only
sift symbol RetryPolicy --refs     # references only

# Cross-reference expansion
sift related src/net/retry.rs
sift related src/net/retry.rs --limit 5
```

### Document retrieval (ported from QMD)

```bash
sift get notes/meeting.md           # get by path
sift get "#abc123"                   # get by docid
sift get notes/meeting.md:50 -l 100 # from line 50, max 100 lines
sift multi-get "docs/*.md"           # glob pattern
sift multi-get "doc1.md, #abc123"    # comma-separated, supports docids
sift multi-get "docs/*.md" --json --max-bytes 20480
```

### Search scope flags

```bash
--code        # restrict to code files only
--docs        # restrict to doc files only
```

These complement the `-c`/`--collection` flag. An agent can search only code within a specific collection, or only docs across all collections.

### Output formats (matching QMD)

```bash
--json        # JSON (default for non-TTY)
--csv         # CSV
--md          # Markdown
--xml         # XML
--files       # docid,score,filepath,context (for agent file lists)
--full        # full document content in results
--line-numbers # add line numbers
```

### Search result shape

Every search result includes QMD-equivalent fields plus code-specific metadata:

```json
{
  "docid": "#a1b2c3",
  "file": "src/net/retry.rs",
  "collection": "myproject",
  "kind": "code",
  "title": "RetryPolicy",
  "context": "Core networking library",
  "score": 0.87,
  "line_start": 42,
  "line_end": 58,
  "snippet": "pub struct RetryPolicy { ... }",
  "symbols": ["RetryPolicy", "budget_remaining"],
  "symbol_kind": "struct",
  "scope": "mod retry"
}
```

For doc results, `symbols`, `symbol_kind`, and `scope` are absent. For code results, `title` is derived from the primary symbol or file name. The `context` field carries the user-supplied context annotation from `sift context add`, exactly as QMD does.

---

## Architecture

### Design borrowed from QMD

Sifter's architecture directly ports several QMD design decisions:

- **SQLite as the storage backbone.** QMD uses SQLite for FTS5 (BM25), schema, and document storage, with sqlite-vec for vector similarity. Sifter follows the same pattern via rusqlite, or alternatively uses Tantivy for lexical search if it proves faster for combined code+doc indexes. The decision is deferred until benchmarking.
- **Smart chunking.** QMD's heading-aware markdown chunking algorithm (900-token target, 15% overlap, break-point scoring that prefers headings > code fence boundaries > horizontal rules > blank lines > list items) is ported directly. This is one of QMD's best design choices and should be preserved.
- **Three-tier search.** BM25-only (fast), vector-only (semantic), hybrid with query expansion and reranking (best quality). Same trade-off spectrum.
- **RRF fusion.** Reciprocal Rank Fusion with position-aware blending. QMD's specific tuning (original query at 2x weight, top-rank bonus, position-aware reranker blending) is a good starting point.
- **Collection and context model.** YAML-based collection config, virtual paths (`sift://collection/path`), user-supplied context annotations returned alongside results.
- **GGUF models for local inference.** Embedding (embeddinggemma-300M or similar), reranking (qwen3-reranker), query expansion (fine-tuned or general small model). Loaded in-process, no external server.

### What Sifter adds

On top of QMD's document search pipeline, Sifter adds a parallel code indexing pipeline:

```
┌──────────────────────────────────────────────────────────────────┐
│                         Sifter Architecture                      │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐    ┌──────────────┐    ┌────────────────────┐  │
│  │  Collection   │    │  File Walker  │    │   File Classifier  │  │
│  │  + Context    │───▶│  (per glob)   │───▶│  (extension-based) │  │
│  │  (YAML cfg)   │    │              │    │                    │  │
│  └──────────────┘    └──────────────┘    └────────┬───────────┘  │
│                                                   │              │
│                                      ┌────────────┴──────────┐   │
│                                      ▼                       ▼   │
│                             ┌──────────────┐       ┌───────────┐ │
│                             │  Doc Pipeline │       │Code Pipe. │ │
│                             │              │       │           │ │
│                             │ Smart chunk  │       │ Tree-sit. │ │
│                             │ (heading-    │       │ parse →   │ │
│                             │  aware, 900  │       │ symbols,  │ │
│                             │  tok, 15%    │       │ scopes,   │ │
│                             │  overlap)    │       │ imports   │ │
│                             │              │       │           │ │
│                             │ Title from   │       │ Code chunk│ │
│                             │ first heading│       │ (function/│ │
│                             │              │       │  class    │ │
│                             │              │       │  aware)   │ │
│                             └──────┬───────┘       └─────┬─────┘ │
│                                    │                     │       │
│                                    ▼                     ▼       │
│                          ┌─────────────────────────────────────┐ │
│                          │          Shared Index Layer          │ │
│                          │                                     │ │
│                          │  ┌─────────┐  ┌──────────────────┐  │ │
│                          │  │  FTS5   │  │  sqlite-vec      │  │ │
│                          │  │  (BM25) │  │  (embeddings)    │  │ │
│                          │  └─────────┘  └──────────────────┘  │ │
│                          │  ┌─────────────────┐  ┌──────────┐  │ │
│                          │  │  Symbol Table    │  │  Xref    │  │ │
│                          │  │  (name, kind,    │  │  Graph   │  │ │
│                          │  │   file, line,    │  │  (edges) │  │ │
│                          │  │   scope)         │  │          │  │ │
│                          │  └─────────────────┘  └──────────┘  │ │
│                          └─────────────────────────────────────┘ │
│                                          │                       │
│                                          ▼                       │
│                          ┌─────────────────────────────────────┐ │
│                          │          Query Engine                │ │
│                          │                                     │ │
│                          │  search  → FTS5 BM25                │ │
│                          │  vsearch → sqlite-vec cosine sim    │ │
│                          │  query   → expansion + BM25 +       │ │
│                          │            vector + RRF + rerank    │ │
│                          │  symbol  → symbol table lookup      │ │
│                          │  related → xref graph traversal     │ │
│                          └─────────────────────────────────────┘ │
│                                                                  │
├──────────────────────────────────────────────────────────────────┤
│                          GGUF Models                             │
│  Embedding: embeddinggemma-300M (or similar, ~300MB)             │
│  Reranker:  qwen3-reranker-0.6b (~640MB)                        │
│  Expansion: small query expansion model (~1GB)                   │
│  Loaded in-process via llama.cpp Rust bindings                   │
└──────────────────────────────────────────────────────────────────┘
```

### Code indexing pipeline

**File classification** is extension-based with a fixed allowlist. Code: `.rs`, `.py`, `.go`, `.ts`, `.js`, `.rb`, `.java`, `.c`, `.cpp`, `.h`. Docs: `.md`, `.rst`, `.txt`, `.adoc`. Unrecognized extensions are skipped.

**Tree-sitter parsing** extracts symbols (functions, structs/classes, types, imports, constants) with their file, line, kind, and enclosing scope. Grammars are compiled into the binary for supported languages.

**Code chunking** is function/class-aware rather than heading-aware. Instead of QMD's markdown break-point scoring, code chunks are split along structural boundaries: function definitions, class/impl blocks, module boundaries. This keeps semantic units together in the vector index.

**Cross-reference extraction** builds edges: "file defines symbol," "file imports symbol," "doc mentions symbol." The last type connects the doc and code indexes — when a markdown ADR mentions `RetryPolicy`, that creates a cross-reference edge. This powers the `sift related` command.

### Embedding strategy

Both doc chunks and code chunks are embedded into the same vector space. Code chunks are formatted as `"symbol: {name} | code: {content}"` (analogous to QMD's `"title: {title} | text: {content}"` format for docs). This means `sift vsearch` and `sift query` search across both indexes seamlessly.

### Language support

Tree-sitter grammars compiled into the v1 binary: Rust, Python, TypeScript/JavaScript, Go. Adding a language means adding a grammar dependency and a small extraction config mapping Tree-sitter node types to symbol kinds.

---

## Roadmap and MVP Plan

### MVP (v0.1) — QMD-equivalent doc search in Rust

**Goal:** A working Rust binary that replicates QMD's core doc search experience.

**Scope:**
- `sift collection add/list/remove/rename` and `sift ls`
- `sift context add/list/rm/check`
- `sift update` (index collections)
- `sift search` (BM25 via FTS5/rusqlite)
- `sift get` and `sift multi-get`
- `sift status`
- Smart markdown chunking (port QMD's heading-aware algorithm)
- YAML-based collection and context config
- JSON, CSV, markdown, XML, files output formats
- Human-readable TTY output with color-coded scores

**Not in MVP:** Vector search, reranking, hybrid query, code indexing, symbol lookup.

**Why this cut:** The MVP validates the core proposition — can a single Rust binary deliver the same doc search quality as QMD without Node.js? If BM25 search over well-chunked markdown doesn't feel fast and useful, nothing else matters.

**Target:** 3–4 weeks.

### v0.2 — Vector search and hybrid query

**Goal:** Add embedding-based search and QMD's hybrid retrieval pipeline.

**Scope:**
- `sift embed` (generate embeddings via in-process GGUF model)
- `sift vsearch` (vector semantic search)
- `sift query` (hybrid: BM25 + vector + query expansion + RRF + reranking)
- sqlite-vec integration for vector storage and similarity
- GGUF model loading via llama.cpp Rust bindings (embedding, reranker, expansion)
- Model auto-download and cache management
- RRF fusion with position-aware blending (port QMD's tuning)

**Target:** 4–5 weeks after MVP.

### v0.3 — Code indexing and symbol search

**Goal:** Add Tree-sitter-backed code intelligence.

**Scope:**
- Tree-sitter integration for Rust and Python (two languages to validate the abstraction)
- Code file detection and routing through the code pipeline
- Symbol extraction: functions, structs/classes, types, imports, constants
- Symbol table storage
- `sift symbol` command
- Code-aware chunking for the vector index
- `--code` and `--docs` scope flags on search commands
- Enrich search results with symbol metadata

**Target:** 3–4 weeks after v0.2.

### v0.4 — Cross-references and related

**Goal:** Connect the doc and code indexes.

**Scope:**
- Cross-reference edge extraction (code→code via imports, doc→code via identifier mentions)
- `sift related` command
- Expand language support to TypeScript/JavaScript and Go
- Incremental re-indexing (only changed files, like QMD's content hashing)

**Target:** 3–4 weeks after v0.3.

### v0.5 — Agent integration hardening

**Goal:** Make Sifter reliable and fast as a tool called by AI agents in automated loops.

**Scope:**
- Stable JSON output schema (versioned)
- Performance benchmarking on large repos (100K+ files)
- Exit code conventions for scripting
- `--explain` flag (score traces, matching QMD's diagnostic output)
- Ruby and Java language support
- Documentation: integration guides for Claude Code, Codex, and similar agent frameworks

**Target:** 3–4 weeks after v0.4.

### Future (post-v0.5, not committed)

- **MCP server mode.** Optional stdio and HTTP transport, mirroring QMD's MCP implementation. Not a default dependency, but useful for clients that prefer it.
- **Watch mode.** Automatic re-indexing on file changes.
- **Per-repo config.** `.sifter.yml` for ignore patterns, language overrides, custom collection mappings.
- **Scope-aware search.** Search within a function, module, or directory subtree.
- **Named indexes.** QMD's `--index` flag for maintaining separate indexes for different contexts.
