# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
