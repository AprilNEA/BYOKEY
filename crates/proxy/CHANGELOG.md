# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-proxy-v0.4.0...byokey-proxy-v0.5.0) - 2026-02-25

### Added

- observability — structured logging, usage stats, request tracing
- config enhancements — proxy_url, model alias/exclusion, payload rules, TLS, streaming config
- *(amp)* add hide_free_tier option to suppress free-tier ads

### Fixed

- *(amp)* remove Content-Length header after rewriting free-tier response

## [0.4.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-proxy-v0.3.0...byokey-proxy-v0.4.0) - 2026-02-24

### Added

- *(auth)* implement OAuth token refresh via CDN credentials
- multi-account OAuth support per provider

### Other

- *(provider)* structured errors, shared HTTP client, Kimi executor, thinking suffix
- introduce tracing + fix config hot-reload via ArcSwap

## [0.3.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-proxy-v0.2.1...byokey-proxy-v0.3.0) - 2026-02-23

### Added

- *(proxy)* route Gemini native API through backend provider
- *(provider)* Copilot model refresh + Gemini→Copilot backend/fallback

## [0.2.1](https://github.com/AprilNEA/BYOKEY/compare/byokey-proxy-v0.2.0...byokey-proxy-v0.2.1) - 2026-02-22

### Added

- *(proxy)* forward query params and add debug logging for /api/internal
- *(proxy)* add /amp/auth/cli-login route for Amp CLI login flow

### Fixed

- *(lint)* resolve clippy warnings and format issues
- *(proxy)* strip thinking field when type is "auto" or tool_choice is forced
- *(auth,proxy)* update Claude token URL and strip thinking on forced tool_choice
- *(proxy)* align Anthropic request headers with reference implementation
- *(proxy)* strip /amp prefix when redirecting to ampcode.com auth

### Other

- add From<rquest::Error>/From<sqlx::Error> for ByokError, eliminate manual .map_err

## [0.2.0](https://github.com/AprilNEA/BYOKEY/compare/byokey-proxy-v0.1.3...byokey-proxy-v0.2.0) - 2026-02-22

### Added

- *(proxy)* add AmpCode shared-proxy mode via amp.upstream_key

## [0.1.1](https://github.com/AprilNEA/BYOKEY/compare/byokey-proxy-v0.1.0...byokey-proxy-v0.1.1) - 2026-02-21

### Added

- tool calling, prompt caching, error codes, reasoning, adjacent message merging
- *(proxy)* add POST /v1/messages Anthropic native passthrough

### Fixed

- resolve all clippy and format warnings across workspace

### Other

- add README_CN and translate CONTRIBUTING to English
