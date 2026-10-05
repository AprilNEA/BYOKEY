# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Configure Responses model picker names with `responses.catalog`: MiniJinja templates, built-in provider labels, model name overrides, and exact aliases to hide. Valid changes hot-reload; rejected configurations retain the last valid settings.
- Discover Copilot Responses models from enabled stored credentials or an API key without requiring a model alias.

### Fixed

- Use consistent provider-labelled names for native ChatGPT, Copilot, and custom upstream models. Show at most one visible entry per upstream and model while retaining hidden aliases for existing sessions and explicit routing.
- Update test assertions for the Rust 1.99 Clippy rules used by CI without changing the asserted conditions.

## [4.0.0](https://github.com/AprilNEA/BYOKEY/compare/v3.0.0...v4.0.0) - 2026-10-03

### Added

- *(proxy)* forward native Codex HTTP endpoints
- *(proxy)* discover custom Responses models
- *(proxy)* support upstream tiers and generated request IDs
- *(proxy)* add a Responses gateway for Codex clients
- *(cli)* accept `mythos` for Fable and warn about unusable routes
- [**breaking**] route Claude models to providers with `byokey route`
- *(cli,config)* apply log.level on reload and reject a filter that hides logs
- *(auth,provider)* log how long each side lookup takes
- *(proxy)* name the requested model on every line of a request
- *(proxy)* correlate a request with Claude Code's ids
- *(proxy)* log each upstream exchange once, when it ends
- *(provider)* send the requested effort to Cursor
- *(types)* name Claude models by family and version

### Fixed

- *(proxy)* stabilize Copilot Responses item IDs
- *(proxy)* handle live Codex catalogs and untyped SSE responses
- *(config,cli)* reload a config file that is created or renamed over
- *(provider,proxy)* start serving before the Copilot client versions arrive
- *(provider,proxy)* attribute Copilot usage to the account that served it
- *(proxy)* log a failed request once, at a level that says who failed
- *(provider)* retry a failed Copilot endpoint lookup within a minute
- *(proxy)* list the last model catalog when a live one is slow

### Other

- refresh gateway status and Codex setup
- *(dev)* initialize devenv environment
- archive the project and point to direct Copilot use ([#120](https://github.com/AprilNEA/BYOKEY/pull/120))
- *(proxy)* split the Messages handler by upstream

### Added

- Each request BYOKEY sends upstream is logged once, when it ends, with its outcome (`completed`, `rejected`, `failed` or `abandoned`), the time to the upstream's first byte and in total, the input, output and cache token counts, the stop reason, the keepalives written while the upstream was silent, the upstream's request id and, on Copilot, whether the request was user- or agent-initiated. A stream the client abandons, such as Esc in Claude Code, is now visible; it left no trace before. See the new Logs section of the README.
- The lookups BYOKEY makes besides the request itself are logged with how long they took: exchanging a Copilot or Cursor token, looking up a Copilot account's API host, Copilot quotas, the Copilot and Cursor model catalogs, and refreshing an OAuth token. The Copilot client versions fetched at startup are logged too. A slow `/v1/models` or a slow first request now shows which lookup took the time.
- A change to `log.level` applies when the configuration reloads, without a restart, unless `RUST_LOG` is set.
- A request's log lines carry BYOKEY's request id, which the client receives as `x-request-id`, and Claude Code's `x-client-request-id` and session id, so a request in `claude --debug` output can be found in BYOKEY's log.

### Fixed

- A streamed answer that ended in an upstream `error` event was counted as a success in the usage statistics; it is a failure now. Cursor answers requested without streaming are counted now; they were missing.
- Copilot usage was recorded against the account `default` whichever stored account the request went out as, so with several Copilot accounts the statistics could not tell them apart. Usage and log lines name the account that served the request now.
- Logs written to a redirected stdout, such as the Homebrew service's log file, no longer contain colour codes, and `NO_COLOR` is respected.
- A successful Copilot answer carried Copilot's `x-request-id` to the client instead of BYOKEY's, so the id the client saw matched no BYOKEY log line.

### Changed

- The per-request "routing Anthropic messages through Copilot" and "anthropic passthrough" lines are `debug` now, and a Copilot request's conversation keys print as eight hex digits instead of 64 numbers. A stream that fails is logged once, in its exchange line, instead of twice at `error`.
- A lookup that fails and falls back to a default is a `warn` instead of `debug` or nothing: the Copilot client versions (the built-in ones are used), a Copilot account's API host (the default host is used), and a token about to expire that could not be refreshed ahead of time. A revoked refresh token is a `warn` naming the login command to run, instead of an `error`.
- A `log.level` or `RUST_LOG` that does not parse stops `serve` with an error instead of being ignored, and `log.level` must set a default level: a typo such as `degub` was read as a module name and turned every other log line off. The default `log.level` is `info,tarpc=warn`, which drops the five lines the control socket logged for each `byokey status`.
- Management API calls, which `byokey tui` makes every few seconds, are logged at `debug`, and a request's line shows its path without the query string.
- The requested model and streaming mode are fields of the request's `http` span instead of a separate `anthropic_messages` span, so every line of a request names them, including "response sent" and a failure rendered after the handler returned. `count_tokens` requests name their model too.

- A failed request is logged once, when its error response is rendered, at a level that says who failed. An upstream's refusal is a `warn` carrying the upstream's status, error type and message, so the log says why Copilot or Anthropic refused (an organisation policy, a context limit) instead of only `status=400`; the message stays out of Sentry because an upstream can quote the request back. A missing login, an unknown model or an unreachable upstream is a `warn`, and only a failure inside BYOKEY is an `error`, so Sentry no longer receives an error event for every upstream 4xx and 5xx.

## [3.0.0](https://github.com/AprilNEA/BYOKEY/compare/v2.1.0...v3.0.0) - 2026-09-27

### Added

- *(proxy)* list models in lineup order with their release dates
- *(provider)* refresh the model registry to current upstream ids
- *(cli)* log the OS service by default and check Claude Desktop in doctor
- *(proxy)* cheaper Claude Code turns on Copilot
- *(cli)* open Claude Desktop against BYOKEY next to the official one
- *(cli)* add `byokey claude desktop`
- *(cli)* add `byokey doctor`
- Claude Code gateway model discovery
- *(proxy)* announce 1M-context models on /v1/models
- *(proxy)* list the models each endpoint can route
- *(proxy)* pass upstream errors through and accept [1m] model ids
- *(proxy)* serve /v1/messages/count_tokens

### Fixed

- *(provider,proxy)* learn which server tools a Copilot account's policy rejects
- *(proxy)* drop the server tools Copilot rejects
- *(proxy)* keep Claude Code's stream alive while the upstream is silent
- *(proxy)* end truncated Messages streams and re-exchange rejected Copilot tokens
- *(proxy)* let cursor/<model> override claude.backend on /v1/messages

### Other

- *(daemon)* one ServeOptions for background and service starts
- *(cli)* type Claude Desktop's deployment mode
- *(cli)* name the resolved target and the doctor report
- *(deps)* replace wreq with reqwest
- [**breaking**] make ProviderId Copy and pass it by value
- [**breaking**] keep only the Anthropic Messages gateway for Claude Code and Claude Desktop
- document claude desktop, doctor and model discovery
- [**breaking**] return stored tokens as AccountToken
- convert management API types with From
- *(proxy)* record usage through a typed Attribution
- *(auth)* name the PKCE pair and the refresh cooldown
- *(provider)* point REJECTED_TOOLS at the method that records it
- *(provider)* type Cursor's model parameters
- type the model catalogs and /v1/models responses
- *(proxy)* order models by a derived Ord key
- Revert "refactor(proxy): route /v1/messages like /v1/chat/completions"
- *(proxy)* route /v1/messages like /v1/chat/completions

### Breaking

- BYOKEY is now an Anthropic Messages gateway for Claude Code and Claude Desktop. `/v1/chat/completions`, `/v1/responses` and `/openapi.json` are gone; `/v1/models` lists what `/v1/messages` routes.
- Only `claude`, `copilot` and `cursor` remain. Codex, Gemini, Kiro, Antigravity, Qwen, Kimi and iFlow, `byokey import-codex` and the `codex/…` model prefix are removed. A config that still has a `providers` entry for one of them fails to load; stored tokens for them are ignored.
- Config keys `model_alias`, `excluded_models`, `streaming`, `payload` and `routing_policies`, and the provider keys `api_keys`, `routing`, `fallback`, `max_retry_credentials`, `claude_headers`, `codex_headers`, `cloak` and `websocket`, are removed and rejected.
- The management API keeps `GetStatus`, `GetUsage` and `ListAccounts`, which is what `byokey tui` reads; the other RPCs are removed.

### Fixed

- Claude Code no longer sits on `Waiting for API response · will retry in …` while Copilot is slow to answer: BYOKEY answers a streaming request itself once the upstream's headers are 15 s late, writes an SSE keepalive comment every 10 s the upstream stays silent, and ends a stream that has been silent for two minutes with an `error` event instead of leaving the client to give up after five.
- Dead upstream connections are noticed within about half a minute (HTTP/2 PING every 15 s, also while idle) and their streams end with an `error` event, so the retry lands on a fresh connection instead of the same dead one. Body errors are logged with their full cause chain.
- An invalid `proxy_url` now fails startup instead of silently sending traffic directly.
- A Copilot organisation policy that turns off the `web_search` or `web_fetch` server tools no longer fails the turn: BYOKEY learns the rejection from Copilot's 400, retries without the tool, and leaves it out of that account's later requests. Accounts whose policy allows the tools keep them.
- Copilot requests go to the API host GitHub names for the account (`endpoints.api` in `/copilot_internal/user`, e.g. `api.enterprise.githubcopilot.com` for organisation seats), as VS Code does, instead of always `api.githubcopilot.com`. `providers.copilot.base_url` still overrides it.

### Changed

- The HTTP client is `reqwest` instead of `wreq`. Nothing used `wreq`'s TLS impersonation, and `reqwest` was already in the dependency tree twice. TLS certificates are now verified against the operating system's trust store instead of a bundled Mozilla root set, so a CA installed on the machine (a corporate proxy, for example) is trusted, and a container without `ca-certificates` needs them installed. The minimum supported Rust version drops from 1.98 to 1.91.

## [2.1.0](https://github.com/AprilNEA/BYOKEY/compare/v2.0.0...v2.1.0) - 2026-09-24

### Added

- *(cursor)* add Cursor provider
- *(cli)* add `byokey claude start` to launch Claude Code against BYOKEY

### Fixed

- *(proxy)* drop request fields Copilot's Messages API rejects

### Other

- *(deps)* bump aigw to 0.7.0

## [2.0.0](https://github.com/AprilNEA/BYOKEY/compare/v1.5.0...v2.0.0) - 2026-09-24

### Breaking

- remove Amp support: the `/api/*` Amp routes, `byokey amp inject`, the `amp` config section and the `AmpService` management RPC are gone, and a config that still lists `providers.amp` fails to load

### Added

- *(cli)* add Claude Code quick injection ([#101](https://github.com/AprilNEA/BYOKEY/pull/101))

## [1.5.0](https://github.com/AprilNEA/BYOKEY/compare/v1.4.1...v1.5.0) - 2026-09-24

### Added

- *(copilot)* log in and send requests as OpenCode by default

### Fixed

- *(ci)* pin sentry-cli and allow rebuilding a release's missing targets

## [1.4.1](https://github.com/AprilNEA/BYOKEY/compare/v1.4.0...v1.4.1) - 2026-09-23

### Fixed

- *(deps)* upgrade aigw to 0.6.1 for Copilot's trimmed envelope

### Other

- *(desktop)* remove the macOS desktop app
- update Cargo.toml dependencies

## [1.4.0](https://github.com/AprilNEA/BYOKEY/compare/v1.3.0...v1.4.0) - 2026-09-23

### Added

- *(copilot)* send the headers VS Code 1.139 sends, read from its source

### Fixed

- *(copilot)* let the Copilot API compress chat responses
- *(copilot)* align request headers with current Copilot Chat
- *(proxy)* stop forwarding upstream body framing on /v1/messages

### Other

- *(release)* include internal crates' commits in the byokey changelog

## [1.3.0](https://github.com/AprilNEA/BYOKEY/compare/v1.2.0...v1.3.0) - 2026-09-23

### Added

- *(auth)* headless login URL + import-codex + TUI
- harden Claude path against reverse-proxy fingerprinting
- *(provider)* gate anthropic-dangerous-direct-browser-access to API-key mode
- *(auth)* update Claude OAuth scopes to align with Claude Code 2.1.88+

### Fixed

- *(deps)* migrate HTTP client from rquest to wreq. Every published `rquest` version is yanked, so `cargo install byokey` could no longer resolve
- *(deps)* upgrade connectrpc 0.3 → 0.9 and clear the remaining advisories
- *(deps)* patch h2, rustls and quinn-proto advisories
- *(ci)* unblock releases and move actions off the removed Node 20
- *(proxy)* allow unused_async_trait_impl on ConnectRPC handlers
- *(tui)* use management API client
- *(clippy)* backtick OpenAI / ConnectRPC in doc comments
- *(auth,store)* address codex review + CI

### Other

- *(deps)* bump aigw 0.4 → 0.5 (from crates.io)
- *(byokey)* Antigravity streaming via aigw + docs cleanup
- *(byokey)* drop apply_thinking + delete byokey-translate crate
- *(byokey)* AmpCode native-Gemini handler uses aigw bridge
- *(byokey)* inline messages.rs helpers + Antigravity uses aigw-gemini
- *(translate,proxy)* thread canonical thinking + drop aigw-covered modules
- *(deps)* switch to local aigw + drop redundant inject_cache_control
- *(release)* mirror byokey-tui publish=false in release-plz.toml
- *(release)* mark byokey-tui as publish=false
- fix CI clippy lints (cloak doc-backticks + items-after-test)
- Add `pullfrog.yml` workflow

### Note

The minimum supported Rust version is now 1.98, raised from 1.85 by `wreq`.

## [1.2.0](https://github.com/AprilNEA/BYOKEY/compare/v1.1.0...v1.2.0) - 2026-04-18

### Added

- *(telemetry)* add Sentry for Rust daemon and Swift desktop

## [1.1.0](https://github.com/AprilNEA/BYOKEY/compare/v1.0.0...v1.1.0) - 2026-04-18

### Added

- *(accounts)* add dialog with 3 authentication methods

### Fixed

- round-2 review fixes — login flow, proto, Swift polish
- *(desktop)* streaming login error handling, clean up dead state
- *(ci)* skip build + homebrew update for per-crate release tags
- *(release-plz)* suppress individual tags for byokey-proto and ampcode

### Other

- *(release-plz)* disable semver_check
- round-3 review polish
- tighten security exit code, workspace-pin streaming deps, add tests
- *(desktop)* migrate CLIRunner to ConnectRPC

### Added
- `AccountsService.AddApiKey` and `AccountsService.ImportClaudeCode` RPCs.
- `AccountsService.Login` server-streaming RPC with live `LoginEvent` progress.
- `AmpService.InjectUrl` RPC.
- CLI subcommands `add-api-key` (optional stdin via `-`) and `import-claude-code`.

### Changed
- Desktop app calls ConnectRPC directly instead of shelling out via `CLIRunner`.

### Removed
- `desktop/Byokey/Services/CLIRunner.swift` — `login`, `addApiKey`,
  `importClaudeCode`, and `ampInject` migrated to RPC; `ampAdsDisable`/
  `ampAdsEnable` removed entirely (the underlying `amp ads` CLI subcommand
  was removed in commit f183662, so these had been pointing at nothing).
- Dead amp-ads UI from `AmpView.swift`.

## [1.0.0](https://github.com/AprilNEA/BYOKEY/compare/v0.11.0...v1.0.0) - 2026-04-18

First stable release. API surface (HTTP proxy endpoints, CLI commands,
config schema, ConnectRPC management service) is now covered by semver.

### Added

- Simplified Chinese README under `docs/README_CN.md` with bilingual switcher
- Light/dark hero screenshots rendered via `<picture>` + `prefers-color-scheme`

### Fixed

- *(ci)* unblock release-plz against historical loadwise path-deps
- *(ci)* unblock Test, Clippy, and Desktop (macOS) jobs
- *(desktop)* swap menu bar icon from `server.rack` to `key.fill`

## [0.11.0](https://github.com/AprilNEA/BYOKEY/compare/v0.10.0...v0.11.0) - 2026-04-09

### Fixed

- *(provider)* align Claude request fingerprint with real CLI behavior
- *(readme)* update Amp configuration path in README ([#63](https://github.com/AprilNEA/BYOKEY/pull/63))
- *(release-plz)* disable internal crate changelogs and add cliff.toml
- *(claude)* normalize temperature for thinking and update beta headers
- *(desktop)* add update menu item and fix CI for x86_64 appcast

### Other

- *(provider)* migrate ClaudeExecutor to aigw translation layer
- *(desktop)* redesign Activity dashboard and Accounts view

## [0.10.0](https://github.com/AprilNEA/BYOKEY/compare/v0.9.2...v0.10.0) - 2026-04-04

### Fixed

- *(proxy)* inject default `instructions` for Codex Responses API passthrough

## [0.9.2](https://github.com/AprilNEA/BYOKEY/compare/v0.9.1...v0.9.2) - 2026-03-30

### Fixed

- *(ci)* remove empty appcast.xml when gh-pages branch does not exist

## [0.9.1](https://github.com/AprilNEA/BYOKEY/compare/v0.9.0...v0.9.1) - 2026-03-30

### Fixed

- *(ci)* use arm64 DMG only for Sparkle appcast generation

## [0.9.0](https://github.com/AprilNEA/BYOKEY/compare/v0.8.0...v0.9.0) - 2026-03-30

### Added

- *(proxy)* add file-watched in-memory index for Amp thread list
- *(desktop)* redesign app shell, add Sparkle updates and provider icons
- *(desktop)* redesign Amp page with model routing config
- *(desktop)* add APIClient for management endpoints and AnsiText view
- add --log-file CLI argument for persistent log output
- *(desktop)* enhanced ModelsView and UsageView
- *(desktop)* type-safe ConfigManager, AmpView injection status
- complete upstream v6.9.4 sync — all 9 remaining items

### Fixed

- *(deps)* update vulnerable dependencies and ignore unpatched rsa advisory
- *(desktop)* remove Dashboard scroll, enforce minimum window size
- switch tokio-tungstenite from native-tls to rustls, drop libssl-dev
- *(desktop)* equal-height stat cards, move log to bottom panel

### Other

- improve code quality across all crates
- *(desktop)* unify Desktop→Rust API to OpenAPI-generated client
- restore libssl-dev for test job linking
- remove unnecessary libssl-dev install from test job
- *(desktop)* split GeneralView into Dashboard sub-components
- *(desktop)* unified DataService, dynamic port, async CLI, restart banner
- install libssl-dev for test job linking

## [0.8.0](https://github.com/AprilNEA/BYOKEY/compare/v0.7.1...v0.8.0) - 2026-03-28

### Added

- *(usage)* add streaming token tracking, persistence, and time-series API

### Other

- *(desktop)* add certificate verification step for debugging

## [0.7.1](https://github.com/AprilNEA/BYOKEY/compare/v0.7.0...v0.7.1) - 2026-03-28

### Other

- *(desktop)* add macOS app build, sign, notarize and DMG release
- *(desktop)* switch from menu-bar-only to windowed app with menu bar extra
- use BYOKEY branding in user-visible text
- add Makefile for dev workflow
- *(desktop)* extract build phase scripts to desktop/scripts/
- *(desktop)* replace launchd daemon with menu bar app

## [0.7.0](https://github.com/AprilNEA/BYOKEY/compare/v0.6.0...v0.7.0) - 2026-03-27

### Fixed

- *(amp)* correct settings path to ~/.config/amp/settings.json

### Other

- *(store)* split sqlite.rs into persistent/ directory

## [0.6.0](https://github.com/AprilNEA/BYOKEY/compare/v0.5.3...v0.6.0) - 2026-03-06

### Added

- *(amp)* add --all flag to `amp ads disable`
- *(desktop)* isolate Debug and Release builds with separate Bundle IDs and ports
- *(proxy,desktop)* add account management API, rate limits, and Accounts UI
- *(desktop)* add management API, provider status UI, settings, and log viewer
- *(desktop)* replace Tauri with native Swift app embedding Rust daemon
- *(cli)* show server running status in byokey status

### Fixed

- *(ci)* move Stdio import into cfg(target_os = "macos") block
- *(desktop)* show log and timeout error when daemon is registered but not reachable
- *(desktop)* add argv[0] to LaunchAgent ProgramArguments ([#37](https://github.com/AprilNEA/BYOKEY/pull/37))
- *(desktop)* use LaunchAgent, add app icon, fix SMAppService registration

### Other

- *(amp)* restructure ads command as `amp ads disable/enable`
- split main.rs into serve, daemon, auth, amp modules
- gitignore Xcode xcuserdata and untrack xcuserstate
- update READMEs with current model names, CLI commands, and config format
- extract daemon management into byokey-daemon crate
- *(cli)* reduce duplication and improve ergonomics
- release v0.5.3 ([#26](https://github.com/AprilNEA/BYOKEY/pull/26))

## [0.5.3](https://github.com/AprilNEA/BYOKEY/compare/v0.5.2...v0.5.3) - 2026-02-28

### Other

- release v0.5.3 ([#25](https://github.com/AprilNEA/BYOKEY/pull/25))

## [0.5.2](https://github.com/AprilNEA/BYOKEY/compare/v0.5.1...v0.5.2) - 2026-02-26

### Fixed

- *(cli)* skip native binaries and re-sign after patching in amp disable-ads

## [0.5.1](https://github.com/AprilNEA/BYOKEY/compare/v0.5.0...v0.5.1) - 2026-02-26

### Added

- *(cli)* add `byokey amp` subcommand

### Other

- add Homebrew installation instructions

## [0.5.0](https://github.com/AprilNEA/BYOKEY/compare/v0.4.0...v0.5.0) - 2026-02-25

### Added

- observability — structured logging, usage stats, request tracing
- config enhancements — proxy_url, model alias/exclusion, payload rules, TLS, streaming config

### Other

- add AGENTS.md, CLAUDE.md and update .gitignore
- *(desktop)* rewrite frontend with React + Webpack + Tailwind + Base UI
- replace cross with native ubuntu-22.04-arm runner for aarch64

## [0.4.0](https://github.com/AprilNEA/BYOKEY/compare/v0.3.0...v0.4.0) - 2026-02-24

### Added

- *(auth)* implement OAuth token refresh via CDN credentials
- multi-account OAuth support per provider

### Other

- *(cli)* extract shared ServerArgs and DaemonArgs structs
- introduce tracing + fix config hot-reload via ArcSwap
- run update-homebrew even if some build targets fail

## [0.3.0](https://github.com/AprilNEA/BYOKEY/compare/v0.2.1...v0.3.0) - 2026-02-23

### Added

- *(proxy)* route Gemini native API through backend provider
- *(config)* default config path ~/.config/byokey/settings.json + JSON support

### Fixed

- align pre-commit clippy flags with CI and fix needless_raw_string_hashes

### Other

- *(desktop)* rewrite from GPUI to Tauri v2
- *(release-plz)* delete stale release-plz branches before running
- use app token for release-plz PR creation ([#12](https://github.com/AprilNEA/BYOKEY/pull/12))

## [0.2.1](https://github.com/AprilNEA/BYOKEY/compare/v0.2.0...v0.2.1) - 2026-02-22

### Added

- *(desktop)* add Info.plist with LSUIElement, separate CI job
- *(cli)* add start/stop/restart and autostart enable/disable/status

### Fixed

- *(main)* gate LAUNCHD_LABEL behind cfg(target_os = "macos")
- *(ci)* upgrade libclang to 7.x for aarch64 cross-compilation

### Other

- add pre-commit config with fmt, clippy, and conventional commit checks
- add From<rquest::Error>/From<sqlx::Error> for ByokError, eliminate manual .map_err

## [0.2.0](https://github.com/AprilNEA/BYOKEY/compare/v0.1.3...v0.2.0) - 2026-02-22

### Other

- guard packaging/upload steps behind release event, add homebrew-tap trigger

## [0.1.3](https://github.com/AprilNEA/BYOKEY/compare/v0.1.2...v0.1.3) - 2026-02-22

### Fixed

- *(ci)* add Cross.toml to install libclang for aarch64 cross-compilation

## [0.1.2](https://github.com/AprilNEA/BYOKEY/compare/v0.1.1...v0.1.2) - 2026-02-22

### Fixed

- *(ci)* use GitHub App token for release-plz to trigger build workflow

## [0.1.1](https://github.com/AprilNEA/BYOKEY/compare/v0.1.0...v0.1.1) - 2026-02-21

### Fixed

- *(release-plz)* use git_tag_name instead of tag_name_template
- *(release-plz)* use tag_name_template instead of invalid tag_name field

### Other

- add binary build workflow triggered on release
- beautify README with badges, provider logos, and sync CN version
- configure release-plz for single unified tag
- rename byok → byokey across codebase
- add release-plz workflow
