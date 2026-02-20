# Contributing to byokey

## 环境要求

- Rust stable（推荐通过 [rustup](https://rustup.rs/) 安装）
- SQLite 3（系统级，macOS 自带）

## 常用命令

```bash
# 全量编译
cargo build --workspace

# 运行测试
cargo test --workspace
cargo test -p byokey-types          # 单 crate 测试（-p 使用 package 名）
cargo test -p byokey-translate      # 纯逻辑测试（无 IO，最快）

# Lint（CI 以 -D warnings 运行，本地同理）
cargo clippy --workspace -- -D warnings

# 格式化
cargo fmt --all

# 启动 CLI 服务器
cargo run -- serve --config config.yaml
```

## 编码规范

- `unsafe` 代码在 workspace 级别被禁止（`forbid`）
- 启用 `clippy::pedantic`，提交前确保零 warning
- edition 2024
- 所有 async trait 使用 `async-trait` 宏
- 错误类型：跨 crate 边界用 `ByokError`，crate 内部用 `anyhow`

## Workspace 分层规则

严格 DAG，禁止跨层反向依赖：

```
Layer 0  byokey-types     — 核心类型 & trait，零工作区内依赖
Layer 1  byokey-config    — 配置解析
         byokey-store     — token 持久化
Layer 2  byokey-auth      — OAuth 流程（不依赖 translate / provider）
         byokey-translate — 格式转换（纯函数，不依赖 auth）
Layer 3  byokey-provider  — Provider Executor
Layer 4  byokey-proxy     — axum HTTP 服务器
```

## 提交规范

使用 [Conventional Commits](https://www.conventionalcommits.org/)：

```
feat(auth): add Kiro device code flow
fix(proxy): handle empty SSE chunk
refactor(translate): simplify gemini response parser
```
