<div align="center">

# BYOKEY

**Bring Your Own Keys**<br>
将 AI 订阅转换为标准 API 端点。<br>
以 OpenAI 或 Anthropic 兼容格式暴露任意 Provider — 本地运行或云端部署。

[![ci](https://img.shields.io/github/actions/workflow/status/AprilNEA/BYOKEY/ci.yml?style=flat-square&labelColor=000&color=444&label=ci)](https://github.com/AprilNEA/BYOKEY/actions/workflows/ci.yml)
&nbsp;
[![crates.io](https://img.shields.io/crates/v/byokey?style=flat-square&labelColor=000&color=444)](https://crates.io/crates/byokey)
&nbsp;
[![license](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-444?style=flat-square&labelColor=000)](LICENSE-MIT)
&nbsp;
[![rust](https://img.shields.io/badge/rust-1.85+-444?style=flat-square&labelColor=000&logo=rust&logoColor=fff)](https://www.rust-lang.org)

</div>

```
订阅                                              工具

Claude Pro  ─┐                              ┌──  Amp Code
OpenAI Plus ─┼──  byokey serve  ────────────┼──  Cursor · Windsurf
Copilot     ─┘                              ├──  Factory CLI (Droid)
                                            └──  任意 OpenAI / Anthropic 客户端
```

## 功能特性

- **多格式 API** — 同时兼容 OpenAI 和 Anthropic 端点，只需修改 base URL
- **OAuth 登录流程** — 自动处理 PKCE、设备码、授权码等流程
- **Token 持久化** — SQLite 存储于 `~/.byokey/tokens.db`，重启后依然有效
- **API Key 直通** — 在配置中设置原始 Key，跳过 OAuth
- **随处部署** — 本地 CLI 运行，或部署为共享 AI 网关
- **Agent 就绪** — 原生支持 [Amp Code](https://ampcode.com)；[Factory CLI (Droid)](https://factory.ai) 即将到来
- **热重载配置** — 基于 YAML，所有选项均有合理默认值

## 支持的 Provider

<table>
  <tr>
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/claude.svg" width="36" alt="Claude"><br>
      <b>Claude</b><br>
      <sup>PKCE</sup><br>
      <sub>claude-opus-4-6<br>claude-sonnet-4-5<br>claude-haiku-4-5</sub>
    </td>
    <td align="center" width="200" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.byokey.io/icons/providers/codex-dark.svg">
        <img src="https://assets.byokey.io/icons/providers/codex.svg" width="36" alt="Codex">
      </picture><br>
      <b>Codex</b><br>
      <sup>PKCE</sup><br>
      <sub>gpt-5.4<br>gpt-5.3-codex<br>gpt-5.1-codex-max<br>o3 · o4-mini</sub>
    </td>
    <td align="center" width="200" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.byokey.io/icons/providers/copilot-dark.svg">
        <img src="https://assets.byokey.io/icons/providers/githubcopilot.svg" width="36" alt="GitHub Copilot">
      </picture><br>
      <b>Copilot</b><br>
      <sup>设备码</sup><br>
      <sub>gpt-5.4<br>claude-sonnet-4.6<br>gemini-3.1-pro<br>grok-code-fast-1</sub>
    </td>
  </tr>
  <tr>
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/gemini.svg" width="36" alt="Gemini"><br>
      <b>Gemini</b><br>
      <sup>PKCE</sup><br>
      <sub>gemini-2.0-flash<br>gemini-1.5-pro<br>gemini-1.5-flash</sub>
    </td>
    <td align="center" width="200" valign="top">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="https://assets.byokey.io/icons/providers/amazonwebservices-dark.svg">
        <img src="https://assets.byokey.io/icons/providers/amazonwebservices.svg" width="36" alt="AWS">
      </picture><br>
      <b>Kiro</b><br>
      <sup>设备码</sup><br>
      <sub>kiro-default</sub>
    </td>
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/gemini.svg" width="36" alt="Antigravity"><br>
      <b>Antigravity</b><br>
      <sup>PKCE</sup><br>
      <sub>ag-gemini-2.5-pro<br>ag-gemini-2.5-flash<br>ag-claude-sonnet-4-5</sub>
    </td>
  </tr>
  <tr>
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/alibabacloud.svg" width="36" alt="Qwen"><br>
      <b>Qwen</b><br>
      <sup>设备码</sup><br>
      <sub>qwen3-max<br>qwen3-coder-plus<br>qwen-plus</sub>
    </td>
    <td align="center" width="200" valign="top">
      <img src="https://assets.byokey.io/icons/providers/kimi.svg" width="36" alt="Kimi"><br>
      <b>Kimi</b><br>
      <sup>设备码</sup><br>
      <sub>kimi-k2-0711</sub>
    </td>
    <td align="center" width="200" valign="top">
      <b>iFlow</b><br>
      <sup>授权码</sup><br>
      <sub>glm-4.5<br>glm-z1-flash<br>kimi-k2</sub>
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

> **环境要求：** Rust 1.85+（edition 2024）、用于 SQLite 的 C 编译器，以及用于 ConnectRPC 代码生成的 `protoc`（`brew install protobuf` / `apt-get install protobuf-compiler` / `choco install protoc`）。

## 快速开始

```sh
# 1. 认证（会打开浏览器或显示设备码）
byokey login claude
byokey login codex
byokey login copilot

# 2. 启动代理
byokey serve

# 3. 将工具指向代理地址
export OPENAI_BASE_URL=http://localhost:8018/v1
export OPENAI_API_KEY=any          # byokey 忽略 key 的值
```

**对于 Amp：**

`byokey serve` 会额外监听一个端口 `18018`（可通过 `amp.port` 配置），
专用于 Amp 兼容路由。将 Amp CLI 指向该端口：

```jsonc
// ~/.config/amp/settings.json
{
  "amp.url": "http://localhost:18018"
}
```

或者让 byokey 自动写入：`byokey amp inject`。

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
  login         向 Provider 认证
  logout        删除指定 Provider 的已存储凭据
  status        显示所有 Provider 的认证状态
  accounts      列出某个 Provider 的所有账户
  switch        切换某个 Provider 的活动账户
  amp           Amp 相关工具
  openapi       导出 OpenAPI 规范（JSON 格式）
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

`serve` 还会在 `amp.port`（默认 `18018`）上启动第二个 HTTP 监听器用于 Amp
兼容路由，并在 `~/.byokey/control.sock` 绑定一个 Unix 控制套接字，供
`stop` / `reload` 使用。若进程通过 `systemfd`、`systemd` 或 `launchd`
以预打开套接字的方式启动，将直接复用继承的 fd 而不重新绑定。

**`byokey start`** — 与 `serve` 选项相同。在后台运行服务器，
日志默认写入 `~/.byokey/server.log`。

**`byokey reload`** — 通过控制套接字触发运行中服务器的配置热重载。
无需进程重启，不中断现有连接。

**`byokey login <PROVIDER>`**

为指定 Provider 运行相应的 OAuth 流程。
支持的名称：`claude`、`codex`、`copilot`、`gemini`、`kiro`、
`antigravity`、`qwen`、`kimi`、`iflow`。

```
Options:
      --account <NAME>  账户标识（默认：`default`）
      --db <PATH>       SQLite 数据库路径 [默认: ~/.byokey/tokens.db]
```

**`byokey logout <PROVIDER>`** — 删除指定 Provider 的已存储 Token。

**`byokey status`** — 打印所有已知 Provider 的认证状态。

**`byokey accounts <PROVIDER>`** — 列出某个 Provider 的所有账户。

**`byokey switch <PROVIDER> <ACCOUNT>`** — 切换某个 Provider 的活动账户。

**`byokey service <install|uninstall|start|stop|status>`** — 将 byokey
注册为系统托管服务。macOS 上使用 `launchd`、Linux 上使用 `systemd`、
Windows 上使用 SCM。

**`byokey amp inject`** — 将 `amp.url`（以及 byokey 配置中 `amp.settings`
的额外字段）写入 `~/.config/amp/settings.json`。

</details>

## 配置

创建配置文件（JSON 或 YAML，例如 `~/.config/byokey/settings.json`），通过 `--config` 传入：

```yaml
port: 8018
host: 127.0.0.1

providers:
  # 使用原始 API Key（优先于 OAuth）
  claude:
    api_key: "sk-ant-..."

  # 完全禁用某个 Provider
  gemini:
    enabled: false

  # 仅 OAuth（无 api_key）— 先运行 `byokey login codex`
  codex:
    enabled: true
```

所有字段均可选；未指定的 Provider 默认启用，并使用数据库中存储的 OAuth Token。

## 贡献

请参阅 [CONTRIBUTING.md](CONTRIBUTING.md) 了解构建命令、架构细节和编码规范。

## 许可证

双协议授权，任选其一：[MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE)。
