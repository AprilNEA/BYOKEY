# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-provider-v0.8.0...byokey-provider-v0.9.0) - 2026-03-28

### Added

- *(auth)* add proactive token refresh and concurrent refresh dedup
- complete upstream v6.9.4 sync — all 9 remaining items
- align Claude/Codex fingerprint and thinking with upstream v6.9.4

### Fixed

- *(proxy)* inject billing header for OAuth passthrough and clean up beta header

### Other

- deduplicate shared patterns across executors and handlers

## [0.8.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-provider-v0.7.1...byokey-provider-v0.8.0) - 2026-03-28

### Added

- *(usage)* add streaming token tracking, persistence, and time-series API

## [0.7.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-provider-v0.6.0...byokey-provider-v0.7.0) - 2026-03-27

### Added

- *(registry)* add gpt-5.4-nano model (Codex API-key only)
- *(registry)* add gpt-5.4-mini model (Codex + Copilot)
- *(provider)* unified model registry with auth-aware resolution

### Fixed

- *(registry)* reject empty tails in parse_qualified_model
- address review feedback — store test, models listing, registry docs

### Other

- *(provider)* reorganize into executor/ and factory modules
- *(auth)* reorganize directory structure into layered modules

## [0.6.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-provider-v0.5.3...byokey-provider-v0.6.0) - 2026-03-06

### Added

- *(proxy,desktop)* add account management API, rate limits, and Accounts UI
- *(desktop)* add management API, provider status UI, settings, and log viewer
- *(copilot)* quota-aware multi-account load balancing

## [0.5.3](https://github.com/AprilNEA/BYOKEY/compare/byokey-provider-v0.5.2...byokey-provider-v0.5.3) - 2026-02-28

### Added

- *(proxy)* route Anthropic messages through Copilot's native /v1/messages endpoint

## [0.4.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-provider-v0.3.0...byokey-provider-v0.4.0) - 2026-02-24

### Added

- *(auth)* implement OAuth token refresh via CDN credentials
- multi-account OAuth support per provider
- *(config)* multi-API-key configuration and credential routing

### Other

- *(provider)* structured errors, shared HTTP client, Kimi executor, thinking suffix
- introduce tracing + fix config hot-reload via ArcSwap

## [0.3.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-provider-v0.2.1...byokey-provider-v0.3.0) - 2026-02-23

### Added

- *(provider)* Copilot model refresh + Gemini→Copilot backend/fallback

## [0.2.1](https://github.com/AprilNEA/BYOKEY/compare/byokey-provider-v0.2.0...byokey-provider-v0.2.1) - 2026-02-22

### Other

- add pre-commit config with fmt, clippy, and conventional commit checks
- add From<rquest::Error>/From<sqlx::Error> for ByokError, eliminate manual .map_err

## [0.1.1](https://github.com/AprilNEA/BYOKEY/compare/byokey-provider-v0.1.0...byokey-provider-v0.1.1) - 2026-02-21

### Added

- tool calling, prompt caching, error codes, reasoning, adjacent message merging
- *(codex)* support reasoning models (o4-mini, o3)
- align Claude/Codex/Copilot providers with CLIProxyAPIPlus

### Fixed

- resolve all clippy and format warnings across workspace
- *(registry)* remove gpt-4o/gpt-4o-mini from Codex, route to Copilot
- *(claude)* translate SSE stream to OpenAI format
