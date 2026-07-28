# chatgpt cassettes — 待实现

这 33 个录像来自 rig (MIT)，记录的是 ChatGPT 网页版的 **Codex Responses API**
（`/backend-api/codex/responses`），不是 OpenAI 官方 API。

## 为什么没有挂载回放测试

`aimux-providers/src/` 下**没有 chatgpt provider 实现**。这个端点的请求/响应格式
与 OpenAI Responses API (`/v1/responses`) 不同：

- 路径是 `/backend-api/codex/responses`（非 `/v1/responses`）
- 请求体用 `input` / `instructions` / `store` / `include` 这套字段
- 鉴权走 ChatGPT 网页 OAuth，不是 `Authorization: Bearer <api-key>`

要回放这些录像，需要先实现一个独立的 `ChatGPTProvider`，不能复用 `OpenAIProvider`。

## 录像内容

全部 33 个录像的请求路径都是 `/backend-api/codex/responses`，覆盖：
- 流式/非流式 completion
- 多轮工具调用（parallel / sequential / nested arguments）
- reasoning session
- prompt cache / store 字段
- unicode 参数、零参数工具调用
- 401 错误响应

## 后续

实现 `ChatGPTProvider` 后，在 `conformance_test.rs` 加 `mod chatgpt_conformance`，
照 bedrock/openai 的模式挂载即可。
