<h4 align="right"><a href="../README.md">English</a> | <strong>简体中文</strong></h4>

<p align="center">
    <img src="./public/icon.png" width=138/>
</p>

<div align="center">

# BYOKEY

**Bring Your Own Keys**<br>
用你已有的订阅运行 Claude Code 和 Claude Desktop。<br>
一个本地 Anthropic Messages API，后端是 GitHub Copilot、Cursor 或你自己的 Claude 登录。

[![ci](https://img.shields.io/github/actions/workflow/status/AprilNEA/BYOKEY/ci.yml?style=flat-square&labelColor=000&color=444&label=ci)](https://github.com/AprilNEA/BYOKEY/actions/workflows/ci.yml)
&nbsp;
[![crates.io](https://img.shields.io/crates/v/byokey?style=flat-square&labelColor=000&color=444)](https://crates.io/crates/byokey)
&nbsp;
[![license](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-444?style=flat-square&labelColor=000)](../LICENSE-MIT)
&nbsp;
[![rust](https://img.shields.io/badge/rust-1.91+-444?style=flat-square&labelColor=000&logo=rust&logoColor=fff)](https://www.rust-lang.org)

</div>

```
订阅                                              客户端

GitHub Copilot ─┐                            ┌──  Claude Code
Cursor         ─┼──  byokey serve  ──────────┼──  Claude Desktop
Claude Pro/Max ─┘                            └──  任意 Anthropic Messages 客户端
```

## 功能特性

- **Anthropic Messages API** — `/v1/messages`、`/v1/messages/count_tokens` 和 `/v1/models`，与 Claude Code、Claude Desktop 的预期一致，包括 `[1m]` 长上下文模型
- **用 Copilot 跑 Claude** — 走 Copilot 的 Anthropic 格式端点，多账号按剩余配额轮换，Claude Code 的附带请求可改走便宜模型，Copilot 不接受的字段自动剔除
- **用 Cursor 跑 Claude** — 通过 Cursor 的 agent 协议使用套餐内的全部模型
- **Claude Code / Claude Desktop 接入** — `byokey claude start`、`byokey claude inject`、`byokey claude desktop`，`byokey doctor` 一次检查全部
- **OAuth 登录与 Token 持久化** — 设备码和 PKCE 流程；SQLite 存储于 `~/.byokey/tokens.db`，后台自动刷新
- **作为系统服务运行** — 注册到 launchd / systemd / Windows SCM，配置热重载

## 支持的 Provider

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

```sh
# 1. 登录（会打开浏览器或显示设备码）
byokey login copilot           # 以 OpenCode 身份；`--client vscode` 以 VS Code 身份登录
byokey login cursor            # 或 `byokey add-api-key cursor crsr_…`

# 2. 启动代理，并把 Claude Code 的请求转给 Copilot
byokey serve                   # 配置里写上 `providers.claude.backend: copilot`

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

**`byokey service <install|uninstall|start|stop|status>`** — 将 byokey
注册为系统托管服务。macOS 上使用 `launchd`、Linux 上使用 `systemd`、
Windows 上使用 SCM。`install` 接受与 `serve` 相同的选项；未指定 `--log-file`
时服务日志写入 `~/.byokey/server.log`。

**`byokey claude start [ARGS]…`** — 以指向 BYOKEY 的 `ANTHROPIC_BASE_URL`
运行 `claude`，`ARGS` 原样传入。已登录 claude.ai 时保留该登录，connectors 等功能
照常可用；没有任何登录时会设置占位用的 `ANTHROPIC_AUTH_TOKEN`，保证 Claude Code
能够启动，同时开启网关模型发现，让 `/model` 列出你 Copilot 和 Cursor 账号下的
Claude 模型（`copilot/…`、`cursor/…`）。

**`byokey claude inject`** — 将同样的设置（以及 byokey 配置中的
`claude_code.settings`）写入 `~/.claude/settings.json`，保留其他设置。可用
`--settings <FILE>` 指定目标文件。

**`byokey claude desktop`** — 在官方 Claude Desktop 旁边再开一个第三方模式的实例，
改用 BYOKEY，模型从 BYOKEY 发现，官方 profile 不会被改动。BYOKEY 实例运行期间可能
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
  # 把 Claude Code / Claude Desktop 的所有请求转给 Copilot（或 `cursor`）。
  # 不设置时，不带前缀的模型走 Anthropic。
  claude:
    backend: copilot
    # 或者直接用原始 API Key 走 Anthropic，替代登录
    # api_key: "sk-ant-..."

  copilot:
    small_model: gpt-5-mini

  # cursor.com/dashboard 生成的 `crsr_…` Key，或运行 `byokey login cursor`
  cursor:
    api_key: "crsr_..."
```

所有字段均可选；未指定的 Provider 默认启用，并使用数据库中存储的登录。
`claude`、`copilot`、`cursor` 之外的 Provider 会被拒绝。

**Copilot** 按 premium request 计费的套餐每次调用计一次，而 Claude Code 每轮对话
前后会发出多个不带工具的调用（标题、建议、摘要）。设置
`providers.copilot.small_model: gpt-5-mini` 可以让这些调用改走便宜的模型；
compaction 请求仍使用你选择的模型。

**Cursor** 提供 Cursor 套餐内的全部模型，包括 `claude-opus-5-5-high-fast`、
`gpt-5.6-sol-low-fast` 这类变体。以 `cursor/<model>` 指定，例如
`byokey claude start --model cursor/claude-opus-5-5-low-fast` 就会让 Claude Code
走 Cursor；`copilot/<model>` 对 Copilot 同理。设置 `providers.claude.backend: cursor`
可以把 Claude Code 的全部请求都转给 Cursor。

## 贡献

请参阅 [CONTRIBUTING.md](../CONTRIBUTING.md) 了解构建命令、架构细节和编码规范。

## 许可证

双协议授权，任选其一：[MIT](../LICENSE-MIT) 或 [Apache-2.0](../LICENSE-APACHE)。
