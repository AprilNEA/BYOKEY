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
> **BYOKEY is no longer archived.** The Responses gateway and native Codex HTTP forwarding are available on `master`.
>
> The latest published release, `v3.0.0`, does not include these features. [Build from source](#installation) to use the [ChatGPT.app / Codex quick start](#chatgptapp--codex).

> [!NOTE]
> **Direct Copilot access is an alternative for Claude clients that only need Copilot.** Its `/v1/messages` endpoint accepts Claude Code's requests without BYOKEY. The Cursor backend drives Cursor's private agent protocol, which Cursor does not allow outside its own clients, and it does not hold up in Claude Code's tool loops.
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

The Anthropic Messages path is shown below. Responses uses the same server with separate model routing.

```
Subscriptions                                     Clients

GitHub Copilot ─┐                            ┌──  Claude Code
Cursor         ─┼──  byokey serve  ──────────┼──  Claude Desktop
Claude Pro/Max ─┘                            └──  any Anthropic Messages client
```

## Features

- **Responses API** — `/v1/responses` and `/codex/responses` for ChatGPT.app / Codex, with model aliases, custom upstreams, and Codex model discovery at `/codex/models`
- **Native Codex HTTP forwarding** — unmatched `/codex/*` paths go to the configured ChatGPT backend, including image generation and editing endpoints; no WebSocket support
- **Anthropic Messages API** — `/v1/messages`, `/v1/messages/count_tokens` and `/v1/models`, as Claude Code and Claude Desktop expect them; `[1m]` long-context ids included
- **Copilot as a Claude backend** — Copilot's Anthropic-format endpoint, with quota-aware account selection and unsupported request fields stripped; model selection and server tools stay under client control
- **Cursor as a Claude backend** — every model on your Cursor plan, driven through Cursor's agent protocol
- **Claude Code and Claude Desktop wiring** — `byokey claude start`, `byokey claude inject`, `byokey claude desktop`; `byokey doctor` checks it all
- **OAuth login and token persistence** — device-code and PKCE flows; SQLite at `~/.byokey/tokens.db`, tokens refreshed in the background
- **Runs as a service** — launchd / systemd / Windows SCM registration, hot-reloaded config

## Supported Providers

Responses supports ChatGPT with client-owned credentials, Copilot with stored accounts or a configured API key, and custom Responses-compatible upstreams. The providers below serve the Anthropic Messages API.

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

For Responses and native Codex HTTP forwarding, build from `master` using the source instructions below. These features are not yet in the published packages or release binaries.

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

After installing from source, run `byokey serve`. Alternatively, with Nix and devenv installed, build and start this checkout:

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

[features]
enable_request_compression = false
```

Use a model available to your account. With a ChatGPT login, Codex discovers the catalog at `base_url/models`. Leave `model_catalog_url` unset: explicitly configured catalog URLs have a 1 MiB limit, which a combined multi-provider catalog can exceed. This setup uses HTTP SSE and local compaction; WebSocket transport and compressed Responses request bodies are not supported. Keep web search disabled until the selected upstream supports the client's search tools.

Unmatched `/codex/*` HTTP paths, including `images/generations` and `images/edits`, go directly to `providers.chatgpt.base_url` (default `https://chatgpt.com/backend-api/codex`) with the client's ChatGPT credentials. This fallback is independent of the selected inference provider. It preserves the method, path suffix, query, body bytes, upstream status and end-to-end headers; cookies and connection-specific request headers are removed. Request and response bodies stream without JSON parsing or automatic decompression. Redirects and retries are disabled. Native requests are logged but do not contribute to Responses token usage statistics. The fallback does not enable client-side features or translate model aliases for native endpoints.

With no Responses configuration, inference requests go to ChatGPT. To add Copilot and a custom gateway, sign in with `byokey login copilot` and merge the following into the BYOKEY config passed to `serve --config`:

```yaml
providers:
  company:
    base_url: https://gateway.example.com/team/v1
    models_url: https://gateway.example.com/team/v1/models
    display_name: Company
    api_key: { env: COMPANY_API_KEY }
    headers:
      X-Tenant: engineering
      X-Special-Token: { env: COMPANY_GATEWAY_TOKEN }
      X-Request-UID: { uuid_prefix: "byokey-" }
    model_overrides:
      my-deployment:
        catalog_model: gpt-5.4-mini

responses:
  routes:
    default: chatgpt
    models:
      copilot-fast: { provider: copilot, model: gpt-5.4-mini }
      company-fast: { provider: company, model: my-deployment }
```

`providers` is one flat map for both protocols. `claude`, `copilot`, `cursor` and `chatgpt` are reserved built-in names; any other name defines a custom Responses provider and requires `base_url`. Connection settings and model metadata belong to the provider, so a route alias holds only `provider` and the `model` ID sent to it.

`api_key` and header values accept a literal string or an explicit environment reference `{ env: NAME }`; BYOKEY never interprets a literal string as a variable or command. Export the referenced variables in the **BYOKEY server process**, not just the client. Missing variables fail the request. A configured `Authorization` header overrides `api_key`. `models_url`, `headers` and `service_tier` are accepted only on custom providers. `/responses` is appended to each `base_url`; custom providers must implement the Responses API themselves.

`providers.<name>.model_overrides` is keyed by the actual model ID sent to that provider, never by an alias. Each entry may set `name` (display name), `catalog_model` (a ChatGPT Codex catalog slug whose metadata matches this model) or `catalog` (a complete Codex ModelInfo object). Without `catalog_model` or `catalog`, the model borrows the ChatGPT catalog entry with the same ID. In the example, `company-fast` assumes that `my-deployment` serves the same model as `gpt-5.4-mini`; choose matching metadata for the actual deployment.

Set `models_url` to an OpenAI-compatible `{"data":[{"id":"..."}]}` endpoint to discover multiple models without defining each alias. BYOKEY lists every ID that also has ChatGPT Codex metadata as `<provider>/<model>`, displayed by default as `<model display name> (<display_name>)`. The provider label defaults to the provider name. Models without matching metadata need a complete `model_overrides.<model>.catalog`; BYOKEY does not fabricate their instructions or capabilities. A model list does not guarantee current account access or Responses support. The model list URL receives the same configured credentials and headers as the Responses URL, never the client's ChatGPT credentials. Configure only trusted URLs. Catalog errors are returned to the client. Explicit aliases override discovered entries with the same slug. Omit `models_url` to keep manual aliases only.

All catalog entries default to `<model name> (<provider>)`, including native models. Configure presentation separately from routing:

```yaml
providers:
  copilot:
    display_name: GitHub Copilot
    model_overrides:
      gpt-6-astra:
        name: GPT-6 Astra

responses:
  catalog:
    name_format: "{{ model }} ({{ provider }})"
    hidden_aliases: ["LLM Router"]
```

`name_format` uses [MiniJinja](https://docs.rs/minijinja/2.24.0/minijinja/syntax/index.html) with two plain-text variables: `model` and `provider`. For example, `"{{ provider }} / {{ model }}"` puts the source first. Built-in filters such as `upper` and `replace` are available; template imports and filesystem access are not. `providers.<name>.display_name` sets the provider label in both the Responses and Anthropic catalogs. The labels default to `ChatGPT`, `Copilot`, `Claude (Anthropic)` and `Cursor`; a custom provider defaults to its name. `model_overrides.<model ID>.name` renames that provider's actual model ID, not an alias or `catalog_model`. Without an override, BYOKEY preserves the shared ChatGPT or explicit `catalog.display_name` verbatim, falling back to the model ID when metadata has no name. It does not rewrite hyphens or strip provider suffixes. Route IDs, instructions and capabilities are unaffected.

Enabled Copilot credentials, from a stored login or `providers.copilot.api_key`, enable discovery without a model alias. The picker shows at most one visible entry per provider and model, preferring the default provider's unqualified ID, then a provider-prefixed ID, then a configured alias. `hidden_aliases` hides exact catalog slugs before that selection; hiding one alias does not hide other aliases for the same model. Duplicate and explicitly hidden aliases retain their metadata with `visibility: hide`, so existing sessions and explicit model IDs still work. Upstream-hidden models remain hidden.

These settings hot-reload. The next catalog request uses the new settings; the client may need to refresh its cached model list. Syntax errors, unknown variables and failed validation renders reject the configuration. Failed reloads retain the last valid configuration and log the error. Render errors with actual model data fail the catalog request rather than silently substituting a name; names must be nonempty.

`uuid_prefix` generates a fresh lowercase UUID v4 for each upstream request, preceded by the configured prefix. Set a custom provider's optional `service_tier` to override the client's top-level `service_tier`, for example `service_tier: fast` when that upstream supports it. Without an override, BYOKEY preserves the client's value. Do not wrap Responses parameters in `extra_body`; send them at the top level.

Select a provider-labelled model in the client, or set a model ID such as `copilot-fast` or `company-fast` explicitly. Exact aliases take priority, followed by `chatgpt/<model>`, `copilot/<model>` or `<provider>/<model>`, followed by `responses.routes.default` for unqualified names. Copilot models must advertise `/responses`; BYOKEY does not translate Chat Completions or Anthropic requests on this path. Copilot uses BYOKEY's stored accounts or `providers.copilot.api_key`, never the client's ChatGPT credential.

Copilot can change an output item's ID between stream events. BYOKEY retains the first ID for each output index so Codex updates one message instead of displaying a duplicate. Response IDs and tool `call_id` values remain unchanged. ChatGPT and custom providers retain their original stream payloads.

The catalog borrows actual ChatGPT model metadata, including instructions and capabilities. When `model_messages.instructions_template` is present, BYOKEY omits the ignored legacy `base_instructions` copy to reduce catalog size. Each routed model needs a ChatGPT catalog entry with its ID or its `catalog_model`, or a complete Codex ModelInfo object in `model_overrides.<model>.catalog`. A missing match is an error. A custom default with complete catalog objects, no `models_url`, and no enabled Copilot credentials or routes needs no ChatGPT catalog access; otherwise catalog discovery needs the client's ChatGPT login. Alias upgrades are disabled so the client does not migrate an alias to a different route. The existing `/v1/models`, `byokey route`, and TUI route list remain Anthropic-only.

Send `Reply with exactly: gateway-ok` and check the gateway log for the intended upstream and model. Then test a tool call and a follow-up. Upstream errors and `retry-after` are preserved. Normal Responses streams end with `response.completed`; failed, incomplete and truncated streams are recorded as failures, while client cancellation is recorded as `abandoned`.

Keep the listener on `127.0.0.1`. BYOKEY has no inbound authentication for its stored Copilot/custom credentials; do not expose this port to an untrusted network. Only the configured ChatGPT backend receives the client's ChatGPT auth headers; configure only a trusted `providers.chatgpt.base_url`. The client owns ChatGPT login and refresh, so `providers.chatgpt` rejects `api_key` and `headers`. Redirects are not followed. Responses routes are excluded from `BYOKEY_DUMP`, and their request headers are removed from Sentry events.

Local logs and usage records retain account identifiers. Local error logs can contain text quoted by the upstream. Account identifiers and upstream error text are removed from Sentry event fields, trace data and breadcrumbs.

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
The routes are saved to `anthropic.routes` in the config file, which the running server reloads,
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
third-party mode against BYOKEY, next to the official one, whose profile is
never modified. Before launch, the command fetches BYOKEY's routed models
and writes an explicit `inferenceModels` list. Each entry keeps its standard
Anthropic ID, with a provider-labelled `labelOverride` such as
`Claude Opus 5.5 · Copilot`. Models documented as native 1M, including
Opus 5.5 and Fable 5/5.1, appear once by default without an additional `1M` entry.
Other models retain the optional 1M entry when the upstream advertises support.
This changes the picker, not the upstream model's context limit, and preserves Desktop's effort
recognition without adding provider prefixes to model IDs; Desktop's Effort
control is not guaranteed for model IDs Desktop does not recognize. After changing
routes, display settings or available models, quit the BYOKEY Desktop instance and run the
command again to refresh its list and labels. Server-side routes still
hot-reload; labels describe the routes at the last launch through this command.
If the catalog request fails or lists no models, Desktop settings stay unchanged.
While the BYOKEY instance runs it can switch Desktop's saved mode, so a cold
launch from the Dock may open it instead of the official one; the command
warns about this. macOS only.

All three accept `--url <URL>` to use a BYOKEY other than the configured one.

</details>

## Configuration

Create a config file (JSON or YAML, e.g. `~/.config/byokey/settings.json`) and pass it with `--config`:

```yaml
port: 8018
host: 127.0.0.1

providers:
  # Use Anthropic with a raw API key instead of a login
  # claude:
  #   api_key: "sk-ant-..."

  # A `crsr_…` key from cursor.com/dashboard, or `byokey login cursor`
  cursor:
    api_key: { env: CURSOR_API_KEY }

anthropic:
  # Which provider serves each Claude model; `byokey route` edits this.
  # A model's route beats its family's, which beats the default. Without
  # any, models go to Anthropic.
  routes:
    default: copilot
    families:
      opus: cursor
    models:
      claude-opus-5-5: copilot
```

All fields are optional; unspecified providers are enabled by default and use
the login stored in the database. Unknown fields fail configuration loading. `providers` is the single provider map for both protocols; see [ChatGPT.app / Codex](#chatgptapp--codex) for custom Responses providers. Anthropic routes accept only `claude`, `copilot` and `cursor`.
Setting `providers.<name>.enabled: false` hides that provider's models and
rejects Messages and token-count requests routed to it with HTTP 400, including
explicit `copilot/` or `cursor/` prefixes. Requests do not fall back to another provider.

`/v1/models` lists each Claude model once, under Anthropic's id
(`claude-opus-5-5`), when the provider its route names offers it. Display
names identify the routed provider. Standard IDs preserve Claude Desktop's
model and effort recognition; provider labels do not change request routing.

Provider and model display names live under `providers`; the Claude client name format in `anthropic.catalog` is independent of `responses.catalog`:

```yaml
providers:
  claude:
    display_name: Anthropic
  copilot:
    display_name: GitHub Copilot
    model_overrides:
      claude-opus-5-5:
        name: Opus 5.5

anthropic:
  catalog:
    name_format: "{{ model }} · {{ provider }}"
    merge_native_1m: true
```

`name_format` reuses the Responses catalog's MiniJinja syntax and validation, with the plain-text variables `model` and `provider`. Filters work here too: `"{{ provider | upper }} / {{ model }}"` puts the provider first. The default format is `"{{ model }} · {{ provider }}"`. `providers.<name>.display_name` is shared with the Responses catalog; labels default to `Claude (Anthropic)`, `Copilot` and `Cursor`. Anthropic `model_overrides` keys are canonical standard Anthropic IDs such as `claude-opus-5-5`, not provider-prefixed IDs or the provider's own spelling such as `claude-opus-5.5`. The override applies when that provider serves the model. Without an override, the model keeps its standard friendly name. These naming settings change only `display_name` and Desktop's `labelOverride`, not IDs, routing, effort or context capabilities. Each model still appears once, labelled with its routed provider.

`anthropic.catalog.merge_native_1m` defaults to `true`: native 1M models keep only their standard picker entry. Set it to `false` to offer an additional `1M` entry when the upstream advertises support, including for native 1M models. This setting does not change model IDs, routing, effort, upstream context limits or the Responses catalog.

The server hot-reloads these settings. Invalid templates or empty names reject configuration loading; failed reloads retain the last valid configuration. A template that fails with actual model data fails the catalog request rather than substituting a name. After changing catalog settings, quit the BYOKEY Desktop instance and rerun `byokey claude desktop` to refresh its saved model list and labels.

**Copilot** requests keep the client-selected model, including ordinary chat
without tools and compaction. Select Claude Code's background model in the
client:

```sh
ANTHROPIC_DEFAULT_HAIKU_MODEL=copilot/claude-haiku-4-5 byokey claude start
```

To persist the choice, set this variable in the `env` object of Claude Code's
user settings. The variable controls both the `haiku` alias and background
functionality; see [Claude Code's model configuration](https://code.claude.com/docs/en/model-config#environment-variables).
Choose a model available to your Copilot account on the Messages endpoint.
Claude Desktop continues to use the model selected in its picker.

A Copilot organisation can turn off Anthropic's `web_search` and `web_fetch`
server tools by policy. BYOKEY returns the upstream error without removing
tools or retrying a modified request. Later generation and token-count
requests also retain their tools. Enable the tools in the organisation's
policy or explicitly disable them in the client.

The effort Claude Code or Claude Desktop picks (`output_config.effort`)
reaches every provider. Cursor takes it as the model's `effort` parameter and
refuses a level the model lacks, as Anthropic and Copilot do.

**Cursor** serves every model on your Cursor plan, including variants such as
`claude-opus-5-5-high-fast` or `gpt-5.6-sol-low-fast`, which `/v1/models`
does not list. Name them as `cursor/<model>`, so
`byokey claude start --model cursor/claude-opus-5-5-low-fast` runs Claude Code
on Cursor; `copilot/<model>` does the same for Copilot. A prefix overrides the
routes for that request.

### Migrating from the previous configuration

BYOKEY does not migrate configuration files automatically. Legacy and unknown fields fail configuration loading, and a failed hot reload keeps the last valid configuration. Edit the file by hand:

| Previous setting | Current setting |
| --- | --- |
| `routes` (root) | `anthropic.routes` |
| `responses.default` | `responses.routes.default` |
| `responses.models.<alias>` with `upstream`, `model` | `responses.routes.models.<alias>` with `provider`, `model` |
| `catalog_model` or `catalog` on an alias | `providers.<provider>.model_overrides.<model>.catalog_model` or `.catalog` |
| `responses.upstreams.<name>` | `providers.<name>` (requires `base_url`) |
| `responses.chatgpt_base_url` | `providers.chatgpt.base_url` |
| `responses.catalog.provider_names`, `anthropic.catalog.provider_names` | `providers.<name>.display_name` |
| `responses.catalog.model_names`, `anthropic.catalog.model_names` | `providers.<provider>.model_overrides.<model>.name` |
| `providers.claude.backend` | `anthropic.routes.default` |
| `providers.copilot.small_model` | Remove; set `ANTHROPIC_DEFAULT_HAIKU_MODEL` in the client |

`port`, `host`, `proxy_url`, `log`, `telemetry` and `claude_code` are unchanged.

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
