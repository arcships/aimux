# Examples

Three small Rust programs against the 0.6 API. They are a workspace member so `cargo build -p aimux-examples` typechecks them. They perform a live HTTP call when you run them. Nothing here is executed by `cargo test`.

From the repository root:

```bash
# Chat Completions. Reads OPENAI_API_KEY when the request is sent.
cargo run -p aimux-examples --bin chat

# Streaming text deltas from the same OpenAI chat model.
cargo run -p aimux-examples --bin stream

# create_provider(name, PresetSettings) — vendor package or registry preset.
# Defaults: deepseek / deepseek-chat, which reads DEEPSEEK_API_KEY.
cargo run -p aimux-examples --bin switch-provider
cargo run -p aimux-examples --bin switch-provider -- alibaba qwen-plus
cargo run -p aimux-examples --bin switch-provider -- moonshotai moonshot-v1-8k
cargo run -p aimux-examples --bin switch-provider -- zhipuai_coding_plan glm-4.5-air
```

Those pairs are what this repository's tests pass in: `deepseek` / `deepseek-chat`, `alibaba` / `qwen-plus`, `moonshotai` / `moonshot-v1-8k`, `zhipuai_coding_plan` / `glm-4.5-air`. Whether an account still serves that model id is the provider's catalogue. `kimi`, `bigmodel`, and `zhipu_v4` are valid package names; pick the model id from the provider.

`openai().chat` is Chat Completions. `Provider::language_model` on the OpenAI package selects the Responses API. `switch-provider` uses `language_model`, which for `deepseek` and the DashScope / Zhipu / Kimi presets is the chat model those packages expose.

Node, Python, and the other bindings use `provider.create` and `provider.model` instead of these Rust constructors. See the language guides under [docs/api/](../docs/api/).
