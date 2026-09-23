<h4 align="right"><strong>English</strong> | <a href="./docs/README_CN.md">简体中文</a></h4>

<p align="center">
    <img src="./docs/public/icon.png" width=138/>
</p>

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
[![rust](https://img.shields.io/badge/rust-1.98+-444?style=flat-square&labelColor=000&logo=rust&logoColor=fff)](https://www.rust-lang.org)

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
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/claude.svg" width="36" alt="Claude"><br>
      <b>Claude</b><br>
      <kbd>OAuth</kbd><br>
      <sub>claude-opus-4-6<br>claude-sonnet-4-5<br>claude-haiku-4-5</sub>
    </td>
    <td align="center" width="200" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.byokey.io/icons/providers/codex-dark.svg">
        <img src="https://assets.byokey.io/icons/providers/codex.svg" width="36" alt="Codex">
      </picture><br>
      <b>Codex</b><br>
      <kbd>OAuth</kbd><br>
      <sub>gpt-5.4<br>gpt-5.3-codex<br>gpt-5.1-codex-max<br>o3 · o4-mini</sub>
    </td>
    <td align="center" width="200" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.byokey.io/icons/providers/copilot-dark.svg">
        <img src="https://assets.byokey.io/icons/providers/githubcopilot.svg" width="36" alt="GitHub Copilot">
      </picture><br>
      <b>Copilot</b><br>
      <kbd>Device code</kbd><br>
      <sub>gpt-5.4<br>claude-sonnet-4.6<br>gemini-3.1-pro<br>grok-code-fast-1</sub>
    </td>
  </tr>
  <tr>
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/gemini.svg" width="36" alt="Gemini"><br>
      <b>Gemini</b><br>
      <kbd>OAuth</kbd><br>
      <sub>gemini-2.0-flash<br>gemini-1.5-pro<br>gemini-1.5-flash</sub>
    </td>
    <td align="center" width="200" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.byokey.io/icons/providers/amazonwebservices-dark.svg">
        <img src="https://assets.byokey.io/icons/providers/amazonwebservices.svg" width="36" alt="AWS">
      </picture><br>
      <b>Kiro</b><br>
      <kbd>Device code</kbd><br>
      <sub>kiro-default</sub>
    </td>
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/gemini.svg" width="36" alt="Antigravity"><br>
      <b>Antigravity</b><br>
      <kbd>OAuth</kbd><br>
      <sub>ag-gemini-2.5-pro<br>ag-gemini-2.5-flash<br>ag-claude-sonnet-4-5</sub>
    </td>
  </tr>
  <tr>
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/alibabacloud.svg" width="36" alt="Qwen"><br>
      <b>Qwen</b><br>
      <kbd>Device code</kbd><br>
      <sub>qwen3-max<br>qwen3-coder-plus<br>qwen-plus</sub>
    </td>
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/kimi.svg" width="36" alt="Kimi"><br>
      <b>Kimi</b><br>
      <kbd>Device code</kbd><br>
      <sub>kimi-k2-0711</sub>
    </td>
    <td align="center" width="200" valign="top">
      <b>iFlow</b><br>
      <kbd>OAuth</kbd><br>
      <sub>glm-4.5<br>glm-z1-flash<br>kimi-k2</sub>
    </td>
  </tr>
</table>

## Installation

**Homebrew (macOS / Linux)**

```sh
brew install AprilNEA/tap/byokey
```

**Install script (Linux / macOS)**

```sh
curl -fsSL https://raw.githubusercontent.com/AprilNEA/BYOKEY/master/install.sh | sh
```

Downloads the latest release binary into `~/.byokey/bin/`. Pin a version with `BYOKEY_VERSION=v1.2.0` or override the install location with `BYOKEY_INSTALL_DIR=/usr/local/bin`.

**From crates.io**

```sh
cargo install byokey
```

**From source**

```sh
git clone https://github.com/AprilNEA/BYOKEY
cd BYOKEY
cargo install --path .
```

> **Requirements:** Rust 1.98+ (edition 2024), a C compiler for SQLite, and `protoc` for ConnectRPC code generation (`brew install protobuf`, `apt-get install protobuf-compiler`, or `choco install protoc`).

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

**For Claude Code with GitHub Copilot:**

```sh
byokey login copilot
byokey claude-code inject --backend copilot
byokey start
```

If BYOKEY is already running, use the restart command printed by `inject`,
with the same `--config` path. Then restart Claude Code and select a Claude
model available to your Copilot account with `/model` or `claude --model <MODEL_ID>`.
The command preserves your existing model choices. `--backend copilot` changes
BYOKEY's global Claude routing, including requests from other clients.

Claude Code needs the proxy's root URL, such as `http://127.0.0.1:8018`,
without `/v1`; it appends `/v1/messages` itself.

**For Amp:**

`byokey serve` spins up a second listener on port `18018` (configurable via
`amp.port`) dedicated to the Amp-compatible router. Point the Amp CLI at it:

```jsonc
// ~/.config/amp/settings.json
{
  "amp.url": "http://localhost:18018"
}
```

Or let byokey write it for you: `byokey amp inject`.

## CLI Reference

```
byokey <COMMAND>

Commands:
  serve         Start the proxy server (foreground)
  start         Start the proxy server in the background
  stop          Stop the background proxy server
  restart       Restart the background proxy server
  reload        Reload the running server's configuration without restarting
  service       Manage OS-level service registration (launchd / systemd / Windows SCM)
  login         Authenticate with a provider
  logout        Remove stored credentials for a provider
  status        Show authentication status for all providers
  tui           Launch the interactive terminal UI
  accounts      List all accounts for a provider
  switch        Switch the active account for a provider
  claude-code   Claude Code configuration utilities
  amp           Amp-related utilities
  openapi       Export the OpenAPI specification as JSON
  completions   Generate shell completions
  help          Print help
```

<details>
<summary><b>Command details</b></summary>
<br>

**`byokey serve`**

```
Options:
  -c, --config <FILE>   Config file (JSON or YAML) [default: ~/.config/byokey/settings.json]
  -p, --port <PORT>     Listen port     [default: 8018]
      --host <HOST>     Listen address  [default: 127.0.0.1]
      --db <PATH>       SQLite DB path  [default: ~/.byokey/tokens.db]
      --log-file <PATH> Log file with daily rotation (default: stdout)
```

`serve` also opens a second HTTP listener on `amp.port` (default `18018`) for
the Amp-compatible router, and binds a Unix control socket at
`~/.byokey/control.sock` used by `stop` / `reload`. If the process is launched
with a pre-opened socket via `systemfd`, `systemd`, or `launchd`, the inherited
fd is adopted in place of a fresh bind.

**`byokey start`** — Same options as `serve`. Runs the server in the background
and writes logs to `~/.byokey/server.log` by default.

**`byokey reload`** — Triggers a hot config reload on the running server via
the control socket. No process restart, no dropped connections.

**`byokey login <PROVIDER>`**

Runs the appropriate OAuth flow for the given provider.
Supported names: `claude`, `codex`, `copilot`, `gemini`, `kiro`,
`antigravity`, `qwen`, `kimi`, `iflow`.

```
Options:
      --account <NAME>  Account identifier (default: `default`)
      --db <PATH>       SQLite DB path [default: ~/.byokey/tokens.db]
```

**`byokey logout <PROVIDER>`** — Deletes the stored token for the given provider.

**`byokey status`** — Prints authentication status for every known provider.

**`byokey tui`** — Opens the terminal management UI. It connects to the
ConnectRPC management API at `http://127.0.0.1:8018` by default; override with
`--url <URL>`.

**`byokey accounts <PROVIDER>`** — Lists all accounts for a provider.

**`byokey switch <PROVIDER> <ACCOUNT>`** — Switches the active account for a provider.

**`byokey service <install|uninstall|start|stop|status>`** — Registers byokey
as an OS-managed service. Uses `launchd` on macOS, `systemd` on Linux, and
Windows SCM on Windows.

**`byokey claude-code inject`** — Configures Claude Code to use BYOKEY.

```sh
byokey claude-code inject [--config FILE] [--settings FILE] [--url URL] \
  [--backend copilot] [--disable-experimental-betas]
```

- `--config`: BYOKEY configuration file (JSON or YAML).
- `--settings`: Claude Code settings file. Defaults to
  `$CLAUDE_CONFIG_DIR/settings.json` when set, otherwise `~/.claude/settings.json`.
- `--url`: Override the proxy root URL. Otherwise, use
  `claude_code.settings.env.ANTHROPIC_BASE_URL` if configured, then fall back
  to BYOKEY's host and port. Do not append `/v1`.
- `--backend copilot`: Set `providers.claude.backend` to `copilot` in the
  BYOKEY configuration. Cannot be combined with `--url`. Other configuration
  values are preserved; rewriting YAML normalizes formatting and removes comments.
- `--disable-experimental-betas`: Set
  `CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS=1` for gateway compatibility. This is
  automatic when BYOKEY's configured Claude backend is Copilot; use the flag
  explicitly for a remote gateway that needs it.

Injection merges extras from `claude_code.settings` in the BYOKEY configuration,
then writes the resolved `env.ANTHROPIC_BASE_URL` and a placeholder
`env.ANTHROPIC_API_KEY`. These generated values take precedence over extras.
Existing environment entries are merged key by key; unrelated settings and
model choices are preserved unless explicitly set in those extras. Malformed
Claude Code settings are rejected instead of overwritten.
The command prints the required server restart command; it does not run it.
Restart Claude Code after injection.

**`byokey amp inject`** — Writes `amp.url` (and any extras from
`amp.settings` in your byokey config) into `~/.config/amp/settings.json`.

</details>

## Configuration

Create a config file (JSON or YAML, e.g. `~/.config/byokey/settings.json`) and pass it with `--config`:

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

Optional Claude Code settings to merge when running `byokey claude-code inject`:

```yaml
claude_code:
  settings:
    env:
      CLAUDE_CODE_EFFORT_LEVEL: high
```

Injection does not select or pin a model automatically. Use model IDs supported
by the provider handling your requests; available features depend on that provider.

With `providers.claude.backend: copilot`, `/v1/messages` automatically converts
recognizable Claude 4+ Opus, Sonnet, Haiku, and Fable IDs to Copilot spelling,
for example `claude-opus-5-5` → `claude-opus-5.5`. Dated snapshot IDs such as
`claude-haiku-4-5-20251001` select the undated same family/version,
`claude-haiku-4.5`. The `-fast` variant is retained; already dotted IDs and
unknown/custom names are unchanged. This does not check model availability or
downgrade versions, and does not change direct Anthropic requests.

Explicit `model_alias.copilot` entries take precedence and send `name` verbatim:

```yaml
model_alias:
  copilot:
    - alias: claude-opus-5-5
      name: claude-opus-5.5
```

Set `alias` and `name` to the same ID to opt that ID out of automatic conversion.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for build commands, architecture details, and coding guidelines.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
