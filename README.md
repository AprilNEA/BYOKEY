<div align="center">

# BYOKEY

**Bring Your Own Keys**<br>
Turn AI subscriptions into standard API endpoints.<br>
Expose any provider as OpenAI- or Anthropic-compatible API — locally or in the cloud.

[![ci](https://img.shields.io/github/actions/workflow/status/AprilNEA/BYOKEY/ci.yml?style=flat-square&labelColor=000&color=444&label=ci)](https://github.com/AprilNEA/BYOKEY/actions/workflows/ci.yml)
&nbsp;
[![crates.io](https://img.shields.io/crates/v/byokey?style=flat-square&labelColor=000&color=444)](https://crates.io/crates/byokey)
&nbsp;
[![license](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-444?style=flat-square&labelColor=000)](LICENSE-MIT)
&nbsp;
[![rust](https://img.shields.io/badge/rust-1.85+-444?style=flat-square&labelColor=000&logo=rust&logoColor=fff)](https://www.rust-lang.org)

</div>

```
Subscriptions                                     Tools

Claude Pro  ─┐                              ┌──  Amp Code
OpenAI Plus ─┼──  byokey serve  ────────────┼──  Cursor · Windsurf
Copilot     ─┘                              ├──  Factory CLI (Droid)
                                            └──  any OpenAI / Anthropic client
```

## Features

- **Multi-format API** — OpenAI and Anthropic compatible endpoints; just change the base URL
- **OAuth login flows** — PKCE, device-code, and auth-code flows handled automatically
- **Token persistence** — SQLite at `~/.byokey/tokens.db`; survives restarts
- **API key passthrough** — Set raw keys in config to skip OAuth entirely
- **Deploy anywhere** — Run locally as a CLI, or deploy as a shared AI gateway
- **Agent-ready** — Native support for [Amp Code](https://ampcode.com); [Factory CLI (Droid)](https://factory.ai) coming soon
- **Hot-reload config** — YAML-based with sensible defaults

## Supported Providers

<table>
  <tr>
    <td align="center" width="120">
      <img src="https://assets.byokey.io/icons/providers/anthropic.svg" width="36" alt="Anthropic"><br>
      <b>Claude</b><br>
      <sup>PKCE</sup><br>
      <sub>opus-4 · sonnet-4.5</sub>
    </td>
    <td align="center" width="120">
      <img src="https://assets.byokey.io/icons/providers/openai.svg" width="36" alt="OpenAI"><br>
      <b>Codex</b><br>
      <sup>PKCE</sup><br>
      <sub>o4-mini · o3 · gpt-4o</sub>
    </td>
    <td align="center" width="120">
      <img src="https://assets.byokey.io/icons/providers/githubcopilot.svg" width="36" alt="GitHub Copilot"><br>
      <b>Copilot</b><br>
      <sup>Device code</sup><br>
      <sub>gpt-4o · claude-3.5 · o3-mini</sub>
    </td>
    <td align="center" width="120">
      <img src="https://assets.byokey.io/icons/providers/googlegemini.svg" width="36" alt="Google Gemini"><br>
      <b>Gemini</b><br>
      <sup>PKCE</sup><br>
      <sub>2.0-flash · 1.5-pro</sub>
    </td>
    <td align="center" width="120">
      <img src="https://assets.byokey.io/icons/providers/amazonwebservices.svg" width="36" alt="AWS"><br>
      <b>Kiro</b><br>
      <sup>Device code</sup><br>
      <sub>kiro-default</sub>
    </td>
  </tr>
</table>

> **Coming soon** — auth implemented, executor in progress:<br>
> Antigravity (Google) · Qwen (Alibaba) · Kimi (Moonshot) · iFlow (Z.ai)

## Installation

**Homebrew (macOS / Linux)**

```sh
brew install AprilNEA/tap/byokey
```

**From crates.io**

```sh
cargo install byokey
```

**From source**

```sh
git clone https://github.com/AprilNEA/BYOKEY
cd BYOK
cargo install --path .
```

> **Requirements:** Rust 1.85+ (edition 2024), a C compiler for SQLite.

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

**For Amp:**

```jsonc
// ~/.amp/settings.json
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

<details>
<summary><b>Command details</b></summary>
<br>

**`byokey serve`**

```
Options:
  -c, --config <FILE>   YAML config file [default: none]
  -p, --port <PORT>     Listen port     [default: 8018]
      --host <HOST>     Listen address  [default: 127.0.0.1]
      --db <PATH>       SQLite DB path  [default: ~/.byokey/tokens.db]
```

**`byokey login <PROVIDER>`**

Runs the appropriate OAuth flow for the given provider.
Supported names: `claude`, `codex`, `copilot`, `gemini`, `kiro`,
`antigravity`, `qwen`, `kimi`, `iflow`.

```
Options:
      --db <PATH>   SQLite DB path [default: ~/.byokey/tokens.db]
```

**`byokey logout <PROVIDER>`** — Deletes the stored token for the given provider.

**`byokey status`** — Prints authentication status for every known provider.

</details>

## Configuration

Create a YAML file (e.g. `~/.byokey/config.yaml`) and pass it with `--config`:

```yaml
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

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for build commands, architecture details, and coding guidelines.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
