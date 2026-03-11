# sifter
A local-first search engine for code and documentation, combining BM25, vector search, and Tree-sitter code intelligence in a single binary.

## Quick start

```bash
sifter config collection add . --name repo
sifter index update
sifter search "retry budget" --docs
sifter search --symbol SearchCommand --defs
sifter show sifter://repo/crates/sifter-cli/src/main.rs:1 -l 10 --line-numbers
```

By default, new collections ignore common VCS and build junk such as `.git`, `target`, `dist`, `build`, `.direnv`, and `node_modules`. Repository `.gitignore` files are also respected during indexing.

## Current CLI

The current MVP supports:
- `sifter config collection add|list|show|remove|rename|include|exclude|update-cmd`
- `sifter config context add|global|list|check|rm`
- `sifter index update|status`
- `sifter search <query>` with `--docs`, `--code`, `--files`, `--csv`, `--md`, `--xml`, and `--full`
- `sifter search --symbol <name>` with `--defs` and `--refs`
- `sifter search --related <path>`
- `sifter search --semantic <query>` and `sifter search --hybrid <query>` as deferred-runtime placeholders
- `sifter show <ref...>` with path, `sifter://...`, or `#docid` references, plus `:line`, `-l`, and `--line-numbers`

If `sifter index update` produces an empty index, the command now returns a warning payload in JSON mode and a direct hint in terminal mode to review the collection path, mask, and ignore rules.

Example JSON-oriented flow:

```bash
sifter config collection add . --name repo
sifter index update --json
sifter search "retry budget" --docs --json
sifter search --symbol SearchCommand --defs --json
sifter search --related crates/sifter-cli/src/main.rs --json
sifter show sifter://repo/README.md:20 -l 10 --line-numbers --json
```

## Development

Enter the pinned development shell with `nix develop`. If you use direnv, `.envrc` enables this automatically.

The default shell is stable Rust and includes:
- `cargo fmt`
- `cargo clippy`
- `cargo nextest`
- `cargo deny`
- `cargo audit`
- `cargo outdated`
- `cargo hack`
- `cargo llvm-cov`
- `cargo mutants`

Use the fast local verification loop during active work:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --workspace --all-features
cargo build --workspace --all-features
```

The nightly-oriented shell is available with `nix develop .#nightly`. Use it only for checks that require nightly:

```bash
cargo +nightly udeps --workspace
cargo +nightly miri test -p sifter
```
