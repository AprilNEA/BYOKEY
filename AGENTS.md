# AGENTS.md — BYOKEY (Anthropic Messages gateway for Claude Code / Claude Desktop)

## Purpose
BYOKEY serves the Anthropic Messages API (`/v1/messages`, `/v1/messages/count_tokens`, `/v1/models`) on `:8018` and fulfils it from GitHub Copilot, Cursor or Anthropic itself, so Claude Code and Claude Desktop run on those subscriptions. There is no OpenAI-format route, no model registry beyond Anthropic's own ids, and no provider other than `claude`, `copilot`, `cursor`.

## Architecture
Layered DAG in `crates/`: `types`(L0) → `config`,`store`(L1) → `auth`(L2) → `provider`,`proto`(L3) → `proxy`(L4). `daemon` sits outside the DAG and is consumed only by the CLI binary (`src/main.rs`, bin=`byokey`). `tui` is a management client that talks to BYOKEY through the ConnectRPC management API rather than linking to server internals.
- **types** — `TokenStore`, `UsageStore`, `ByokError` (with `from_response` for upstream failures), `OAuthToken`, `ProviderId { Claude, Copilot, Cursor }`, `CopilotClient`, `ThinkingCapability`
- **store** — SQLite token/usage persistence via `sea-orm v2` + `sea-orm-migration`; `InMemoryTokenStore` for tests. Migrations 3–4 (conversations/messages) stay registered so existing databases keep a valid history, but nothing writes those tables.
- **auth** — Claude PKCE, Copilot device-code (OpenCode or VS Code client), Cursor browser login / `crsr_` key exchange, Claude Code credential import; `AuthManager` (token lifecycle, 30s refresh cooldown, background refresh loop: 60s interval / 5min lead)
- **provider** — `copilot` (credentials, quota-aware account selection, `/models` catalog, client identity headers fetched from `assets.byokey.io/versions/copilot.json`), `cursor` (agent.v1 client yielding `aigw_core` stream events), `claude` (Anthropic version/beta/fingerprint headers), `cloak` (Claude Code billing header + tool-name remapping for OAuth), `device_profile`, `registry` (Anthropic ids and thinking capability)
- **proto** — ConnectRPC schema generated from `crates/proto/proto/*.proto` via `connectrpc-build` + `buffa`: `StatusService { GetStatus, GetUsage }`, `AccountsService { ListAccounts }`, plus the `client` feature the TUI uses. Build-time dep on `protoc`. Isolated from the workspace `unsafe_code = "forbid"` lint because buffa's generated code uses `unsafe impl`.
- **proxy** — axum server: `handler/messages.rs` routes by `copilot/`/`cursor/` prefix, then `providers.claude.backend`, then Anthropic; `normalize.rs` shapes request bodies, `copilot_messages.rs` and `cursor_messages.rs` serve the other two upstreams, `forward.rs` relays the answer; `count_tokens.rs`, `models.rs`; ConnectRPC management as the fallback service; `ApiError` renders the Anthropic error envelope and forwards upstream status/body/`retry-after` verbatim. `http.rs` builds the one upstream client (connect timeout, HTTP/2 PING); `util/stream.rs` keeps SSE responses alive toward Claude Code (`: keepalive` after 10 s of upstream silence, first-byte grace 15 s, 120 s silence cutoff) and always ends them with `message_stop` or `error`.
- **daemon** — PID/process management, Unix control socket (`~/.byokey/control.sock`, tarpc), OS service registration (launchd/systemd/Windows SCM)
- **Key constraint:** `auth` must NOT depend on `provider` or `proxy`; `types` has zero workspace deps; `tui` uses `proto`/ConnectRPC only.

## Code Style
- `unsafe_code = "forbid"`, `clippy::pedantic = "warn"`, edition 2024, async traits via `async-trait` macro (ConnectRPC handlers use plain `async fn`)
- HTTP client is `reqwest` (rustls, platform root store), built once by `byokey_proxy::http::upstream_client` with TCP and HTTP/2 keepalives; HTTP server is `axum 0.8`; config via `figment`; errors: `thiserror` cross-crate (`ByokError`), `anyhow` crate-internal
- OAuth app credentials are fetched at runtime (see `crates/auth/src/credentials.rs`)
- Socket activation supported: `serve` adopts an inherited fd via `listenfd` (systemfd/systemd/launchd) if one is passed in; otherwise binds fresh
- **Build-time dep on `protoc`** — needed by `byokey-proto`'s build.rs. Install via `brew install protobuf` / `apt-get install protobuf-compiler`.
- Probe Copilot with a scratch instance before changing what is sent to it: `HOME=/tmp/bk-test byokey serve --config … --port 18126 --db /tmp/bk-test/.byokey/tokens.db` (a short `HOME` keeps the control socket path under `SUN_LEN`).
