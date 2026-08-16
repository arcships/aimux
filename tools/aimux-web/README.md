# aimux-web — Web 控制台(RFC-0029)

浏览器端的 model call 验证与 trace 可视化工具。后端(axum)提供 `/api/*`
端点并托管前端 SPA;每次调用自动落 `Recording`(RFC-0023)、`TraceRecord`
(RFC-0015)并归组到会话(RFC-0024)。

## 安装 / 获取

**方式 1：GitHub Releases 预编译二进制（推荐，普通用户）**

每个版本在 GitHub Releases 发布**自包含单文件**（前端已内嵌进二进制，
无需安装 Rust / Node）：
- `aimux-web-linux-x64`（Ubuntu 等）
- `aimux-web-macos-arm64`（Apple Silicon）
- `aimux-web-windows-x64.exe`

下载后直接运行，自动打开浏览器：

```bash
./aimux-web            # 自动开 http://127.0.0.1:<port>
./aimux-web --port 8787 --no-open   # 指定端口 / 不开浏览器
```

同一 Release 也附 `aimux-cli-*` / `aimux-replay-*`（缓存探测与请求回放 CLI）。

**方式 2：从源码运行（开发者 / 贡献者）**

```bash
# 1. 构建前端(首次需要)
cd tools/aimux-web/web
npm install
npm run build

# 2. 启动服务(自动打开浏览器)
cargo run -p aimux-web
# 指定端口 / 不开浏览器
cargo run -p aimux-web -- --port 8787 --no-open
```

### 内嵌前端构建

发布二进制由 CI 用 `--features embed-frontend` 构建：`web/dist` 在编译期
被 `rust-embed` 打进二进制（需要先 `npm run build`），产物完全自包含。
本地 dev 构建默认从磁盘读 `web/dist`（前端可独立热更新）。

## 快速开始

```bash
# 1. 构建前端(首次需要)
cd tools/aimux-web/web
npm install
npm run build

# 2. 启动服务(自动打开浏览器)
cargo run -p aimux-web
# 指定端口 / 不开浏览器
cargo run -p aimux-web -- --port 8787 --no-open
```

打开后使用 Playground / Agent 页跑一次调用,Traces 页即可看到完整录制。

## 开发模式(前端热更新)

```bash
# 终端 1:后端(固定端口,不开浏览器)
cargo run -p aimux-web -- --port 8787 --no-open

# 终端 2:前端 dev server(代理 /api → 8787)
cd tools/aimux-web/web
npm run dev
# 浏览器打开 http://localhost:5173
```

## 页面

| 页面 | 用途 |
|---|---|
| Playground | 单次 model call 验证:provider/model 选择、参数、流式输出、tool 调用展示 |
| Agent | 浏览器 JS 驱动的 agent loop(前端引擎 `src/agent/engine.ts`),多步共享 session_id |
| Traces | 录制列表 + 三层详情(输入 / Provider / HTTP exchanges)+ 会话瀑布图 + jsonl 导入导出 |
| Sessions | 会话归组与调用链 |
| Replay | 请求回放(重发真实 API)+ 新旧输出 diff + mock 模式加载 |
| Cache | 在线 prefix 缓存探测(dry-run 免费) |

## 关键设计

- **Agent loop 在前端**(RFC-0029 决策 D1):每次 model call 走 `POST /api/calls`
  (SSE),循环逻辑、tool 执行桥(`POST /api/tools/:name`)在浏览器驱动。
- **多步关联**:同一 `session_id` + 后端 `SessionStore` 自动分配 step,
  Traces 页按会话聚合瀑布图。
- **凭据安全**:API key 只接受 `env:VAR` 引用或由 provider 注册表 env var 自动读取;
  明文 key 一律拒绝;录制脱敏复用 core 规则。
- **Mock 模式**:`POST /api/mock/load` 加载录制 jsonl(与 `aimux-replay` 格式互通),
  之后 `mock: true` 的调用走 `MockReplayModel`,零成本离线验证。

## 无真实 API 的验收/演示

```bash
# 生成一条 mock 录制 fixture
cargo run -p aimux-web --example gen_fixture /tmp/fixture.jsonl
# 启动后:POST /api/mock/load 上传 fixture.jsonl,再在 Playground 勾选 mock 模式发送即可
```

## 目录

```
tools/aimux-web/
├── src/          Rust 后端(axum):main/state/wire/agents/model_builder + api/*
├── examples/     gen_fixture / gen_demo_fixtures:mock 录制生成器
└── web/          Vue 3 + shadcn 风格 + Tailwind + markstream-vue
    ├── src/agent/engine.ts   前端 agent loop 引擎
    ├── src/api/client.ts     SSE/端点封装
    └── src/types/            d.ts(core 类型来自 ts-rs,见下)
```

## 前端类型来源

- **core 类型**(`Recording`/`StreamPart`/`TraceRecord`/`SessionView` 等):ts-rs 从
  `aimux-core` 生成到 `bindings/node/src/types/`(`scripts/gen_ts_types.py` 管线),
  **拷贝**到 `web/src/types/` 供前端使用。改 core 类型后:跑 `scripts/gen_ts_types.py`
  并同步拷贝。
- **wire 类型**(`Wire*.ts`):是 aimux-web 的 API 契约,**手动维护**在
  `web/src/types/`(不 ts-rs 导出,避免污染 node bindings 的导出目录)。
  改 `src/wire.rs` 时同步更新对应的 `web/src/types/Wire*.ts`。
