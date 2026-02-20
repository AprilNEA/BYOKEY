# byok — 使用你自己的订阅

将 AI **订阅账号**转换为 **OpenAI 兼容 API 端点**的本地代理，
让任何兼容 OpenAI 的工具（Amp、Cursor、Continue 等）无需付费 API Key 即可使用。

```
Claude Pro  ─┐
OpenAI Plus ─┤  byok serve  ─►  http://localhost:8018/v1/chat/completions
Copilot     ─┘                  (OpenAI 兼容，支持流式输出)
```

## 功能特性

- **OpenAI 兼容 API** — 直接替换，只需修改 base URL
- **OAuth 登录流程** — 自动处理 PKCE、设备码、授权码等流程
- **SQLite Token 存储** — Token 持久化，重启后依然有效，存储于 `~/.byok/tokens.db`
- **API Key 直通** — 更喜欢原始 API Key？在配置文件中设置即可
- **Amp CLI 兼容** — 内置 `/amp/*` 路由，开箱即用支持 [Amp](https://ampcode.com)
- **YAML 配置** — 热重载友好，所有选项均有合理默认值

## 支持的 Provider

| Provider | 登录方式 | 可用模型 |
|---|---|---|
| **Claude** (Anthropic) | PKCE 浏览器 | claude-opus-4-6, claude-sonnet-4-5, … |
| **Codex** (OpenAI) | PKCE 浏览器 | o4-mini, o3, gpt-4o, gpt-4o-mini |
| **Copilot** (GitHub) | 设备码 | gpt-4o, claude-3.5-sonnet, o3-mini |
| **Gemini** (Google) | PKCE 浏览器 | gemini-2.0-flash, gemini-1.5-pro, … |
| **Kiro** (AWS) | 设备码 | kiro-default |
| **Antigravity** (Google) | PKCE 浏览器 | — *(认证已就绪，执行器开发中)* |
| **Qwen** (Alibaba) | 设备码 + PKCE | — *(认证已就绪，执行器开发中)* |
| **Kimi** (Moonshot) | 设备码 | — *(认证已就绪，执行器开发中)* |
| **iFlow** (Z.ai / GLM) | 授权码 | — *(认证已就绪，执行器开发中)* |

## 安装

### 从 crates.io 安装

```sh
cargo install byok
```

### 从源码安装

```sh
git clone https://github.com/AprilNEA/BYOKEY
cd BYOK
cargo install --path .
```

**环境要求：** Rust 1.85+（edition 2024），以及用于 SQLite 的 C 编译器。

## 快速开始

```sh
# 1. 认证（会打开浏览器或显示设备码）
byok login claude
byok login codex
byok login copilot

# 2. 启动代理
byok serve

# 3. 将工具指向代理地址
export OPENAI_BASE_URL=http://localhost:8018/v1
export OPENAI_API_KEY=any          # byok 忽略 key 的值
```

对于 Amp：

```jsonc
// ~/.amp/settings.json（或 Amp 读取配置的路径）
{
  "amp.url": "http://localhost:8018/amp"
}
```

## CLI 参考

```
byok <COMMAND>

Commands:
  serve    启动代理服务器
  login    向 Provider 认证
  logout   删除指定 Provider 的已存储凭据
  status   显示所有 Provider 的认证状态
  help     打印帮助信息
```

### `byok serve`

```
Options:
  -c, --config <FILE>   YAML 配置文件 [默认: 无]
  -p, --port <PORT>     监听端口     [默认: 8018]
      --host <HOST>     监听地址     [默认: 127.0.0.1]
      --db <PATH>       SQLite 数据库路径 [默认: ~/.byok/tokens.db]
```

### `byok login <PROVIDER>`

为指定 Provider 运行相应的 OAuth 流程。
支持的名称：`claude`、`codex`、`copilot`、`gemini`、`kiro`、
`antigravity`、`qwen`、`kimi`、`iflow`。

```
Options:
      --db <PATH>   SQLite 数据库路径 [默认: ~/.byok/tokens.db]
```

### `byok logout <PROVIDER>`

删除指定 Provider 的已存储 Token。

### `byok status`

打印所有已知 Provider 的认证状态。

## 配置

创建 YAML 文件（例如 `~/.byok/config.yaml`），通过 `--config` 传入：

```yaml
# ~/.byok/config.yaml
port: 8018
host: 127.0.0.1

providers:
  # 使用原始 API Key（优先于 OAuth）
  claude:
    api_key: "sk-ant-..."

  # 完全禁用某个 Provider
  gemini:
    enabled: false

  # 仅 OAuth（无 api_key）— 先运行 `byok login codex`
  codex:
    enabled: true
```

所有字段均可选；未指定的 Provider 默认启用，并使用数据库中存储的 OAuth Token。

## API 端点

| 方法 | 路径 | 说明 |
|---|---|---|
| `POST` | `/v1/chat/completions` | OpenAI 兼容对话（支持流式） |
| `GET` | `/v1/models` | 列出已启用的模型 |
| `GET` | `/amp/v1/login` | Amp 登录重定向 |
| `ANY` | `/amp/v0/management/{*path}` | Amp 管理 API 代理 |
| `POST` | `/amp/v1/chat/completions` | Amp 兼容对话 |

请求体中的 `model` 字段决定使用哪个 Provider。

## 工作原理

```
客户端请求
    │
    ▼
byok-proxy  (axum HTTP 服务器)
    │  根据 model 解析 → provider
    ▼
byok-provider  (每个 provider 的执行器)
    │  获取 OAuth Token（或 api_key）
    ▼
byok-auth  (AuthManager + OAuth 流程)
    │
    ▼
上游 API  (Anthropic / OpenAI / Google / …)
    │  将响应转换为 OpenAI 格式
    ▼
客户端响应  (JSON 或 SSE 流)
```

### Workspace Crate 说明

| Crate | 说明 |
|---|---|
| `byok-types` | 共享类型、Trait、错误定义 |
| `byok-config` | YAML 配置 + 文件监听 |
| `byok-store` | SQLite（及内存）Token 持久化 |
| `byok-auth` | 各 Provider 的 OAuth 2.0 登录流程 |
| `byok-translate` | 请求/响应格式转换 |
| `byok-provider` | Provider 执行器与模型注册表 |
| `byok-proxy` | axum HTTP 服务器与路由 |

## 构建与测试

```sh
# 构建全部
cargo build --workspace

# 运行全部测试（173 个测试，无需网络）
cargo test --workspace

# Lint
cargo clippy --workspace --all-targets -- -D warnings

# 格式化
cargo fmt --all
```

## 许可证

双协议授权，任选其一：

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)
