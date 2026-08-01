# aimux-go

Go bindings for [aimux](https://github.com/arcships/aimux) — a unified LLM
access layer for 172+ AI providers, built on a Rust core.

## Install

```bash
go get github.com/arcships/aimux/bindings/go
```

The binding links `libaimux_ffi.a` (static archive) via cgo. The archive is
downloaded per-platform from the [GitHub Releases](https://github.com/arcships/aimux/releases)
page by the `go generate` script:

```bash
go generate ./...
```

You need a C toolchain (gcc/clang) but **not** a Rust toolchain.

## Quick start

```go
package main

import (
    "fmt"
    "log"

    aimux "github.com/arcships/aimux/bindings/go"
)

func main() {
    p, err := aimux.NewOpenAI("sk-...", "gpt-4o")
    if err != nil {
        log.Fatal(err)
    }
    defer p.Close()

    text, err := p.GenerateText("Explain Rust ownership in one sentence.", "")
    if err != nil {
        log.Fatal(err)
    }
    fmt.Println(text)
}
```

## Features

- Typed API for text, streaming, embeddings, tools, images, speech, and more
- One API for 172+ providers (OpenAI, Anthropic, Google, Bedrock, …)
- Single static binary — the Rust core is linked in, no runtime deps

See the [API docs](https://github.com/arcships/aimux/blob/master/docs/API.md)
for the full API surface.

## License

MIT
