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
      <sup>设备码</sup><br>
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
      <sup>设备码</sup><br>
      <sub>kiro-default</sub>
    </td>
  </tr>
</table>

> **即将到来** — 认证已实现，执行器开发中：<br>
> Antigravity (Google) · Qwen (Alibaba) · Kimi (Moonshot) · iFlow (Z.ai)

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
cd BYOK
cargo install --path .
```

> **环境要求：** Rust 1.85+（edition 2024），以及用于 SQLite 的 C 编译器。

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

```jsonc
// ~/.amp/settings.json
{
  "amp.url": "http://localhost:8018/amp"
}
```

## CLI 参考

```
byokey <COMMAND>

Commands:
  serve    启动代理服务器
  login    向 Provider 认证
  logout   删除指定 Provider 的已存储凭据
  status   显示所有 Provider 的认证状态
  help     打印帮助信息
```

<details>
<summary><b>命令详情</b></summary>
<br>

**`byokey serve`**

```
Options:
  -c, --config <FILE>   YAML 配置文件 [默认: 无]
  -p, --port <PORT>     监听端口     [默认: 8018]
      --host <HOST>     监听地址     [默认: 127.0.0.1]
      --db <PATH>       SQLite 数据库路径 [默认: ~/.byokey/tokens.db]
```

**`byokey login <PROVIDER>`**

为指定 Provider 运行相应的 OAuth 流程。
支持的名称：`claude`、`codex`、`copilot`、`gemini`、`kiro`、
`antigravity`、`qwen`、`kimi`、`iflow`。

```
Options:
      --db <PATH>   SQLite 数据库路径 [默认: ~/.byokey/tokens.db]
```

**`byokey logout <PROVIDER>`** — 删除指定 Provider 的已存储 Token。

**`byokey status`** — 打印所有已知 Provider 的认证状态。

</details>

## 配置

创建 YAML 文件（例如 `~/.byokey/config.yaml`），通过 `--config` 传入：

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
