# AGENTS.md — BYOKEY (Rust AI API proxy gateway)

## Architecture
Layered DAG in `crates/`: `types`(L0) → `config`,`store`(L1) → `auth`,`translate`(L2) → `provider`,`proto`(L3) → `proxy`(L4). `daemon` sits outside the DAG and is consumed only by the CLI binary (`src/main.rs`, bin=`byokey`). `tui` is an upper-layer management client used by the CLI binary; it talks to BYOKEY through the ConnectRPC management API rather than linking to server internals.
- **types** — core traits (`TokenStore`, `UsageStore`, `ProviderExecutor`, `RequestTranslator`, `ResponseTranslator`), `ByokError`, `OAuthToken`, `ProviderId`
- **store** — SQLite token/usage persistence via `sea-orm v2` + `sea-orm-migration`; `InMemoryTokenStore` for tests
- **auth** — per-provider OAuth flows + `AuthManager` (token lifecycle, 30s refresh cooldown, background refresh loop: 60s interval / 5min lead)
- **translate** — pure OpenAI↔Claude↔Gemini format conversion (no auth dependency)
- **provider** — executor impls per provider + model registry + `CredentialRouter` (round-robin) + `VersionStore` (runtime-fetched UA/fingerprint strings from `assets.byokey.io/versions/{provider}.json`)
- **proto** — ConnectRPC service definitions generated from `crates/proto/proto/*.proto` via `connectrpc-build` + `buffa`. Owns the management API schema: `byokey.status.StatusService`, `byokey.accounts.AccountsService`, `byokey.amp.AmpService`, plus the optional protocol-only `client` feature for shared management API clients. Build-time dep on `protoc`. Isolated from the workspace `unsafe_code = "forbid"` lint because buffa's generated code uses `unsafe impl` for marker traits.
- **proxy** — axum HTTP server, SSE streaming, **single listener** on `:8018`. Serves three kinds of traffic from one port: REST AI proxy (`/v1/chat/completions` etc.), amp CLI compatibility (`/api/provider/*`, amp redirects, ampcode.com proxy), and ConnectRPC management as the fallback service (`/byokey.status.StatusService/{Method}`, `/byokey.accounts.AccountsService/{Method}`, `/byokey.amp.AmpService/{Method}`). The amp sub-router is wrapped in `forward_headers_middleware` scoped via `.layer()` before `.merge()`.
- **tui** — ratatui management UI. Depends on `byokey-proto` with the `client` feature and renders snapshots from `StatusService` / `AccountsService`; it must not depend on `byokey-daemon`, `byokey-auth`, or `byokey-store`, and must not read SQLite directly.
- **daemon** — PID/process management, Unix control socket (`~/.byokey/control.sock`, tarpc), OS service registration (launchd/systemd/Windows SCM); not in the DAG, only used by the CLI binary
- **Key constraint:** `translate` must NOT depend on `auth`; `auth` must NOT depend on `translate` or `provider`; `types` has zero workspace deps; `proto` owns protobuf-generated types and protocol-only client glue (no business logic); upper-layer apps such as `tui` use `proto`/ConnectRPC instead of bypassing `proxy` into `auth`, `store`, or `daemon`.

## Code Style
- `unsafe_code = "forbid"`, `clippy::pedantic = "warn"`, edition 2024, async traits via `async-trait` macro (ConnectRPC handlers use plain `async fn`)
- HTTP client is `rquest` (NOT reqwest); HTTP server is `axum 0.8`; config via `figment`; errors: `thiserror` cross-crate (`ByokError`), `anyhow` crate-internal
- OAuth credentials fetched at runtime. (see `crates/auth/src/credentials.rs`)
- Don't call `unwrap_err()` on Results (`Box<dyn ProviderExecutor>` isn't `Debug`); use `is_err()` or pattern match
- Socket activation supported: `serve` adopts an inherited fd via `listenfd` (systemfd/systemd/launchd) if one is passed in; otherwise binds fresh
- **Build-time dep on `protoc`** — needed by `byokey-proto`'s build.rs. Install via `brew install protobuf` / `apt-get install protobuf-compiler`.
- See `CLAUDE.md` for full provider OAuth details, dependency tables, and API endpoint docs
