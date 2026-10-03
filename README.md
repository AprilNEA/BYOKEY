<h4 align="right"><strong>English</strong> | <a href="./docs/README_CN.md">简体中文</a></h4>

<p align="center">
    <img src="./docs/public/icon.png" width=138/>
</p>

<div align="center">

# BYOKEY

**Bring Your Own Keys**<br>
Run ChatGPT.app / Codex and Claude clients through a local gateway.<br>
Responses supports your ChatGPT subscription, GitHub Copilot and custom upstreams; Anthropic Messages supports Copilot, Cursor and Claude.

[![ci](https://img.shields.io/github/actions/workflow/status/AprilNEA/BYOKEY/ci.yml?style=flat-square&labelColor=000&color=444&label=ci)](https://github.com/AprilNEA/BYOKEY/actions/workflows/ci.yml)
&nbsp;
[![crates.io](https://img.shields.io/crates/v/byokey?style=flat-square&labelColor=000&color=444)](https://crates.io/crates/byokey)
&nbsp;
[![license](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-444?style=flat-square&labelColor=000)](LICENSE-MIT)
&nbsp;
[![rust](https://img.shields.io/badge/rust-1.91+-444?style=flat-square&labelColor=000&logo=rust&logoColor=fff)](https://www.rust-lang.org)

</div>

> [!IMPORTANT]
> **The published Anthropic-only release was archived on 2026-09-28.** The Responses gateway described below is a source-build feature on this branch, not a published release.
>
> GitHub Copilot's `/v1/messages` endpoint now accepts Claude Code's requests as they are, so Claude Code no longer needs a gateway to run on Copilot. The Cursor backend drives Cursor's private agent protocol, which Cursor does not allow outside its own clients, and it does not hold up in Claude Code's tool loops.
>
> To use Copilot directly (GitHub does not document this endpoint), sign in with the GitHub CLI and configure:
>
> **Claude Code** — `~/.claude/settings.json`
>
> ```json
> {
>   "env": { "ANTHROPIC_BASE_URL": "https://api.githubcopilot.com" },
>   "apiKeyHelper": "gh auth token"
> }
> ```
>
> **Claude Desktop** — third-party inference with the gateway provider. Copilot serves no `/v1/models`, so list the models by full id:
>
> ```json
> {
>   "inferenceProvider": "gateway",
>   "inferenceGatewayBaseUrl": "https://api.githubcopilot.com",
>   "inferenceGatewayAuthScheme": "bearer",
>   "inferenceCredentialKind": "helper-script",
>   "inferenceCredentialHelper": "/opt/homebrew/bin/gh",
>   "inferenceCredentialHelperArgs": ["auth", "token"],
>   "modelDiscoveryEnabled": false,
>   "inferenceModels": ["claude-opus-5-5", "claude-fable-5-1", "claude-sonnet-5", "claude-haiku-4-5"]
> }
> ```

```
Subscriptions                                     Clients

GitHub Copilot ─┐                            ┌──  Claude Code
Cursor         ─┼──  byokey serve  ──────────┼──  Claude Desktop
Claude Pro/Max ─┘                            └──  any Anthropic Messages client
```

## Features

- **Anthropic Messages API** — `/v1/messages`, `/v1/messages/count_tokens` and `/v1/models`, as Claude Code and Claude Desktop expect them; `[1m]` long-context ids included
- **Copilot as a Claude backend** — Copilot's Anthropic-format endpoint, with quota-aware rotation across accounts, a cheaper model for Claude Code's incidental calls, and the request fields Copilot rejects stripped
- **Cursor as a Claude backend** — every model on your Cursor plan, driven through Cursor's agent protocol
- **Claude Code and Claude Desktop wiring** — `byokey claude start`, `byokey claude inject`, `byokey claude desktop`; `byokey doctor` checks it all
- **OAuth login and token persistence** — device-code and PKCE flows; SQLite at `~/.byokey/tokens.db`, tokens refreshed in the background
- **Runs as a service** — launchd / systemd / Windows SCM registration, hot-reloaded config

## Supported Providers

<table>
  <tr>
    <td align="center" width="200" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.byokey.io/icons/providers/copilot-dark.svg">
        <img src="https://assets.byokey.io/icons/providers/githubcopilot.svg" width="36" alt="GitHub Copilot">
      </picture><br>
      <b>Copilot</b><br>
      <kbd>Device code</kbd><br>
      <sub>claude-fable-5.1<br>claude-opus-5.5<br>claude-sonnet-5<br>…every Claude model on your plan</sub>
    </td>
    <td align="center" width="200" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.byokey.io/icons/providers/cursor-dark.svg">
        <img src="https://assets.byokey.io/icons/providers/cursor.svg" width="36" alt="Cursor">
      </picture><br>
      <b>Cursor</b><br>
      <kbd>OAuth · API key</kbd><br>
      <sub>claude-opus-5-5<br>claude-opus-5-5-low-fast<br>composer-2.5<br>…every model on your plan</sub>
    </td>
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/claude.svg" width="36" alt="Claude"><br>
      <b>Claude</b><br>
      <kbd>OAuth · API key</kbd><br>
      <sub>claude-fable-5-1<br>claude-opus-5-5<br>claude-sonnet-5<br>claude-haiku-4-5</sub>
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

> **Requirements:** Rust 1.91+ (edition 2024), a C compiler for SQLite, and `protoc` for ConnectRPC code generation (`brew install protobuf`, `apt-get install protobuf-compiler`, or `choco install protoc`).

## Quick Start

### ChatGPT.app / Codex

This connects the Codex functionality in the ChatGPT desktop app, not the legacy ChatGPT conversation API. Keep the client signed in to ChatGPT: the client supplies its access token and account header and remains responsible for token refresh. BYOKEY does not import or store that login.

Build and start this checkout:

```sh
devenv shell cargo run -- serve
```

Merge these settings into `~/.codex/config.toml`, keeping top-level keys before any TOML table, then restart the client:

```toml
model = "gpt-6-sol"
model_provider = "byokey"
web_search = "disabled"

[model_providers.byokey]
name = "BYOKEY"
base_url = "http://127.0.0.1:8018/codex"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = false
model_catalog_url = "http://127.0.0.1:8018/codex/models"

[features]
enable_request_compression = false
```

Use a model available to your account. `model_catalog_url` is the full catalog URL, not the API root. This setup uses HTTP SSE and local compaction; WebSocket transport and compressed request bodies are not supported. Keep web search disabled until the selected upstream supports the client's search tools.

With no Responses configuration, all requests go to ChatGPT. To add Copilot and a custom gateway, sign in with `byokey login copilot` and merge the following into the BYOKEY config passed to `serve --config`:

```yaml
responses:
  default: chatgpt
  models:
    copilot-fast:
      upstream: copilot
      model: gpt-5.4-mini
      catalog_model: gpt-5.4-mini
    company-fast:
      upstream: company
      model: my-deployment
      catalog_model: gpt-5.4-mini
  upstreams:
    company:
      base_url: https://gateway.example.com/team/v1
      api_key: { env: COMPANY_API_KEY }
      headers:
        X-Tenant: engineering
        X-Special-Token: { env: COMPANY_GATEWAY_TOKEN }
```

`company-fast` assumes that `my-deployment` serves the same model as `catalog_model`; choose matching metadata for the actual deployment. Export the referenced variables in the **BYOKEY server process**, not just the client. Missing variables fail the request. A configured `Authorization` header overrides `api_key`. `/responses` is appended to each `base_url`; custom upstreams must implement the Responses API themselves.

Select `copilot-fast` or `company-fast` in the client. Exact aliases take priority, followed by `chatgpt/<model>`, `copilot/<model>` or `<upstream>/<model>`, followed by `responses.default` for unqualified names. Copilot models must advertise `/responses`; BYOKEY does not translate Chat Completions or Anthropic requests on this path. Copilot uses BYOKEY's stored accounts or `providers.copilot.api_key`, never the client's ChatGPT credential.

The catalog borrows actual ChatGPT model metadata, including instructions and capabilities. When `model_messages.instructions_template` is present, BYOKEY omits the legacy `base_instructions` copy to reduce catalog size: Codex limits remote custom catalogs to 1 MiB. An alias needs a matching `catalog_model`, or a complete Codex ModelInfo object under `catalog`. A missing match is an error. A custom default with complete catalog objects needs no ChatGPT catalog access; otherwise catalog discovery needs the client's ChatGPT login. Alias upgrades are disabled so the client does not migrate an alias to a different route. The existing `/v1/models`, `byokey route`, and TUI route list remain Anthropic-only.

Send `Reply with exactly: gateway-ok` and check the gateway log for the intended upstream and model. Then test a tool call and a follow-up. Upstream errors and `retry-after` are preserved. Normal Responses streams end with `response.completed`; failed, incomplete and truncated streams are recorded as failures, while client cancellation is recorded as `abandoned`.

Keep the listener on `127.0.0.1`. BYOKEY has no inbound authentication for its stored Copilot/custom credentials; do not expose this port to an untrusted network. Only the configured ChatGPT backend receives the client's ChatGPT auth headers. Redirects are not followed. Responses routes are excluded from `BYOKEY_DUMP`, and their request headers are removed from Sentry events.

### Claude Code / Claude Desktop

```sh
# 1. Sign in (opens a browser or shows a device code)
byokey login copilot           # as OpenCode; `--client vscode` to log in as VS Code
byokey login cursor            # or `byokey add-api-key cursor crsr_…`

# 2. Send Claude models to Copilot, and start the proxy
byokey route set --default copilot
byokey serve

# 3. Run Claude Code on it
byokey claude start
```

`byokey claude start [claude args…]` launches Claude Code against BYOKEY;
plain `claude` keeps using your own login. To make BYOKEY the default instead,
run `byokey claude inject`. `byokey claude desktop` does the same for Claude
Desktop (macOS). `byokey doctor` checks the whole setup. Any other Anthropic
Messages client works with `ANTHROPIC_BASE_URL=http://127.0.0.1:8018`.

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
  login         Authenticate with a provider (claude, copilot, cursor)
  add-api-key   Store a static API key as a provider account
  import-claude-code
                Import the local Claude Code CLI's login as a Claude account
  logout        Remove stored credentials for a provider
  status        Show authentication status for all providers
  doctor        Check the server, provider logins, Claude Code and Claude Desktop wiring
  tui           Launch the interactive terminal UI
  accounts      List all accounts for a provider
  switch        Switch the active account for a provider
  claude        Run Claude Code or Claude Desktop against BYOKEY (alias: claude-code)
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

`serve` also binds a Unix control socket at `~/.byokey/control.sock` used by `stop` / `reload`. If the process is launched
with a pre-opened socket via `systemfd`, `systemd`, or `launchd`, the inherited
fd is adopted in place of a fresh bind.

**`byokey start`** — Same options as `serve`. Runs the server in the background
and writes logs to `~/.byokey/server.log` by default.

**`byokey reload`** — Triggers a hot config reload on the running server via
the control socket. No process restart, no dropped connections.

**`byokey login <PROVIDER>`**

Runs the appropriate OAuth flow for the given provider: `claude` (PKCE in
the browser), `copilot` (GitHub device code) or `cursor` (Cursor's browser
login).

```
Options:
      --account <NAME>  Account identifier (default: `default`)
      --client <CLIENT> Client to log in as; Copilot: `opencode` (default) or `vscode`
      --db <PATH>       SQLite DB path [default: ~/.byokey/tokens.db]
```

**`byokey logout <PROVIDER>`** — Deletes the stored token for the given provider.

**`byokey status`** — Prints authentication status for every known provider.

**`byokey doctor`** — Checks that the server answers, `/v1/models` and
`/v1/messages/count_tokens` work, each configured provider is signed in,
whether plain `claude` points at BYOKEY, and whether Claude Desktop's
third-party profile does (macOS). Each failing line says what to run.
Exits non-zero when a check fails.

**`byokey route`** — Lists each Claude model, the provider serving it, the
route that picked it, and the signed-in providers that offer it.
`byokey route set <TARGET> <PROVIDER>` routes a target to `claude`
(Anthropic), `copilot` or `cursor`, and `byokey route unset <TARGET>` removes
that route. The target is one of:

- `--model <MODEL>`: one model, by Anthropic's id (`claude-opus-5-5`)
- `--family <FAMILY>`: `fable` (or `mythos`), `opus`, `sonnet` or `haiku`
- `--default`: every model without a model or family route

A model's route beats its family's, which beats the default; without any, the
model goes to Anthropic. Claude Code's incidental requests (titles,
summaries) use the Haiku model, so `--family haiku` decides where those go.
The routes are saved to the config file, which the running server reloads,
also when `byokey route` creates it. `byokey route set` warns when the
provider is not signed in, and `byokey doctor` checks every routed provider.

**`byokey tui`** — Opens the terminal management UI. It connects to the
ConnectRPC management API at `http://127.0.0.1:8018` by default; override with
`--url <URL>`.

**`byokey accounts <PROVIDER>`** — Lists all accounts for a provider.

**`byokey switch <PROVIDER> <ACCOUNT>`** — Switches the active account for a provider.

**`byokey service <install|uninstall|start|stop|status>`** — Registers byokey
as an OS-managed service. Uses `launchd` on macOS, `systemd` on Linux, and
Windows SCM on Windows. `install` takes the `serve` options; the service logs
to `~/.byokey/server.log` unless `--log-file` says otherwise.

**`byokey claude start [ARGS]…`** — Runs `claude` with `ANTHROPIC_BASE_URL`
pointing at BYOKEY, passing `ARGS` through. A claude.ai login stays in effect,
keeping features such as connectors; without any login, a placeholder
`ANTHROPIC_AUTH_TOKEN` is set so Claude Code can start, along with gateway
model discovery, which lists the Claude models your routes serve in `/model`.

**`byokey claude inject`** — Writes the same settings, plus any
`claude_code.settings` from your byokey config, into `~/.claude/settings.json`,
keeping its other settings. Override the target with `--settings <FILE>`.

**`byokey claude desktop`** — Opens a second Claude Desktop in its
third-party mode against BYOKEY, with models discovered from BYOKEY, next to
the official one, whose profile is never modified. While the BYOKEY instance
runs it can switch Desktop's saved mode, so a cold launch from the Dock may
open it instead of the official one; the command warns about this. macOS only.

All three accept `--url <URL>` to use a BYOKEY other than the configured one.

</details>

## Configuration

Create a config file (JSON or YAML, e.g. `~/.config/byokey/settings.json`) and pass it with `--config`:

```yaml
port: 8018
host: 127.0.0.1

# Which provider serves each Claude model; `byokey route` edits this.
# A model's route beats its family's, which beats the default. Without
# any, models go to Anthropic.
routes:
  default: copilot
  families:
    opus: cursor
  models:
    claude-opus-5-5: copilot

providers:
  # Use Anthropic with a raw API key instead of a login
  # claude:
  #   api_key: "sk-ant-..."

  copilot:
    small_model: gpt-5-mini

  # A `crsr_…` key from cursor.com/dashboard, or `byokey login cursor`
  cursor:
    api_key: "crsr_..."
```

All fields are optional; unspecified providers are enabled by default and use
the login stored in the database. Providers other than `claude`, `copilot`
and `cursor` are rejected in `providers`; Responses custom upstreams belong in `responses.upstreams`.

`/v1/models` lists each Claude model once, under Anthropic's id
(`claude-opus-5-5`), when the provider its route names offers it. Claude
Desktop only recognises those ids, and reads each model's effort levels from
them.

**Copilot** plans that meter premium requests charge one per call, and Claude
Code makes several tool-less calls around each turn (titles, suggestions,
summaries). Set `providers.copilot.small_model: gpt-5-mini` to serve those with
a cheaper model; compaction requests keep the model you chose.
A Copilot organisation can turn off Anthropic's `web_search` and `web_fetch`
server tools by policy; Copilot then rejects the whole request. BYOKEY learns
this from the first rejection, retries without the tool, and leaves it out of
later requests from that account, so Claude Code's `WebSearch` and `WebFetch`
silently do nothing there instead of failing the turn.

The effort Claude Code or Claude Desktop picks (`output_config.effort`)
reaches every provider. Cursor takes it as the model's `effort` parameter and
refuses a level the model lacks, as Anthropic and Copilot do.

**Cursor** serves every model on your Cursor plan, including variants such as
`claude-opus-5-5-high-fast` or `gpt-5.6-sol-low-fast`, which `/v1/models`
does not list. Name them as `cursor/<model>`, so
`byokey claude start --model cursor/claude-opus-5-5-low-fast` runs Claude Code
on Cursor; `copilot/<model>` does the same for Copilot. A prefix overrides the
routes for that request.

## Logs

BYOKEY logs one line for each request it sends upstream, when that exchange
ends:

```
INFO http{… model=claude-opus-5-5 stream=true}:upstream{provider=copilot model=claude-opus-5.5 account=default initiator="user" upstream_request_id="00000-…"}: byokey_proxy::exchange: upstream finished outcome="completed" first_byte_ms=812 duration_ms=14233 input_tokens=9 output_tokens=412 cache_read_tokens=51200 cache_write_tokens=0 stop_reason="end_turn"
```

`outcome` is one of:

- `completed`: the upstream finished its answer.
- `rejected`: the upstream answered with an error status. The line carries
  its `status`, `error_type` and `upstream_message`, such as an organisation
  policy or a context limit.
- `failed`: the connection failed, the stream carried an error, or it stopped
  or went silent before the end.
- `abandoned`: the client went away first, for example Esc in Claude Code.

On Copilot, `initiator` is `user` for a prompt you typed and `agent` for a
tool-loop step, which is how Copilot counts premium requests. `keepalives`
counts the comments BYOKEY wrote while the upstream was silent.
`upstream_message` is text the upstream wrote, so it stays in the local log
and is never sent to Sentry.

Every line of a request starts with its `http{…}` span, which names the
model and streaming mode the client asked for; the `upstream{…}` span names
the model actually sent. `request_id` is BYOKEY's id for the request, which
the client also receives as the `x-request-id` header. When Claude Code sends them, `client_request_id` and
`session` are its `x-client-request-id` and session id: `claude --debug`
prints the former for each API request, so a request in Claude Code's debug
log can be found in BYOKEY's. Management API calls, which `byokey tui` makes
every few seconds, are logged only at `debug`.

`log.level` in the config file takes `RUST_LOG`-style directives, such as
`debug` or `info,byokey_proxy=debug`, and defaults to `info,tarpc=warn`. The
running server applies a change on the next config reload, without a restart.
`RUST_LOG`, when set, overrides it for the life of the process. A value that
does not parse, or that sets no default level (a typo such as `degub` is read
as a module name), stops `serve` from starting.

```yaml
log:
  level: info,byokey_proxy=debug
  format: json      # one JSON object per line; default `text`
  file: /path/to/byokey.log   # rotated daily, instead of stdout
```

Colour is used only when stdout is a terminal and `NO_COLOR` is unset, so a
service's redirected log stays plain text.

## Contributing

With Nix and devenv installed, run `devenv shell` to load the Rust toolchain and build dependencies.

See [CONTRIBUTING.md](CONTRIBUTING.md) for build commands, architecture details, and coding guidelines.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
