# Research Plan: LLM Request Trace & Cache-Hit Audit for aimux

> Core question: 为 aimux(统一 LLM 访问层)设计 trace + 统计子系统,利用对连续 agent model call 的原始请求体字符串级对比,审计各 provider 缓存命中率上报的真实性,并提供诊断能力。
> min_rounds: 8
> Date: 2026-08-01

## Dimensions

1. **D1 缓存机制原理** — KV cache / prefix caching 的物理机制、token 与 block 粒度、命中条件(相同 provider+model+序列化前缀)、TTL 与淘汰策略、cache key 组成。这是判断"什么能被缓存"的地基。
2. **D2 各家 provider API 差异** — OpenAI / Anthropic / Gemini / DeepSeek / Azure / Bedrock / Cohere / Mistral / xAI / OpenRouter / 网关(LiteLLM、Portkey)/ 自托管(vLLM、SGLang、Ollama)的缓存语法(显式 cache_control vs 自动)、usage 上报字段、最低缓存 token 门槛、流式 usage 上报方式。
3. **D3 usage 上报可信度** — 已知的缓存命中上报不准/掺水的案例与原因(流式 usage 丢失、网关改写、cache write 与 no_cache 混淆、OpenAI 缓存不保证命中、模型版本更新导致缓存失效)、业界已有的审计/监控方案。
4. **D4 agent loop 连续调用识别** — 会话模型、连续同模型调用检测、前缀连续性(用户消息变化但 system+history 稳定)、并行请求的缓存竞争、多模型切换。真实世界案例(如 TradingAgents 0% 命中率)。
5. **D5 字符串级对比与指纹** — 请求体 canonicalization(JSON key 顺序、空白、aimux 协议转换后的 provider 格式)、LCP 算法、字符级 vs token 级估算、滚动窗口、敏感数据脱敏、指纹存储成本控制。
6. **D6 aimux 集成设计** — 现有 `request_body` / `Usage`(cache_read/cache_write/no_cache)/ `response_headers` 字段、流式 usage 解析现状、wrapper 模式(Vercel AI SDK telemetry 类比)、新 crate 划分、trace 数据模型、统计 API、性能预算(aimux 以性能为卖点)。

## Completion criteria

- [ ] 每个维度被 ≥2 个 agent 从不同角度覆盖,关键结论可追溯到主源
- [ ] ≥12 家 provider 的缓存机制/上报字段/最低门槛/已知缺陷成表
- [ ] 审计算法完整设计:canonicalization → LCP → 可缓存上限估计 → 与上报值对比 → 判定规则(掺水/合理/未知)与置信度
- [ ] 设计融入 aimux 架构(新 crate + wrapper,不破坏现有 `dyn LanguageModel` 接口),有性能预算
- [ ] Verifier PASS(≥8 轮)

## Scope

- **In**: 缓存命中审计方法论、trace 数据模型、agent loop 识别算法、aimux 集成设计、核心原型
- **Out**: 本任务不实现各 provider 显式缓存创建 API(cache_control 写入)、不做计费/成本追踪、不做完整 FFI 绑定集成
