# 覆盖率审计（Round 4 Phase 2）

> **基线**: master @ cf2cea5 · **日期**: 2026-08-14 · 对应审计计划目标 2
> **工具**: cargo-llvm-cov（本轮首次为仓库建立覆盖率基础设施），产物 `lcov.info`

## 1. 基础设施（本轮新建，长期资产）

```bash
cd <repo>
cargo llvm-cov --workspace --ignore-run-fail --lcov --output-path lcov.info
```

限制说明：不含 doctests；绑定语言测试（TS/Python/Go 等）不在 llvm-cov 统计内（见 §5 L4）；无 per-test 归因（文件级映射）。

## 2. 总量与 per-crate

**Workspace 行覆盖：28,931 / 36,847 = 78.5%**

| crate | 行覆盖 | 评价 |
|---|---|---|
| aimux-stream | 93.8% | 优秀 |
| aimux-core | 89.7% | 优秀 |
| aimux-provider-utils | 87.0% | 良好 |
| aimux-providers | 77.8% | 良好（体量最大） |
| tools/aimux-cli | 58.6% | main 入口未测（bin 部分） |
| aimux-ffi | **44.8%** | 最大短板，见 §4 |
| tools/aimux-replay | 0% | main 未测 |
| scripts/fix_tool | 0% | 开发脚本，可接受 |

## 3. 子流程覆盖矩阵（用户 API → 子流程模块）

| 子流程模块 | 覆盖率 | 驱动它的测试层 |
|---|---|---|
| core/generate.rs（用户 API 入口） | 83.9% | L1 e2e + L2 cassette |
| core/recording.rs / replay.rs / session.rs | 94.3% / 92.9% / 97.5% | core 集成测试（RFC-0023/0024 产物） |
| core/router.rs / moa.rs / openai_output.rs | 96.7% / 93.4% / 83.7% | core 集成测试 |
| utils/http.rs / retry.rs / response.rs / multipart.rs | 88.3% / 82.5% / 99.1% / 95.8% | L1 + L2 + 单元测试 |
| **utils/ws.rs（RFC-0028 新增）** | **63.6%** | 仅 happy-path + 部分 abort/timeout |
| **providers/openai/transcription.rs（RFC-0028 新增）** | **61.0%** | 同上；4e 指出 connect 失败/error 事件/peer close/两类超时零覆盖 |
| openai/convert.rs / google/convert.rs / mistral / cohere / bedrock | 90.8–94.0% | L2 cassette 密集 |
| anthropic/convert.rs | 79.6% | L2，分支残留较多 |
| **ffi/lib.rs（94 个导出）** | **43.1%** | Rust 侧仅 4 个 FFI 测试文件；实际大量覆盖由 L4 绑定测试承担（不在本统计内） |
| ffi/transcription_session.rs（新） | 74.4% | 新增会话测试 |

## 4. 关键发现

1. **RFC-0028 新代码是覆盖率洼地**（ws.rs 63.6%、transcription.rs 61.0%）：与 4e 深审结论互相印证。补测优先级最高（新代码 + 错误路径 + 8 绑定无行为测试）。校验修正：零覆盖路径为 connect 失败 / error 事件 / peer close / chunk-idle+total 超时 / connect 阶段 abort；首块超时与会话中 abort **已有测试**（stream_first_chunk_timeout_fires、stream_abort_mid_session）。
2. **FFI 44.8% 的口径解释**：94 个导出中约半数无 Rust 侧直接测试；部分由 bindings e2e（L4）间接执行，但 swift 绑定无 e2e，kotlin 的 nextPart 超时哨兵是死代码（4e-H）——FFI 层建议补一组 Rust 侧冒烟测试遍历全部导出。
3. **零覆盖文件 24 个**：
   - 15 个本地运行时 provider 手写薄包装（vllm/llamacpp/onnx/mlx/sglang/xinference/localai/oobabooga/litellm_proxy/docker_model_runner/jlama/gaudi/omlx/cybertron/local.rs）——registry 化候选（与架构报告 R3 精简度建议呼应），或至少各补 1 个 smoke 测试
   - core/{model_id, reranking_model, result, util}.rs
   - tools 两个 main + fix_tool
4. **插桩下 3 个测试二进制失败**：anthropic_aws_model_test（`anthropic_aws_sigv4_auth` 稳定 502）/ aws_polly_test / bedrock_model_test（三者共享 `sign_request`）。**机理经独立校验推翻"时间敏感"假设**：真因是 sigv4 签名用 `Url::host_str()` 发出不带端口的 host 头，叠加环境 `http_proxy`（无 no_proxy）时 reqwest 自动代理把回环请求转发给本地代理 → 空体 502；取消代理即通过。这暴露一个真实签名缺陷（非 443 端口 host 不完整），详见 issue #125。初报的 cassette_full_test 为 api-key 认证，与 SigV4 无关，已移出。

## 5. L1–L4 分层现状

| 层 | 定义 | 现状 | 缺口 |
|---|---|---|---|
| L1 协议 E2E | wiremock 往返 | e2e_test.rs 14 用例，**仅 openai + anthropic** | 其余 9 个原生协议零 L1（bedrock/vertex 由 L2 部分弥补，但无错误路径 e2e） |
| L2 转换层 | cassette replay | 32 个 cassette 目录、数百用例，providers 77.8% | anthropic 分支残留；本地 provider 全零 |
| L3 跨绑定 parity | contract-tests | **仅 1 个 fixture（wire-format.json）** | fixture 面过窄，未覆盖流式/工具调用/错误形态 |
| L4 绑定 E2E | 各语言 | node 13 / python 12 / go 10 / java 11 / flutter 14 个测试文件，多数含 e2e | 校验修正：swift **有**功能等价的未命名 e2e（MockHTTPServer.swift + 30 测试）；真实缺口 = **c 绑定零测试** + kotlin nextPart 零覆盖（4e-H） |

## 6. 建议与 ROI 排序

| # | 动作 | 预期收益 | 成本 |
|---|---|---|---|
| 1 | RFC-0028 错误路径补测（connect 失败/error 事件/peer close/双超时/abort） | 消除新代码风险聚集 | 中 |
| 2 | coverage CI job（报告模式：llvm-cov + lcov 上传，不设门禁） | 防退化、趋势可视 | 低 |
| 3 | L1 扩到 google + bedrock + mistral/cohere（复用 e2e_test.rs 模式） | 错误路径端到端保障 | 中 |
| 4 | 15 个本地 provider：registry 化或各补 smoke 测试 | 消除零覆盖块 | 低–中 |
| 5 | contract-tests fixtures 扩面（流式/tool call/错误形态） | 跨绑定 parity 实质化 | 中 |
| 6 | FFI Rust 侧全导出冒烟遍历 | ffi/lib.rs 43%→70%+ | 低 |
| 7 | c 绑定补最小测试 + swift e2e 命名统一；SigV4 host 端口修复（#125） | L4 补齐 / 稳定性 | 低 |
| 8 | 门禁决策：workspace ≥75% + 新增代码 ≥85%（在 1–3 落地后） | 防回归 | — |
