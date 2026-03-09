# Build Sifter v1 on a Tantivy + SQLite Core

## Summary

- Create a Rust workspace named `sifter` with a `sifter` binary.
- Use QMD as the interface reference for collections, contexts, retrieval commands, output formats, and doc/result shape.
- Deliver a working first cut for docs + code lexical search, `symbol`, `related`, `get`, `multi-get`, `status`, and collection/context management.
- Expose `embed`, `vsearch`, and `query` now, but make them return a deterministic deferred-runtime error until the vector/runtime phase lands.
- Design code indexing around a language-plugin registry so adding languages does not require storage or CLI redesign.
- Set up Rust development through a Nix flake, using `untangle` as the reference shape for toolchain pinning, dev shells, and CI entrypoints.

## Key Changes

- Use `clap` for CLI parsing, `serde`/`serde_json`/`serde_yaml` for config and output, `rusqlite` with bundled SQLite for the catalog, `tantivy` for lexical indexing, `tree-sitter` plus `tree-sitter-rust` for initial code parsing, `ignore` and `globset` for filesystem walking, and `blake3` for content hashing.
- Store config at `~/.config/sifter/<index>.yml` and runtime state at `~/.cache/sifter/<index>/`.
- Preserve QMD-style YAML config keys: `global_context`, `collections.<name>.path`, `pattern`, `ignore`, `context`, `update`, `includeByDefault`. Accept `include_by_default` as a read alias, but write `includeByDefault`.
- Change the default collection `pattern` to `**/*` so one collection can index both docs and code. File classification decides whether a matched file becomes a doc record or a code record.
- Use SQLite for content-addressed blobs, file records, chunk metadata, symbol rows, reference/import edges, relation scores, and deferred vector/cache tables.
- Use one Tantivy lexical index with a `kind` field (`doc` or `code`) instead of separate lexical indexes.
- Implement a markdown/doc pipeline with heading-aware chunking, code-fence-safe boundaries, stored byte offsets, and stored line ranges.
- Implement a code pipeline that parses files through a pluggable Tree-sitter registry and emits normalized `CodeSpan` records for definitions, references, imports, and file-text/comment fallback spans.
- Introduce a `LanguagePlugin` interface with `name`, `file_matchers`, `language`, query/extractor definitions, and `extract(source, path) -> Vec<CodeSpan>`. Core indexing and search must depend only on normalized `CodeSpan` and `RelationEdge` records.
- Ship Rust as the first code language. Adding a new language must only require a new plugin module, grammar dependency, fixtures, and one registry entry.
- Keep the QMD-style CLI families under the `sifter` binary: `collection add/list/remove/rename/show/include/exclude/update-cmd`, `context add/list/rm/check`, `ls`, `update`, `status`, `search`, `get`, `multi-get`, `symbol`, `related`, `embed`, `vsearch`, `query`.
- Use `sifter://<collection>/<path>` virtual paths throughout. `get` must accept filesystem paths, virtual paths, or `#docid`, plus `:line` and `-l`.
- Default to JSON on non-TTY and human-readable output on TTY. Support `--json`, `--csv`, `--md`, `--xml`, `--files`, `--full`, and `--line-numbers`.
- Standard search result JSON must include `docid`, `file`, `collection`, `kind`, `title`, `context`, `score`, `snippet`, `line_start`, and `line_end`. Code hits also include `language`, `symbols`, `symbol_kind`, and `scope`.
- Make `symbol` definition-first by default, with `--defs` and `--refs` filters. Make `related` rank candidates from symbol overlap, reference edges, shared imports, and doc mentions.
- Wire `embed`, `vsearch`, and `query` into CLI help and parsing now, but return exit code `2` with a structured error payload in JSON/non-TTY mode: `{"error":"vector_runtime_pending","command":"<name>","message":"vector runtime is not implemented in this build yet"}`. TTY mode prints the same message in human-readable form.
- Make `status` report lexical readiness plus deferred vector state, including `has_vector_index: false` and `vector_runtime: "pending"`.

## Rust + Nix Environment

- Create `flake.nix`, `package.nix`, and `default.nix` following the `untangle` structure: pinned `nixpkgs`, `rust-overlay`, `flake-utils`, one packaged app, and Nix-native build inputs.
- Add `.envrc` with `use flake` so entering the repo provides the pinned toolchain automatically.
- Define a default stable Rust toolchain in the flake with extensions: `rustfmt`, `clippy`, `rust-src`, and `llvm-tools-preview`.
- Put these tools in the default dev shell:
  - `cargo-nextest`
  - `cargo-edit`
  - `cargo-deny`
  - `cargo-audit`
  - `cargo-outdated`
  - `cargo-llvm-cov`
  - `cargo-hack`
  - `cargo-mutants`
- Provide a second dev shell for nightly-only checks, with a nightly toolchain and support for:
  - `miri`
  - `cargo-udeps`
- Use stable by default for normal development and CI. Use the nightly shell only for checks that require nightly.
- Add `.config/nextest.toml` with a `ci` profile and JUnit output path.
- Add `deny.toml` for advisory, license, duplicate-version, and source policy checks.
- Document the Nix workflow in `README.md`: entering the shell, running the fast local verification loop, and when to use the nightly shell.

## CI and Quality Gates

- Run all CI steps through `nix develop` so local and CI environments stay aligned.
- Make fast PR checks run on every push/PR:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo nextest run --workspace --all-features`
  - `cargo deny check`
  - `cargo build --workspace --all-features`
- Run slower hygiene/security checks on a scheduled workflow or manual workflow:
  - `cargo audit`
  - `cargo outdated`
  - `cargo hack check --workspace --feature-powerset`
  - `cargo +nightly udeps --workspace`
- Run coverage in CI with `cargo llvm-cov`, initially as a reporting job. Add an enforcement threshold once the first implementation stabilizes.
- Run `miri` only on suitable pure-logic crates that do not depend on SQLite, Tantivy, or Tree-sitter FFI-heavy code paths.
- Run mutation testing only on critical pure-logic crates and modules, not the full workspace. Target ranking, chunking, context resolution, and relation-scoring logic first.

## Test Plan

- Add CLI integration tests for `collection`, `context`, `ls`, `update`, `status`, `search`, `symbol`, `related`, `get`, and `multi-get`.
- Add CLI tests that assert `embed`, `vsearch`, and `query` return the deferred-runtime contract and exit code `2`.
- Add unit tests for XDG config resolution, YAML round-trip, longest-prefix context matching, docid lookup, line slicing, collection filters, and `--docs`/`--code` filters.
- Add unit tests for markdown chunking so headings and code fences stay intact.
- Add code-intel tests for the Rust plugin covering definitions, references, scopes, and imports.
- Add formatter tests for JSON/files/CSV/Markdown/XML, including code metadata and omission of absent optional fields.
- Add a plugin-registry test with a minimal fake language plugin to prove new languages can be added without changing core indexing/search logic.
- Use acceptance fixtures that prove `sifter collection add . --name sifter && sifter update && sifter search ...`, `sifter symbol ...`, and `sifter related ...` all work on the `sifter` repo itself.

## Assumptions

- The first working code language is Rust; extensibility for additional languages is required immediately, but additional grammars do not ship in the first cut.
- QMD compatibility is behavioral rather than byte-for-byte: command families, config model, output formats, and result fields are mirrored where relevant, while storage and implementation stay Rust-native.
- MCP, remote indexing, type resolution, and live embedding/reranking runtime are deferred beyond this first implementation.
