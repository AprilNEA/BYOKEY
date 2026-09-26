# Contributing to BYOKEY

## Requirements

- Rust stable 1.98+ (recommended via [rustup](https://rustup.rs/))
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
- HTTP client is `wreq` (not reqwest) — supports TLS fingerprint impersonation
- HTTP server is `axum 0.8`

## Architecture

```
Anthropic Messages request  (Claude Code, Claude Desktop, …)
    │
    ▼
byokey-proxy  (axum HTTP server)
    │  `copilot/` or `cursor/` prefix, else `providers.claude.backend`, else Anthropic
    ▼
byokey-provider  (Copilot credentials + catalog, Cursor agent client, Claude headers)
    │  get OAuth token (or api_key)
    ▼
byokey-auth  (AuthManager + OAuth flows)
    │
    ▼
Upstream  (api.anthropic.com · api.githubcopilot.com /v1/messages · Cursor agent.v1)
    │  Anthropic SSE or JSON; Cursor events rendered as Anthropic SSE
    ▼
Client response
```

### Workspace Crates

Strict layered DAG — no reverse cross-layer dependencies:

| Crate | Layer | Description |
|-------|:-----:|-------------|
| `byokey-types` | 0 | Core types, traits, errors (zero intra-workspace deps) |
| `byokey-config` | 1 | YAML configuration (figment) + file watching |
| `byokey-store` | 1 | SQLite token/usage persistence (sea-orm v2 + sea-orm-migration) |
| `byokey-auth` | 2 | OAuth flows (does not depend on provider / proxy) |
| `byokey-provider` | 3 | Copilot credentials and catalog, Cursor agent client, Anthropic headers and model registry |
| `byokey-proto` | 3 | ConnectRPC management API schema and generated client/server protocol types |
| `byokey-proxy` | 4 | axum HTTP server, Messages routing and passthrough, ConnectRPC management fallback |
| `byokey-tui` | — | ratatui management client using the ConnectRPC API |
| `byokey-daemon` | — | Process/service management, PID file, Unix control socket (separate from the layered DAG — used by the CLI binary only) |

CLI entry point: `src/main.rs` (package = `byokey`, bin = `byokey`).

### API Endpoints

`byokey serve` binds a single HTTP listener (default `:8018`):

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/messages` | Anthropic Messages API (streaming supported) |
| `POST` | `/v1/messages/count_tokens` | Token counting, routed like `/v1/messages` |
| `GET` | `/v1/models` | The models `/v1/messages` can route |
| `POST` | `/byokey.*.*Service/{Method}` | ConnectRPC management API (status, accounts, usage) |

A `copilot/` or `cursor/` prefix on the `model` field picks the provider for
one request; otherwise `providers.claude.backend` does, and without it the
request goes to Anthropic.

### Daemon and control socket

`serve` also binds a Unix control socket at `~/.byokey/control.sock`. The
`stop`, `restart`, and `reload` CLI subcommands talk to the running server
over this socket via tarpc. `start` forks a detached child and monitors its
PID file. At startup, `serve` adopts an inherited listener fd if one is passed
in by `systemfd` / `systemd` / `launchd` socket activation; otherwise it binds
`host:port` fresh. An in-process background loop refreshes OAuth tokens every
60s with a 5min lead, and the Copilot client identity is fetched from
`https://assets.byokey.io/versions/copilot.json` at startup (falling back to
compile-time defaults on network failure).

## Commit Convention

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(auth): add the Cursor browser login
fix(proxy): handle empty SSE chunk
refactor(provider): split Copilot headers from credentials
```
