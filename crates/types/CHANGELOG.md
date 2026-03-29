# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-types-v0.8.0...byokey-types-v0.9.0) - 2026-03-29

### Added

- *(auth)* add proactive token refresh and concurrent refresh dedup
- complete upstream v6.9.4 sync — all 9 remaining items

### Other

- improve code quality across all crates
- *(desktop)* unify Desktop→Rust API to OpenAPI-generated client

## [0.8.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-types-v0.7.1...byokey-types-v0.8.0) - 2026-03-28

### Added

- *(usage)* add streaming token tracking, persistence, and time-series API

## [0.7.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-types-v0.6.0...byokey-types-v0.7.0) - 2026-03-27

### Other

- *(store)* split sqlite.rs into persistent/ directory

## [0.6.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-types-v0.5.3...byokey-types-v0.6.0) - 2026-03-06

### Added

- *(proxy,desktop)* add account management API, rate limits, and Accounts UI
- *(desktop)* add management API, provider status UI, settings, and log viewer

## [0.4.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-types-v0.3.0...byokey-types-v0.4.0) - 2026-02-24

### Added

- *(auth)* implement OAuth token refresh via CDN credentials
- multi-account OAuth support per provider

### Other

- *(provider)* structured errors, shared HTTP client, Kimi executor, thinking suffix

## [0.2.1](https://github.com/AprilNEA/BYOKEY/compare/byokey-types-v0.2.0...byokey-types-v0.2.1) - 2026-02-22

### Other

- add From<rquest::Error>/From<sqlx::Error> for ByokError, eliminate manual .map_err

## [0.1.1](https://github.com/AprilNEA/BYOKEY/compare/byokey-types-v0.1.0...byokey-types-v0.1.1) - 2026-02-21

### Fixed

- resolve all clippy and format warnings across workspace

### Other

- rename byok → byokey across codebase
- add README_CN and translate CONTRIBUTING to English
