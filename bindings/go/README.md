# aimux-go

Go bindings for [aimux](https://github.com/arcships/aimux) — a unified LLM
access layer for 172+ AI providers, built on a Rust core.

## Install

```bash
go get github.com/arcships/aimux/bindings/go
go generate github.com/arcships/aimux/bindings/go   # downloads libaimux_ffi.a for your platform
go build ./...
```

The binding links `libaimux_ffi.a` (static archive) via cgo. Every release
builds the archive for 4 platforms (linux x64, macOS x64/arm64, Windows x64)
on CI and ships them on the [GitHub Releases](https://github.com/arcships/aimux/releases)
page. `go generate` fetches the right one for your machine — a few MB, no
Rust toolchain, no compilation. Pin a specific version with
`AIMUX_FFI_VERSION=v0.1.0 go generate github.com/arcships/aimux/bindings/go`.

> Platforms without a prebuilt archive (e.g. FreeBSD, RISC-V): clone the
> repo and run `cargo build -p aimux-ffi --release` locally, then build
> your Go code from that checkout.
> The archive lands in `<module-cache>/.../aimux/target/release/`; a `go
> clean -modcache` or Go upgrade removes it — re-run `go generate`.

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
