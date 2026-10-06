<h4 align="right"><a href="../README.md">English</a> | <strong>简体中文</strong></h4>

<p align="center">
    <img src="./public/icon.png" width=138/>
</p>

<div align="center">

# BYOKEY

**Bring Your Own Keys**<br>
通过本地网关接入 ChatGPT.app / Codex 和 Claude 客户端。<br>
Responses 支持 ChatGPT 订阅、GitHub Copilot 和自定义上游；Anthropic Messages 支持 Copilot、Cursor 和 Claude。

[![ci](https://img.shields.io/github/actions/workflow/status/AprilNEA/BYOKEY/ci.yml?style=flat-square&labelColor=000&color=444&label=ci)](https://github.com/AprilNEA/BYOKEY/actions/workflows/ci.yml)
&nbsp;
[![crates.io](https://img.shields.io/crates/v/byokey?style=flat-square&labelColor=000&color=444)](https://crates.io/crates/byokey)
&nbsp;
[![license](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-444?style=flat-square&labelColor=000)](../LICENSE-MIT)
&nbsp;
[![rust](https://img.shields.io/badge/rust-1.91+-444?style=flat-square&labelColor=000&logo=rust&logoColor=fff)](https://www.rust-lang.org)

</div>

> [!IMPORTANT]
> **BYOKEY 已解除归档。** Responses 网关和原生 Codex HTTP 转发已合入 `master`。
>
> 最新发布版本 `v3.0.0` 尚不包含这些功能。请[从源码构建](#安装)，再按 [ChatGPT.app / Codex 快速开始](#chatgptapp--codex)配置。

> [!NOTE]
> **仅需 Copilot 的 Claude 客户端也可以选择直连。** Copilot 的 `/v1/messages` 端点无需 BYOKEY 即可接受 Claude Code 的请求。Cursor 后端依赖 Cursor 的私有 agent 协议，Cursor 不允许在其官方客户端之外使用，而且在 Claude Code 的工具调用循环中也不可靠。
>
> 直连 Copilot（GitHub 未公开文档化这个接口）：先用 GitHub CLI 登录，再按下面配置。
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
> **Claude Desktop** — 第三方推理，选择 gateway 提供方。Copilot 没有 `/v1/models`，需要按完整 id 列出模型：
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

下图展示 Anthropic Messages 路径。Responses 使用同一服务，但采用独立的模型路由。

```
订阅                                              客户端

GitHub Copilot ─┐                            ┌──  Claude Code
Cursor         ─┼──  byokey serve  ──────────┼──  Claude Desktop
Claude Pro/Max ─┘                            └──  任意 Anthropic Messages 客户端
```

## 功能特性

- **Responses API** — 通过 `/v1/responses` 和 `/codex/responses` 接入 ChatGPT.app / Codex，支持模型别名、自定义上游及 `/codex/models` 模型发现
- **原生 Codex HTTP 转发** — 未匹配已有路由的 `/codex/*` 请求直接转发到配置的 ChatGPT 后端，包括图片生成和编辑端点；不支持 WebSocket
- **Anthropic Messages API** — `/v1/messages`、`/v1/messages/count_tokens` 和 `/v1/models`，与 Claude Code、Claude Desktop 的预期一致，包括 `[1m]` 长上下文模型
- **用 Copilot 跑 Claude** — 走 Copilot 的 Anthropic 格式端点，多账号按剩余配额选择，自动剔除不支持的请求字段；模型选择和服务端工具由客户端控制
- **用 Cursor 跑 Claude** — 通过 Cursor 的 agent 协议使用套餐内的全部模型
- **Claude Code / Claude Desktop 接入** — `byokey claude start`、`byokey claude inject`、`byokey claude desktop`，`byokey doctor` 一次检查全部
- **OAuth 登录与 Token 持久化** — 设备码和 PKCE 流程；SQLite 存储于 `~/.byokey/tokens.db`，后台自动刷新
- **作为系统服务运行** — 注册到 launchd / systemd / Windows SCM，配置热重载

## 支持的 Provider

Responses 支持使用客户端登录凭据的 ChatGPT、使用已存储账户或配置的 API Key 的 Copilot，以及兼容 Responses 的自定义上游。下表列出 Anthropic Messages API 的 Provider。

<table>
  <tr>
    <td align="center" width="200" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.byokey.io/icons/providers/copilot-dark.svg">
        <img src="https://assets.byokey.io/icons/providers/githubcopilot.svg" width="36" alt="GitHub Copilot">
      </picture><br>
      <b>Copilot</b><br>
      <kbd>Device code</kbd><br>
      <sub>claude-fable-5.1<br>claude-opus-5.5<br>claude-sonnet-5<br>……套餐内的全部 Claude 模型</sub>
    </td>
    <td align="center" width="200" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.byokey.io/icons/providers/cursor-dark.svg">
        <img src="https://assets.byokey.io/icons/providers/cursor.svg" width="36" alt="Cursor">
      </picture><br>
      <b>Cursor</b><br>
      <kbd>OAuth · API key</kbd><br>
      <sub>claude-opus-5-5<br>claude-opus-5-5-low-fast<br>composer-2.5<br>……套餐内的全部模型</sub>
    </td>
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/claude.svg" width="36" alt="Claude"><br>
      <b>Claude</b><br>
      <kbd>OAuth · API key</kbd><br>
      <sub>claude-fable-5-1<br>claude-opus-5-5<br>claude-sonnet-5<br>claude-haiku-4-5</sub>
    </td>
  </tr>
</table>

## 安装

如需 Responses 或原生 Codex HTTP 转发，请按下方说明从 `master` 源码构建。已发布的软件包和二进制文件尚不包含这些功能。

**Homebrew（macOS / Linux）**

```sh
brew install AprilNEA/tap/byokey
```

**从 crates.io 安装**

```sh
cargo install byokey
```

**从源码安装**

```sh
git clone https://github.com/AprilNEA/BYOKEY
cd BYOKEY
cargo install --path .
```

> **环境要求：** Rust 1.91+（edition 2024）、用于 SQLite 的 C 编译器，以及用于 ConnectRPC 代码生成的 `protoc`（`brew install protobuf` / `apt-get install protobuf-compiler` / `choco install protoc`）。

## 快速开始

### ChatGPT.app / Codex

这里接入的是 ChatGPT 桌面应用中的 Codex 功能，不是传统的 ChatGPT 会话 API。保持客户端登录 ChatGPT：客户端提供访问令牌和账户标识，并负责刷新令牌。BYOKEY 不导入或保存这份登录凭据。

从源码安装后运行 `byokey serve`，或在已安装 Nix 和 devenv 的源码目录中构建并启动：

```sh
devenv shell cargo run -- serve
```

将以下配置合入 `~/.codex/config.toml`，确保顶层键位于所有 TOML 表之前，然后重启客户端：

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

选择账户可用的模型。Codex 通过 `/codex/models` 发现模型，无需设置 `model_catalog_url`。此配置使用 HTTP SSE 和本地上下文压缩，不支持 WebSocket 或压缩的 Responses 请求体。

模型名称默认显示为「模型名 (来源)」。可通过配置修改展示，不改变路由 ID、指令和能力：

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

`providers` 是两种协议共用的扁平 Provider 表。`claude`、`copilot`、`cursor`、`chatgpt` 是保留的内置名称；其他名称定义自定义 Responses Provider，必须设置 `base_url`。连接设置和模型元数据都写在 Provider 下，`responses.routes.models` 中的别名只包含 `provider` 和发送给该 Provider 的 `model`。`api_key` 接受字面量字符串或显式环境变量引用 `{ env: NAME }`，字面量不会被解释为变量或命令。ChatGPT 凭据由客户端持有，`providers.chatgpt` 不接受 `api_key` 和 `headers`。

`name_format` 使用 MiniJinja 模板，提供纯文本变量 `model` 和 `provider`，支持 `upper`、`replace` 等内置过滤器，不支持模板导入或文件访问。例如 `"{{ provider }} / {{ model }}"` 会将来源放在前面。`providers.<name>.display_name` 同时用于 Responses 和 Anthropic 目录，默认名称为 `ChatGPT`、`Copilot`、`Claude (Anthropic)` 和 `Cursor`，自定义 Provider 默认使用其名称。`providers.<name>.model_overrides` 以发送给该 Provider 的实际模型 ID 为键，不按别名或 `catalog_model` 匹配；每项可设置 `name`（显示名称）、`catalog_model`（元数据相符的 ChatGPT Codex 目录模型）或 `catalog`（完整的 Codex 元数据）。未设置 `catalog_model` 或 `catalog` 时，使用 ChatGPT 目录中同 ID 的元数据。未覆盖的名称保留 ChatGPT 元数据或显式 `catalog.display_name` 的原始拼写，缺少名称时使用模型 ID，不再自动改写连字符或去掉来源后缀。

同一 Provider、同一模型最多显示一个可选条目，优先选择未隐藏的无前缀 ID，再选择带来源前缀的 ID，最后选择配置别名。`hidden_aliases` 精确匹配列表中的 ID，隐藏一个别名不会隐藏同一模型的其他别名。重复别名和手动隐藏的条目都保留元数据，旧会话仍可继续使用；上游已隐藏的模型不会重新显示。不同上游和不同部署不会合并。配置 Copilot API Key 或完成 Copilot 登录后即可自动发现模型，无需占位别名；`providers.copilot.enabled: false` 可禁用该来源。

配置支持热重载，下次模型目录请求即可使用新配置，但客户端可能需要刷新缓存。模板语法错误、未知变量或校验渲染失败会拒绝配置；热重载失败时保留上一份有效配置并记录错误。实际模型数据导致渲染错误或空名称时，目录请求会明确报错，不会静默回退。

未匹配已有路由的 `/codex/*` 请求使用客户端的 ChatGPT 凭据，转发到 `providers.chatgpt.base_url`，默认值为 `https://chatgpt.com/backend-api/codex`。图片生成和编辑等原生接口不受推理模型所选 Provider 影响。转发不会自动启用客户端功能，也不会改写原生接口的模型别名。完整的模型别名、自定义上游和能力限制说明见[英文版快速开始](../README.md#chatgptapp--codex)。

保持监听地址为 `127.0.0.1`，且只配置可信的上游地址。BYOKEY 不为已存储的 Copilot 或自定义上游凭据提供入站鉴权，不要向不可信网络暴露端口。

### Claude Code / Claude Desktop

```sh
# 1. 登录（会打开浏览器或显示设备码）
byokey login copilot           # 以 OpenCode 身份；`--client vscode` 以 VS Code 身份登录
byokey login cursor            # 或 `byokey add-api-key cursor crsr_…`

# 2. 把 Claude 模型转给 Copilot，并启动代理
byokey route set --default copilot
byokey serve

# 3. 在它上面运行 Claude Code
byokey claude start
```

`byokey claude start [claude 参数…]` 以 BYOKEY 启动 Claude Code；直接运行 `claude`
则照常使用你自己的登录。想让 BYOKEY 成为默认，运行 `byokey claude inject`。
`byokey claude desktop` 对 Claude Desktop（macOS）做同样的事。`byokey doctor` 可以检查
整套配置。其他 Anthropic Messages 客户端设置 `ANTHROPIC_BASE_URL=http://127.0.0.1:8018` 即可。

## CLI 参考

```
byokey <COMMAND>

Commands:
  serve         启动代理服务器（前台）
  start         在后台启动代理服务器
  stop          停止后台代理服务器
  restart       重启后台代理服务器
  reload        热重载运行中服务器的配置，无需重启
  service       管理系统级服务注册（launchd / systemd / Windows SCM）
  login         向 Provider 认证（claude、copilot、cursor）
  add-api-key   把静态 API Key 存为某个 Provider 的账户
  import-claude-code
                把本机 Claude Code CLI 的登录导入为 Claude 账户
  logout        删除指定 Provider 的已存储凭据
  status        显示所有 Provider 的认证状态
  doctor        检查服务器、Provider 登录、Claude Code 和 Claude Desktop 的接入状态
  tui           启动交互式终端 UI
  accounts      列出某个 Provider 的所有账户
  switch        切换某个 Provider 的活动账户
  claude        以 BYOKEY 运行 Claude Code 或 Claude Desktop（别名：claude-code）
  completions   生成 Shell 补全脚本
  help          打印帮助信息
```

<details>
<summary><b>命令详情</b></summary>
<br>

**`byokey serve`**

```
Options:
  -c, --config <FILE>   配置文件（JSON 或 YAML）[默认: ~/.config/byokey/settings.json]
  -p, --port <PORT>     监听端口     [默认: 8018]
      --host <HOST>     监听地址     [默认: 127.0.0.1]
      --db <PATH>       SQLite 数据库路径 [默认: ~/.byokey/tokens.db]
      --log-file <PATH> 日志文件路径，按天轮转（默认输出到 stdout）
```

`serve` 还会在 `~/.byokey/control.sock` 绑定一个 Unix 控制套接字，供
`stop` / `reload` 使用。若进程通过 `systemfd`、`systemd` 或 `launchd`
以预打开套接字的方式启动，将直接复用继承的 fd 而不重新绑定。

**`byokey start`** — 与 `serve` 选项相同。在后台运行服务器，
日志默认写入 `~/.byokey/server.log`。

**`byokey reload`** — 通过控制套接字触发运行中服务器的配置热重载。
无需进程重启，不中断现有连接。

**`byokey login <PROVIDER>`**

为指定 Provider 运行相应的 OAuth 流程：`claude`（浏览器 PKCE）、
`copilot`（GitHub 设备码）或 `cursor`（Cursor 浏览器登录）。

```
Options:
      --account <NAME>  账户标识（默认：`default`）
      --client <CLIENT> 登录所用的客户端；Copilot：`opencode`（默认）或 `vscode`
      --db <PATH>       SQLite 数据库路径 [默认: ~/.byokey/tokens.db]
```

**`byokey logout <PROVIDER>`** — 删除指定 Provider 的已存储 Token。

**`byokey status`** — 打印所有已知 Provider 的认证状态。

**`byokey tui`** — 打开终端管理 UI。默认连接
`http://127.0.0.1:8018` 上的 ConnectRPC 管理 API；可用 `--url <URL>`
覆盖。

**`byokey doctor`** — 检查服务器是否响应、`/v1/models` 和
`/v1/messages/count_tokens` 是否可用、各个已配置的 Provider 是否已登录、直接运行
`claude` 时是否指向 BYOKEY，以及 Claude Desktop 的第三方 profile 是否指向 BYOKEY（macOS）。
每条未通过的检查都会给出要运行的命令；有检查失败时以非零状态退出。

**`byokey accounts <PROVIDER>`** — 列出某个 Provider 的所有账户。

**`byokey switch <PROVIDER> <ACCOUNT>`** — 切换某个 Provider 的活动账户。

**`byokey route`** — 列出每个 Claude 模型、当前为它服务的 Provider、选中它的路由，
以及已登录的 Provider 中哪些提供它。`byokey route set <目标> <PROVIDER>` 把目标路由到
`claude`（Anthropic）、`copilot` 或 `cursor`，`byokey route unset <目标>` 删除这条路由。
目标是以下之一：

- `--model <MODEL>`：单个模型，用 Anthropic 的 id（`claude-opus-5-5`）
- `--family <FAMILY>`：模型系列，`fable`（也可写 `mythos`）、`opus`、`sonnet` 或 `haiku`
- `--default`：没有模型或系列路由的所有模型

模型路由优先于系列路由，系列路由优先于默认；都没有时模型走 Anthropic。Claude Code
的附带请求（标题、摘要）用的是 Haiku，所以 `--family haiku` 决定它们的去向。路由保存在
配置文件的 `anthropic.routes` 中，运行中的服务器会自动重新加载，配置文件由 `byokey route` 新建时也一样。
Provider 未登录时 `byokey route set` 会给出提醒，`byokey doctor` 会检查所有被路由到的 Provider。

**`byokey service <install|uninstall|start|stop|status>`** — 将 byokey
注册为系统托管服务。macOS 上使用 `launchd`、Linux 上使用 `systemd`、
Windows 上使用 SCM。`install` 接受与 `serve` 相同的选项；未指定 `--log-file`
时服务日志写入 `~/.byokey/server.log`。

**`byokey claude start [ARGS]…`** — 以指向 BYOKEY 的 `ANTHROPIC_BASE_URL`
运行 `claude`，`ARGS` 原样传入。已登录 claude.ai 时保留该登录，connectors 等功能
照常可用；没有任何登录时会设置占位用的 `ANTHROPIC_AUTH_TOKEN`，保证 Claude Code
能够启动，同时开启网关模型发现，让 `/model` 列出路由所提供的 Claude 模型。

**`byokey claude inject`** — 将同样的设置（以及 byokey 配置中的
`claude_code.settings`）写入 `~/.claude/settings.json`，保留其他设置。可用
`--settings <FILE>` 指定目标文件。

**`byokey claude desktop`** — 在官方 Claude Desktop 旁边再开一个第三方模式的实例，
改用 BYOKEY，官方 profile 不会被改动。启动前读取 BYOKEY 的模型目录，将标准模型 ID、
显示名称和可选的 1M 上下文标志写入 `inferenceModels`，显示名称使用 `labelOverride`，例如
「Claude Opus 5.5 · Copilot」。已确认原生支持 1M 的模型（包括 Opus 5.5、Fable 5/5.1）
只显示一项，不再添加单独的「1M」选项；其他模型在上游声明支持时保留可选的 1M 选项。
此规则只改变列表展示，不改变上游模型的上下文上限。标准 ID 保留 Desktop 识别 effort 的依据；
对于 Desktop 无法识别的模型 ID，不保证 Effort 控件可用。
修改路由、显示设置或可用模型后，退出 BYOKEY Desktop 实例，再运行此命令刷新列表和名称；
服务器路由仍会热重载，Desktop 标签反映上次通过此命令启动时的路由。
目录请求失败或没有可用模型时，不修改 Desktop 设置。BYOKEY 实例运行期间可能
改写 Desktop 保存的模式，官方 Desktop 关着时从 Dock 冷启动可能会打开 BYOKEY 那个，
命令运行时会给出警告。仅支持 macOS。

三个命令都可以用 `--url <URL>` 指定其他 BYOKEY 地址。

</details>

## 配置

创建配置文件（JSON 或 YAML，例如 `~/.config/byokey/settings.json`），通过 `--config` 传入：

```yaml
port: 8018
host: 127.0.0.1

providers:
  # 直接用原始 API Key 走 Anthropic，替代登录
  # claude:
  #   api_key: "sk-ant-..."

  # cursor.com/dashboard 生成的 `crsr_…` Key，或运行 `byokey login cursor`
  cursor:
    api_key: { env: CURSOR_API_KEY }

anthropic:
  # 每个 Claude 模型由哪个 Provider 提供；`byokey route` 会编辑这里。
  # 模型路由优先于系列路由，系列路由优先于 default；都没有时模型走 Anthropic。
  routes:
    default: copilot
    families:
      opus: cursor
    models:
      claude-opus-5-5: copilot
```

所有字段均可选；未指定的 Provider 默认启用，并使用数据库中存储的登录。未知字段会使配置加载失败。
`providers` 是两种协议共用的 Provider 表，自定义 Responses Provider 见[英文版快速开始](../README.md#chatgptapp--codex)；Anthropic 路由只接受 `claude`、`copilot`、`cursor`。
设置 `providers.<name>.enabled: false` 会隐藏该 Provider 的模型，并以 HTTP 400
拒绝路由到它的生成和 token 计数请求，包括带 `copilot/` 或 `cursor/` 显式前缀的请求，
不会自动改走其他 Provider。

`/v1/models` 以 Anthropic 的 id（`claude-opus-5-5`）列出每个 Claude 模型，且只在
其路由指向的 Provider 提供该模型时列出，每个模型只列一次。显示名称标明路由选中的
Provider，标准 ID 保留 Claude Desktop 识别模型和 effort 的依据；显示名称不改变路由。

Provider 和模型的显示名称写在 `providers` 下；Claude 客户端的名称格式通过 `anthropic.catalog` 配置，与 `responses.catalog` 相互独立：

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
```

`name_format` 复用 Responses 目录的 MiniJinja 语法和校验，支持 `model`、`provider` 两个纯文本变量及过滤器。例如 `"{{ provider | upper }} / {{ model }}"` 会把 Provider 名称大写并放在前面。默认格式为 `"{{ model }} · {{ provider }}"`。`providers.<name>.display_name` 与 Responses 目录共用，默认名称为 `Claude (Anthropic)`、`Copilot` 和 `Cursor`。Anthropic 的 `model_overrides` 以标准 Anthropic ID（如 `claude-opus-5-5`）为键，不使用 Provider 前缀或 Provider 自己的写法（如 `claude-opus-5.5`），在该 Provider 提供此模型时生效；未覆盖的模型保留标准友好名称。这些设置仅影响 `display_name` 和 Desktop 的 `labelOverride`，不改变模型 ID、路由、effort 或上下文能力。

服务器会热重载这些设置。无效模板或空名称会使配置加载失败；重载失败时保留上次有效配置。模板在实际模型数据上渲染失败时，目录请求直接报错，不替换成默认名称。修改显示设置后，退出 BYOKEY Desktop 实例，再运行 `byokey claude desktop` 刷新已保存的标签。

**Copilot** 请求保留客户端选择的模型，包括普通无工具聊天和 compaction。
在 Claude Code 客户端选择后台模型：

```sh
ANTHROPIC_DEFAULT_HAIKU_MODEL=copilot/claude-haiku-4-5 byokey claude start
```

如需持久保存，将该变量写入 Claude Code 用户设置的 `env` 对象。该变量同时控制
`haiku` 别名和后台功能，详见 [Claude Code 模型配置](https://code.claude.com/docs/en/model-config#environment-variables)。
选择你的 Copilot 账号可通过 Messages 端点调用的模型。Claude Desktop 继续使用
模型选择器中选定的模型。

Copilot 组织可以通过策略关闭 Anthropic 的 `web_search`、`web_fetch` 服务端工具，
此时 BYOKEY 返回上游错误，不会删除工具或重试修改后的请求。后续生成和 token 计数
请求也会保留工具。请在组织策略中启用工具，或在客户端明确禁用对应工具。

Claude Code 或 Claude Desktop 选择的 effort（`output_config.effort`）会传到每个
Provider。Cursor 把它作为模型的 `effort` 参数，模型不支持的档位会被拒绝，和 Anthropic、
Copilot 的行为一致。

**Cursor** 提供 Cursor 套餐内的全部模型，包括 `claude-opus-5-5-high-fast`、
`gpt-5.6-sol-low-fast` 这类变体，`/v1/models` 不会列出它们。以 `cursor/<model>`
指定，例如 `byokey claude start --model cursor/claude-opus-5-5-low-fast` 就会让
Claude Code 走 Cursor；`copilot/<model>` 对 Copilot 同理。前缀会覆盖该请求的路由。

### 从旧配置迁移

BYOKEY 不会自动迁移配置文件。旧字段和未知字段都会使配置加载失败；热重载失败时保留上次有效配置。请手动修改：

| 旧配置 | 新配置 |
| --- | --- |
| 根级 `routes` | `anthropic.routes` |
| `responses.default` | `responses.routes.default` |
| `responses.models.<alias>`（`upstream`、`model`） | `responses.routes.models.<alias>`（`provider`、`model`） |
| 别名上的 `catalog_model` 或 `catalog` | `providers.<provider>.model_overrides.<model>.catalog_model` 或 `.catalog` |
| `responses.upstreams.<name>` | `providers.<name>`（必须设置 `base_url`） |
| `responses.chatgpt_base_url` | `providers.chatgpt.base_url` |
| `responses.catalog.provider_names`、`anthropic.catalog.provider_names` | `providers.<name>.display_name` |
| `responses.catalog.model_names`、`anthropic.catalog.model_names` | `providers.<provider>.model_overrides.<model>.name` |
| `providers.claude.backend` | `anthropic.routes.default` |
| `providers.copilot.small_model` | 删除；在客户端设置 `ANTHROPIC_DEFAULT_HAIKU_MODEL` |

`port`、`host`、`proxy_url`、`log`、`telemetry` 和 `claude_code` 保持不变。

## 日志

BYOKEY 每向上游发出一个请求，就在这次交互结束时记一行日志：

```
INFO http{… model=claude-opus-5-5 stream=true}:upstream{provider=copilot model=claude-opus-5.5 account=default initiator="user" upstream_request_id="00000-…"}: byokey_proxy::exchange: upstream finished outcome="completed" first_byte_ms=812 duration_ms=14233 input_tokens=9 output_tokens=412 cache_read_tokens=51200 cache_write_tokens=0 stop_reason="end_turn"
```

`outcome` 取以下值之一：

- `completed`：上游完整给出了回答。
- `rejected`：上游返回了错误状态码。这一行带有它的 `status`、`error_type` 和
  `upstream_message`，例如组织策略或上下文长度限制。
- `failed`：连接失败、流中出现错误，或者流在结束前中断或长时间无响应。
- `abandoned`：客户端先离开了，例如在 Claude Code 里按了 Esc。

在 Copilot 上，`initiator` 为 `user` 表示你输入的提示词，为 `agent` 表示工具循环中的
一步，Copilot 按这个区分 premium request 的计数。`keepalives` 是上游沉默期间
BYOKEY 写出的保活注释数。`upstream_message` 是上游写的文本，只留在本地日志里，
不会发送到 Sentry。

一个请求的每行日志都以它的 `http{…}` span 开头，其中有客户端请求的模型和是否流式；
`upstream{…}` span 里是实际发给上游的模型。`request_id` 是 BYOKEY 给这个请求的
id，客户端也会在响应头 `x-request-id` 里收到它。Claude Code 发送时，`client_request_id`
和 `session` 分别是它的 `x-client-request-id` 和会话 id：`claude --debug` 会为每个 API
请求打印前者，因此可以用它在 BYOKEY 的日志里找到 Claude Code 调试日志中的请求。
`byokey tui` 每隔几秒调用一次的管理 API 只在 `debug` 级别记录。

配置文件中的 `log.level` 使用 `RUST_LOG` 风格的指令，例如 `debug` 或
`info,byokey_proxy=debug`，默认值为 `info,tarpc=warn`。修改后，运行中的服务会在下一次
重新加载配置时生效，无需重启。设置了 `RUST_LOG` 时，它在整个进程生命周期内覆盖
`log.level`。无法解析的值，或者没有设置默认级别的值（例如拼错的 `degub` 会被当作模块名），
会让 `serve` 无法启动。

```yaml
log:
  level: info,byokey_proxy=debug
  format: json      # 每行一个 JSON 对象；默认为 `text`
  file: /path/to/byokey.log   # 按天轮转，替代 stdout
```

只有 stdout 是终端且未设置 `NO_COLOR` 时才输出颜色，因此服务重定向出来的日志保持纯文本。

## 贡献

请参阅 [CONTRIBUTING.md](../CONTRIBUTING.md) 了解构建命令、架构细节和编码规范。

## 许可证

双协议授权，任选其一：[MIT](../LICENSE-MIT) 或 [Apache-2.0](../LICENSE-APACHE)。
