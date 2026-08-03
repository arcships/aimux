# aimux (Python)

Unified LLM service layer for Python — one API for **325 AI providers**
(OpenAI, Anthropic, Google, Bedrock, Vertex, DeepSeek, …), powered by a Rust
core.

## Install

```bash
pip install arcships-aimux
```

Pre-built wheels for Linux, macOS, and Windows (Python ≥ 3.8). No Rust
toolchain needed.

## Quick start

```python
from aimux import generate_text, openai

model = openai("sk-...", "gpt-4o")
result = generate_text(model, "Explain Rust ownership in one sentence.")
print(result["text"])
```

## Features

- **Unified interface** — switch providers by changing one constructor
- **Streaming** — token-by-token output with cancellation
- **Full modality coverage** — text, embeddings, images, speech,
  transcription, video, reranking, files
- **Fast** — Rust core, zero GC pauses; benchmarked faster than the official
  OpenAI SDK in [docs/PERF-RESULTS.md](https://github.com/arcships/aimux/blob/master/docs/PERF-RESULTS.md)

## Documentation

- [Full API reference](https://github.com/arcships/aimux/blob/master/docs/API.md)
- [Project overview](https://github.com/arcships/aimux/blob/master/docs/PROJECT-OVERVIEW.md)

## License

MIT
