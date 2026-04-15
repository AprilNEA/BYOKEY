# AGENTS.md — BYOKEY (Rust AI API proxy gateway)

## Architecture
Layered DAG in `crates/`: `types`(L0) → `config`,`store`(L1) → `auth`,`translate`(L2) → `provider`(L3) → `proxy`(L4). `daemon` sits outside the DAG and is consumed only by the CLI binary (`src/main.rs`, bin=`byokey`).
- **types** — core traits (`TokenStore`, `UsageStore`, `ProviderExecutor`, `RequestTranslator`, `ResponseTranslator`), `ByokError`, `OAuthToken`, `ProviderId`
- **store** — SQLite token/usage persistence via `sea-orm v2` + `sea-orm-migration`; `InMemoryTokenStore` for tests
- **auth** — per-provider OAuth flows + `AuthManager` (token lifecycle, 30s refresh cooldown, background refresh loop: 60s interval / 5min lead)
- **translate** — pure OpenAI↔Claude↔Gemini format conversion (no auth dependency)
- **provider** — executor impls per provider + model registry + `CredentialRouter` (round-robin) + `VersionStore` (runtime-fetched UA/fingerprint strings from `assets.byokey.io/versions/{provider}.json`)
- **proxy** — axum HTTP server, SSE streaming, two listeners: main (`:8018`, OpenAI/Anthropic/management) and amp (`:18018` via `amp.port`)
- **daemon** — PID/process management, Unix control socket (`~/.byokey/control.sock`, tarpc), OS service registration (launchd/systemd/Windows SCM); not in the DAG, only used by the CLI binary
- **Key constraint:** `translate` must NOT depend on `auth`; `auth` must NOT depend on `translate` or `provider`; `types` has zero workspace deps.

## Code Style
- `unsafe_code = "forbid"`, `clippy::pedantic = "warn"`, edition 2024, async traits via `async-trait` macro
- HTTP client is `rquest` (NOT reqwest); HTTP server is `axum 0.8`; config via `figment`; errors: `thiserror` cross-crate (`ByokError`), `anyhow` crate-internal
- OAuth credentials fetched at runtime. (see `crates/auth/src/credentials.rs`)
- Don't call `unwrap_err()` on Results (`Box<dyn ProviderExecutor>` isn't `Debug`); use `is_err()` or pattern match
- Socket activation supported: `serve` adopts an inherited fd via `listenfd` (systemfd/systemd/launchd) if one is passed in; otherwise binds fresh
- See `CLAUDE.md` for full provider OAuth details, dependency tables, and API endpoint docs
