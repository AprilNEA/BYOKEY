# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.1](https://github.com/AprilNEA/BYOKEY/compare/v1.0.0...v1.0.1) - 2026-04-17

### Fixed

- *(release-plz)* suppress individual tags for byokey-proto and ampcode

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
