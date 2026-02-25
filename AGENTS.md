# AGENTS.md — BYOKEY (Rust AI API proxy gateway)

## Architecture
Layered DAG in `crates/`: `types`(L0) → `config`,`store`(L1) → `auth`,`translate`(L2) → `provider`(L3) → `proxy`(L4). CLI entry: `src/main.rs` (bin=`byokey`).
- **types** — core traits (`TokenStore`, `ProviderExecutor`, `RequestTranslator`, `ResponseTranslator`), `ByokError`, `OAuthToken`, `ProviderId`
- **store** — SQLite token persistence (`sqlx`); `InMemoryTokenStore` for tests
- **auth** — per-provider OAuth flows + `AuthManager` (token lifecycle, 30s refresh cooldown)
- **translate** — pure OpenAI↔Claude↔Gemini format conversion (no auth dependency)
- **provider** — executor impls per provider + model registry + `CredentialRouter` (round-robin)
- **proxy** — axum HTTP server, SSE streaming, `/v1/chat/completions`, `/v1/models`
- **Key constraint:** `translate` must NOT depend on `auth`; `auth` must NOT depend on `translate` or `provider`; `types` has zero workspace deps.

## Code Style
- `unsafe_code = "forbid"`, `clippy::pedantic = "warn"`, edition 2024, async traits via `async-trait` macro
- HTTP client is `rquest` (NOT reqwest); HTTP server is `axum 0.8`; config via `figment`; errors: `thiserror` cross-crate (`ByokError`), `anyhow` crate-internal
- OAuth credentials fetched at runtime. (see `crates/auth/src/credentials.rs`)
- Don't call `unwrap_err()` on Results (`Box<dyn ProviderExecutor>` isn't `Debug`); use `is_err()` or pattern match
- See `CLAUDE.md` for full provider OAuth details, dependency tables, and API endpoint docs
