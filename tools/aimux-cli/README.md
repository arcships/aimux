# aimux-cli — 缓存探测 client(RFC-0025)

基于 aimux 构建的调试二进制(`aimux`),第一版做缓存探测业务(三层拆分②):
消费 RFC-0015 的探测数据(离线 jsonl / 在线探测),输出人可读报告。
**不实现探测算法**(判定/LCP/存储全在 `aimux-core`),不内置告警决策。

## 构建与运行

```bash
cargo run -p aimux-cli -- cache-probe <SUBCOMMAND>
# 或安装后直接:
#   cargo install --path tools/aimux-cli
#   aimux cache-probe <SUBCOMMAND>
```

## 子命令

### offline — 离线审计

读 `RingTraceStore::export_jsonl` 输出的 TraceRecord jsonl,输出审计统计。

```bash
aimux cache-probe offline --file trace.jsonl
aimux cache-probe offline --file trace.jsonl --provider deepseek
aimux cache-probe offline --file trace.jsonl --session sess-1
aimux cache-probe offline --file trace.jsonl --format json   # 给脚本
```

输出:每 (provider, model) 一组 — 请求数、声称命中率(reported)、
客户端 LCP 上界命中率(client upper bound)、verdict 分布、TTFT 分位。
声称命中率 > 客户端 LCP 上界的记录以 `SuspectOverclaim` 标记并高亮提示。

### session — 会话级诊断

读 jsonl,输出指定 session 的链级视图(记录序列 + 前缀稳定性 + 断点)。

```bash
aimux cache-probe session --file trace.jsonl --session sess-1
```

`prefix stability < 0.5` 时提示:前缀持续变化,超过共享前缀的命中可疑。

### provider — 在线探测缓存能力

直接调 provider 发测试请求(固定模板 + 每轮追加对话内容),挂
`TraceLayer` + 内置 auditor,报告该 provider 的缓存行为。

```bash
aimux cache-probe provider --provider deepseek --model deepseek-chat \
    --api-key env:DEEPSEEK_API_KEY
aimux cache-probe provider --provider openai --model gpt-4o \
    --api-key env:OPENAI_API_KEY --max-requests 4
```

**⚠ 消耗真实 API 费用**:默认 4 次请求(约 8-12K token),用 `--max-requests` 限制,
`--prompt` 可覆盖默认 system 模板(建议 ≥1024 token 以触发多数 provider 缓存门槛)。

支持的原生 provider:`openai` / `anthropic` / `mistral` / `xai` / `cohere` / `google`;
其余名字走注册表(compat,如 `deepseek` / `groq` / `moonshotai` 等)。
`azure` / `bedrock` / `vertex` 需额外参数,第一版不直接支持(可用 compat 镜像)。

判定口径:客户端 LCP 是钳制服务端声称命中数的标尺——第二轮起应观察到
`cache_read > 0`(前缀已建立);首轮即命中、或声称命中超过客户端前缀,标为可疑。
注意集群部署(多机路由)下"前缀稳定但报 0"是正常现象,不是异常。
