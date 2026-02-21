# Contributing to BYOKEY

## Requirements

- Rust stable 1.85+ (recommended via [rustup](https://rustup.rs/))
- SQLite 3 (system-level; pre-installed on macOS)

## Common Commands

```bash
cargo build --workspace                   # Build everything
cargo test --workspace                    # Run all tests (no network required)
cargo test -p byokey-auth                 # Single crate
cargo test -p byokey-auth auth::pkce      # Single test module
cargo clippy --workspace -- -D warnings   # Lint (CI runs with -D warnings)
cargo fmt --all                           # Format
cargo run -- serve                        # Start proxy (default :8018)
```

## Coding Guidelines

- `unsafe` code is forbidden at the workspace level (`forbid`)
- `clippy::pedantic` is enabled; ensure zero warnings before committing
- edition 2024
- All async traits use the `async-trait` macro
- Error types: use `ByokError` (`thiserror`) across crate boundaries, `anyhow` within a crate
- HTTP client is `rquest` (not reqwest) — supports TLS fingerprint impersonation
- HTTP server is `axum 0.8`
- `Box<dyn ProviderExecutor>` does not implement `Debug`; don't call `unwrap_err()` on Results, use `is_err()` or pattern match

## Architecture

```
Client request
    │
    ▼
byok-proxy  (axum HTTP server)
    │  resolve model → provider
    ▼
byok-provider  (executor per provider)
    │  get OAuth token (or api_key)
    ▼
byok-auth  (AuthManager + OAuth flows)
    │
    ▼
Upstream API  (Anthropic / OpenAI / Google / …)
    │  translate response → OpenAI format
    ▼
Client response  (JSON or SSE stream)
```

### Workspace Crates

Strict layered DAG — no reverse cross-layer dependencies:

| Crate | Layer | Description |
|-------|:-----:|-------------|
| `byokey-types` | 0 | Core types, traits, errors (zero intra-workspace deps) |
| `byokey-config` | 1 | YAML configuration (figment) + file watching |
| `byokey-store` | 1 | SQLite token persistence (sqlx) |
| `byokey-auth` | 2 | OAuth flows (does not depend on translate / provider) |
| `byokey-translate` | 2 | Format conversion — pure functions (does not depend on auth) |
| `byokey-provider` | 3 | Provider Executor + model registry |
| `byokey-proxy` | 4 | axum HTTP server, routing, SSE passthrough |

CLI entry point: `src/main.rs` (package = `byokey`, bin = `byokey`).

### API Endpoints (default `:8018`)

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/chat/completions` | OpenAI-compatible chat (streaming supported) |
| `GET` | `/v1/models` | List enabled models |
| `GET` | `/amp/v1/login` | Amp login redirect |
| `ANY` | `/amp/v0/management/{*path}` | Amp management API proxy |
| `POST` | `/amp/v1/chat/completions` | Amp-compatible chat |

The `model` field in the request body determines which provider is used.

## Commit Convention

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(auth): add Kiro device code flow
fix(proxy): handle empty SSE chunk
refactor(translate): simplify gemini response parser
```
