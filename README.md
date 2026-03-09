# sifter
A local-first search engine for code and documentation, combining BM25, vector search, and Tree-sitter code intelligence in a single binary.

## Current CLI

The current MVP supports:
- `sifter collection add`
- `sifter collection list`
- `sifter context add|list|check|rm`
- `sifter update`
- `sifter status`
- `sifter search`
- `sifter symbol`
- `sifter related`
- `sifter get`
- `sifter multi-get`
- `sifter embed|vsearch|query` as explicit deferred-runtime placeholders

Example:

```bash
sifter collection add . --name repo
sifter update --json
sifter search "retry budget" --json
sifter symbol RetryPolicy --json
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
