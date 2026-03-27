# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-auth-v0.6.0...byokey-auth-v0.7.0) - 2026-03-27

### Fixed

- *(auth)* replace Chinese error messages with English in callback.rs

### Other

- *(auth)* introduce AuthCodeFlow and DeviceCodeFlow traits
- *(auth)* reorganize directory structure into layered modules

## [0.6.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-auth-v0.5.3...byokey-auth-v0.6.0) - 2026-03-06

### Other

- *(auth)* unify all OAuth URLs via remote credentials and clean up stale refresh tokens
- *(auth)* fetch all OAuth client IDs from assets.byokey.io at runtime

## [0.5.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-auth-v0.4.0...byokey-auth-v0.5.0) - 2026-02-25

### Added

- observability — structured logging, usage stats, request tracing

## [0.4.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-auth-v0.3.0...byokey-auth-v0.4.0) - 2026-02-24

### Added

- *(auth)* implement OAuth token refresh via CDN credentials
- multi-account OAuth support per provider

## [0.2.1](https://github.com/AprilNEA/BYOKEY/compare/byokey-auth-v0.2.0...byokey-auth-v0.2.1) - 2026-02-22

### Fixed

- *(auth,proxy)* update Claude token URL and strip thinking on forced tool_choice

### Other

- add From<rquest::Error>/From<sqlx::Error> for ByokError, eliminate manual .map_err

## [0.1.1](https://github.com/AprilNEA/BYOKEY/compare/byokey-auth-v0.1.0...byokey-auth-v0.1.1) - 2026-02-21

### Added

- align Claude/Codex/Copilot providers with CLIProxyAPIPlus

### Other

- rename byok → byokey across codebase
