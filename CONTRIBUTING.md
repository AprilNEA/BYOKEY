# Contributing to byokey

## Requirements

- Rust stable (recommended via [rustup](https://rustup.rs/))
- SQLite 3 (system-level; pre-installed on macOS)

## Common Commands

```bash
# Build everything
cargo build --workspace

# Run tests
cargo test --workspace
cargo test -p byokey-types          # single crate (-p uses package name)
cargo test -p byokey-translate      # pure logic tests (no I/O, fastest)

# Lint (CI runs with -D warnings, same locally)
cargo clippy --workspace -- -D warnings

# Format
cargo fmt --all

# Start the CLI server
cargo run -- serve --config config.yaml
```

## Coding Guidelines

- `unsafe` code is forbidden at the workspace level (`forbid`)
- `clippy::pedantic` is enabled; ensure zero warnings before committing
- edition 2024
- All async traits use the `async-trait` macro
- Error types: use `ByokError` across crate boundaries, `anyhow` within a crate

## Workspace Layering Rules

Strict DAG — no reverse cross-layer dependencies:

```
Layer 0  byokey-types     — core types & traits, zero intra-workspace deps
Layer 1  byokey-config    — configuration parsing
         byokey-store     — token persistence
Layer 2  byokey-auth      — OAuth flows (does not depend on translate / provider)
         byokey-translate — format conversion (pure functions, does not depend on auth)
Layer 3  byokey-provider  — Provider Executor
Layer 4  byokey-proxy     — axum HTTP server
```

## Commit Convention

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(auth): add Kiro device code flow
fix(proxy): handle empty SSE chunk
refactor(translate): simplify gemini response parser
```
