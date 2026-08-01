# aimux-go

Go bindings for [aimux](https://github.com/arcships/aimux) — a unified LLM
access layer for 172+ AI providers, built on a Rust core.

## Install

```bash
go get github.com/arcships/aimux/bindings/go
```

The binding links `libaimux_ffi.a` (static archive) via cgo. The archive is
built from the Rust core with cargo — **native build for your own platform,
no cross-compilation involved**. You need the [Rust toolchain](https://rustup.rs)
(one-time install):

```bash
cd <your-module>            # or anywhere inside a Go module that imports the binding
go generate ./...           # builds target/release/libaimux_ffi.a (first build: a few minutes)
go build ./...              # subsequent builds are fast (incremental)
```

> The cgo `LDFLAGS` point at `<repo>/target/release/libaimux_ffi.a` — the
> standard location when the build runs from a checkout of this repository.

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
