# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1](https://github.com/AprilNEA/BYOKEY/compare/byokey-auth-v0.3.0...byokey-auth-v0.3.1) - 2026-02-24

### Added

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
