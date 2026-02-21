# BYOKEY — Bring Your Own Keys

A local proxy that turns AI **subscriptions** into **OpenAI-compatible API endpoints**,
so any OpenAI-compatible tool (Amp, Cursor, Continue, etc.) can use them without a paid API key.

```
Claude Pro  ─┐
OpenAI Plus ─┤  byokey serve  ─►  http://localhost:8018/v1/chat/completions
Copilot     ─┘                  (OpenAI-compatible, streaming)
```

## Features

- **OpenAI-compatible API** — drop-in replacement, just change the base URL
- **OAuth login flows** — PKCE, device code, and auth-code flows handled for you
- **SQLite token store** — tokens survive restarts; stored in `~/.byokey/tokens.db`
- **API key passthrough** — prefer raw API keys? Set them in the config file
- **Amp CLI compatibility** — `/amp/*` routes for [Amp](https://ampcode.com) out of the box
- **YAML config** — hot-reload friendly, all options have sensible defaults

## Supported Providers

| Provider | Login flow | Models |
|---|---|---|
| **Claude** (Anthropic) | PKCE browser | claude-opus-4-6, claude-sonnet-4-5, … |
| **Codex** (OpenAI) | PKCE browser | o4-mini, o3, gpt-4o, gpt-4o-mini |
| **Copilot** (GitHub) | Device code | gpt-4o, claude-3.5-sonnet, o3-mini |
| **Gemini** (Google) | PKCE browser | gemini-2.0-flash, gemini-1.5-pro, … |
| **Kiro** (AWS) | Device code | kiro-default |
| **Antigravity** (Google) | PKCE browser | — *(auth ready, executor WIP)* |
| **Qwen** (Alibaba) | Device code + PKCE | — *(auth ready, executor WIP)* |
| **Kimi** (Moonshot) | Device code | — *(auth ready, executor WIP)* |
| **iFlow** (Z.ai / GLM) | Auth code | — *(auth ready, executor WIP)* |

## Installation

### From crates.io

```sh
cargo install byokey
```

### From source

```sh
git clone https://github.com/AprilNEA/BYOKEY
cd BYOK
cargo install --path .
```

**Requirements:** Rust 1.85+ (edition 2024), a C compiler for SQLite.

## Quick Start

```sh
# 1. Authenticate (opens browser or shows a device code)
byokey login claude
byokey login codex
byokey login copilot

# 2. Start the proxy
byokey serve

# 3. Point your tool at it
export OPENAI_BASE_URL=http://localhost:8018/v1
export OPENAI_API_KEY=any          # byokey ignores the key value
```

For Amp:

```jsonc
// ~/.amp/settings.json  (or wherever Amp reads config)
{
  "amp.url": "http://localhost:8018/amp"
}
```

## CLI Reference

```
byokey <COMMAND>

Commands:
  serve    Start the proxy server
  login    Authenticate with a provider
  logout   Remove stored credentials for a provider
  status   Show authentication status for all providers
  help     Print help
```

### `byokey serve`

```
Options:
  -c, --config <FILE>   YAML config file [default: none]
  -p, --port <PORT>     Listen port     [default: 8018]
      --host <HOST>     Listen address  [default: 127.0.0.1]
      --db <PATH>       SQLite DB path  [default: ~/.byokey/tokens.db]
```

### `byokey login <PROVIDER>`

Runs the appropriate OAuth flow for the given provider.
Supported names: `claude`, `codex`, `copilot`, `gemini`, `kiro`,
`antigravity`, `qwen`, `kimi`, `iflow`.

```
Options:
      --db <PATH>   SQLite DB path [default: ~/.byokey/tokens.db]
```

### `byokey logout <PROVIDER>`

Deletes the stored token for the given provider.

### `byokey status`

Prints authentication status for every known provider.

## Configuration

Create a YAML file (e.g. `~/.byokey/config.yaml`) and pass it with `--config`:

```yaml
# ~/.byokey/config.yaml
port: 8018
host: 127.0.0.1

providers:
  # Use a raw API key (takes precedence over OAuth)
  claude:
    api_key: "sk-ant-..."

  # Disable a provider entirely
  gemini:
    enabled: false

  # OAuth-only (no api_key) — use `byokey login codex` first
  codex:
    enabled: true
```

All fields are optional; unspecified providers are enabled by default and use
the OAuth token stored in the database.

## API Endpoints

| Method | Path | Description |
|---|---|---|
| `POST` | `/v1/chat/completions` | OpenAI-compatible chat (streaming supported) |
| `GET` | `/v1/models` | List enabled models |
| `GET` | `/amp/v1/login` | Amp login redirect |
| `ANY` | `/amp/v0/management/{*path}` | Amp management API proxy |
| `POST` | `/amp/v1/chat/completions` | Amp-compatible chat |

The `model` field in the request body determines which provider is used.

## How It Works

```
Client request
    │
    ▼
byok-proxy  (axum HTTP server)
    │  resolve model → provider
    ▼
byok-provider  (executor per provider)
    │  get OAuth token (or api_key)
    ▼
byok-auth  (AuthManager + OAuth flows)
    │
    ▼
Upstream API  (Anthropic / OpenAI / Google / …)
    │  translate response → OpenAI format
    ▼
Client response  (JSON or SSE stream)
```

### Workspace Crates

| Crate | Description |
|---|---|
| `byokey-types` | Shared types, traits, errors |
| `byokey-config` | YAML configuration + file watching |
| `byokey-store` | SQLite (and in-memory) token persistence |
| `byokey-auth` | OAuth 2.0 login flows for every provider |
| `byokey-translate` | Request/response format translation |
| `byokey-provider` | Provider executors and model registry |
| `byokey-proxy` | axum HTTP server and routing |

## Building & Testing

```sh
# Build everything
cargo build --workspace

# Run all tests (173 tests, no network required)
cargo test --workspace

# Lint
cargo clippy --workspace --all-targets -- -D warnings

# Format
cargo fmt --all
```

## License

Licensed under either of:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option.
