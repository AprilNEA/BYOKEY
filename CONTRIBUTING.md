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
byokey-proxy  (axum HTTP server)
    │  resolve model → provider
    ▼
byokey-provider  (executor per provider)
    │  get OAuth token (or api_key)
    ▼
byokey-auth  (AuthManager + OAuth flows)
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
| `byokey-store` | 1 | SQLite token/usage persistence (sea-orm v2 + sea-orm-migration) |
| `byokey-auth` | 2 | OAuth flows (does not depend on provider / proxy) |
| `byokey-provider` | 3 | Provider Executor + model registry + `VersionStore`; protocol conversion lives in `aigw` |
| `byokey-proto` | 3 | ConnectRPC management API schema and generated client/server protocol types |
| `byokey-proxy` | 4 | axum HTTP server, routing, SSE passthrough, ConnectRPC management fallback |
| `byokey-tui` | — | ratatui management client using the ConnectRPC API |
| `byokey-daemon` | — | Process/service management, PID file, Unix control socket (separate from the layered DAG — used by the CLI binary only) |

CLI entry point: `src/main.rs` (package = `byokey`, bin = `byokey`).

### API Endpoints

`byokey serve` binds two HTTP listeners:

**Main listener (default `:8018`)**

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/chat/completions` | OpenAI-compatible chat (streaming supported) |
| `POST` | `/v1/messages` | Anthropic-compatible messages |
| `POST` | `/v1/responses` | Codex Responses API passthrough |
| `GET` | `/v1/models` | List enabled models |
| `ANY` | `/v0/management/*` | Accounts / usage / ratelimits management API |
| `GET` | `/openapi.json` | OpenAPI 3.1 spec for the management API |

**Amp listener (default `:18018`, set via `amp.port`)** — shaped to match what
the Amp CLI sends on the wire; `new URL(path, ampUrl)` in Amp's JS drops any
base path, so there is no `/amp` prefix.

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/auth/cli-login` | Amp CLI login redirect |
| `GET` | `/v1/login` | Amp web login redirect |
| `ANY` | `/v0/management/{*path}` | Amp management API proxy |
| `POST` | `/api/provider/anthropic/v1/messages` | Anthropic-compatible chat via byokey |
| `POST` | `/api/provider/openai/v1/chat/completions` | OpenAI-compatible chat via byokey |
| `POST` | `/api/provider/openai/v1/responses` | Codex Responses API via byokey |
| `POST` | `/api/provider/google/v1beta/models/{action}` | Gemini native passthrough |
| `ANY` | `/api/{*path}` | Catch-all: forward remaining `/api/*` to ampcode.com |

The `model` field in the request body determines which provider is used.

### Daemon and control socket

`serve` also binds a Unix control socket at `~/.byokey/control.sock`. The
`stop`, `restart`, and `reload` CLI subcommands talk to the running server
over this socket via tarpc. `start` forks a detached child and monitors its
PID file. At startup, `serve` adopts an inherited listener fd if one is passed
in by `systemfd` / `systemd` / `launchd` socket activation; otherwise it binds
`host:port` fresh. An in-process background loop refreshes OAuth tokens every
60s with a 5min lead, and a `VersionStore` fetches runtime User-Agent /
fingerprint strings from `https://assets.byokey.io/versions/{provider}.json`
at startup (falling back to compile-time defaults on network failure).

## Commit Convention

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(auth): add Kiro device code flow
fix(proxy): handle empty SSE chunk
refactor(translate): simplify gemini response parser
```
