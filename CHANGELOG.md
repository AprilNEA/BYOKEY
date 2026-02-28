# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
