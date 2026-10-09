# Contributing to BYOKEY

## Requirements

With Nix and [devenv](https://devenv.sh/) installed, run:

```sh
devenv shell
```

The environment provides Rust, Cargo, Clippy, rustfmt, rust-analyzer, a C compiler, and `protoc`. Commit `devenv.lock` to keep the tool versions reproducible. Run `devenv update` to update the pinned inputs.

To run one command without opening a shell:

```sh
devenv shell cargo check --workspace --all-targets
```

If your shell already uses direnv, review `.envrc` and run `direnv allow` to activate the environment when entering this directory.

Without devenv, install Rust stable 1.91+, a C compiler, and `protoc`. SQLite is compiled from the bundled source by Cargo.

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
- HTTP clients use `reqwest` (rustls, platform root store), with shared proxy and keepalive settings in `crates/proxy/src/http.rs`. Protocol-aware requests share the upstream client; native Codex forwarding uses a separate client without automatic decompression or retries.
- HTTP server is `axum 0.8`

## Architecture

```
Anthropic Messages request  (Claude Code, Claude Desktop, …)
    │
    ▼
byokey-proxy  (axum HTTP server)
    │  provider qualifier, else `anthropic.routes`: the model's, its family's, the default; else Anthropic
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
| `byokey-proxy` | 4 | axum HTTP server, Responses and Messages routing, native Codex HTTP forwarding, ConnectRPC management fallback |
| `byokey-tui` | — | ratatui management client using the ConnectRPC API |
| `byokey-daemon` | — | Process/service management, PID file, Unix control socket (separate from the layered DAG — used by the CLI binary only) |

CLI entry point: `src/main.rs` (package = `byokey`, bin = `byokey`).

### API Endpoints

`byokey serve` binds a single HTTP listener (default `:8018`):

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/messages` | Anthropic Messages API (streaming supported) |
| `POST` | `/v1/messages/count_tokens` | Token counting, routed like `/v1/messages` |
| `GET` | `/v1/models` | The Anthropic models `/v1/messages` serves under the routes |
| `POST` | `/v1/responses`, `/codex/responses` | Responses passthrough to ChatGPT, Copilot or a custom upstream |
| `GET` | `/codex/models` | Codex-native model metadata and configured aliases |
| Any HTTP method | Other `/codex/*` paths | Raw HTTP forwarding to the configured ChatGPT backend; no WebSocket support |
| `POST` | `/byokey.*.*Service/{Method}` | ConnectRPC management API (status, accounts, usage) |

An explicit `<provider>/<model>` or `<model>[<provider>]` on the `model` field picks a built-in or configured Messages provider for
one request; otherwise `anthropic.routes.models` does for that Claude model, then
`anthropic.routes.families` for its family, then `anthropic.routes.default`, and without any the
request goes to Anthropic.
`byokey route` edits the routes and lists them through the management API.

Custom Messages providers use `providers.<name>.anthropic` for their URL, model discovery, credentials and headers. These settings are independent of Responses and stored logins. `handler/custom_messages.rs` builds native requests; the existing Anthropic forwarder handles responses. Custom requests retain thinking signatures and skip Claude OAuth remapping. The catalog keeps routed built-in IDs and adds custom `[provider]`-tagged IDs, or bare IDs when routed to that custom provider. The bracket tag preserves Desktop's base-model Effort recognition. Upstream IDs containing `/` retain provider prefixes. `messages::route` strips the provider qualifier and optional `[1m]` before thinking normalization and returns the long-context flag for both request handlers. HTTP tests in `handler/custom_messages/tests.rs` cover simultaneous catalogs, explicit effort, credential isolation, per-request headers, JSON, SSE, counting and errors.

Responses routing is separate: an exact `responses.routes.models` alias takes priority over a provider prefix and `responses.routes.default`. Each alias names a `provider` and its `model`. The shared `providers` map owns connections, credentials, display names and model metadata overrides. The client owns ChatGPT credentials and refresh. Copilot reuses stored accounts, while custom providers use configured credentials. The Responses forwarder preserves JSON and SSE bytes and accounts for Responses terminal events; do not reuse the Anthropic terminal-event parser. HTTP mock tests in `crates/proxy/src/handler/responses/` cover credential isolation, model capabilities, aliases, errors and stream cancellation.

Native Codex forwarding in `handler/responses/passthrough.rs` always uses `providers.chatgpt.base_url`, without model routing or body parsing. Keep its HTTP client separate: automatic decompression would change the forwarded bytes and headers. Both clients use the proxy and connection settings from `http.rs`. `AppState::new` returns a `Result` because it builds the native client. The native client disables redirects and retries; it streams bodies without Responses event parsing or token accounting. Tests cover raw bytes, encoded paths, credential isolation, errors, redirects and streaming cancellation.

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
