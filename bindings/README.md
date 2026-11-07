# aimux bindings

Multi-language bindings for aimux. All bindings share the same Rust core (aimux-core + aimux-providers), accessed via two paths:

> **0.6.0.** Every binding uses ABI 1 / ops 1.0. Provider-specific
> constructors and the 0.5 error getters are removed. See the
> [0.6 upgrade guide](../docs/upgrading-to-0.6.md) and each binding's README.

```
                  ┌─ Native path ──→ aimux-core + aimux-providers (directly maps Rust types + async)
Binding layer ────┤
                  └─ C ABI path ──→ aimux-ffi (handles + JSON results + aimux_error_t * returned error)
```

| Binding | Path | Tool | Status | Directory |
|------|------|------|------|------|
| **Node.js** | Native | napi-rs v3 | ✅ Available | [node/](node/) |
| **Python** | Native | PyO3 + maturin | ✅ Available | [python/](python/) |
| **Swift** | C ABI | Swift Package (module.modulemap) | ✅ Available | [swift/](swift/) |
| **Kotlin** | C ABI | JNA + coroutines | ✅ Available | [kotlin/](kotlin/) |
| **Flutter** | C ABI | dart:ffi handwritten | ✅ Available | [flutter/](flutter/) |
| **Go** | C ABI | cgo (static linking `libaimux_ffi.a`) | ✅ Available | [go/](go/) |
| **Java** | C ABI | JNA | ✅ Available | [java/](java/) |
| **C / C++** | C ABI | direct link to `aimux.h` | ✅ Available | [c/](c/) |

## Build

### Node.js
```bash
cd bindings/node
npm install
npx napi build --platform    # debug
npx napi build --platform --release  # release
npm test
```

### Python
```bash
cd bindings/python
python -m venv .venv && source .venv/bin/activate
pip install maturin pytest
maturin develop
pytest tests/ -v
```

### Swift
```bash
# First build the aimux-ffi dylib
cargo build -p aimux-ffi --release
cp target/release/libaimux_ffi.dylib /usr/local/lib/

cd bindings/swift
swift build
swift test
```

### Kotlin
```bash
# First build the aimux-ffi .so
cargo build -p aimux-ffi --release
mkdir -p bindings/kotlin/src/main/resources
cp target/release/libaimux_ffi.so bindings/kotlin/src/main/resources/

cd bindings/kotlin
export LD_LIBRARY_PATH="$PWD/src/main/resources:$LD_LIBRARY_PATH"
gradle test
```

### Flutter
```bash
# First build the aimux-ffi .so
cargo build -p aimux-ffi --release

cd bindings/flutter
dart pub get
LD_LIBRARY_PATH=../../target/release dart test
```

### C / C++
```bash
# First build aimux-ffi
cargo build -p aimux-ffi --release

# C
gcc -o example bindings/c/example.c \
    -I aimux-ffi/include -L target/release -laimux_ffi -lpthread -ldl -lm

# C++
g++ -std=c++17 -o example_cpp bindings/c/example.cpp \
    -I aimux-ffi/include -L target/release -laimux_ffi -lpthread -ldl -lm
```

### Go
```bash
# First build the static archive with the profile the releases use
# (release without LTO; a fat-LTO archive is twice the size, RFC-0043 §5.1)
cargo build -p aimux-ffi --profile staticlib-release

cd bindings/go
go test ./...
```

The Go binding statically links `libaimux_ffi.a` via cgo; the artifact is a **single binary** (the Rust core is compiled into the executable, no need to distribute `.so`). See [RFC-0011](../rfc/0011-golang-bindings.md) for details.

### Java
```bash
# First build the aimux-ffi .so
cargo build -p aimux-ffi --release

cd bindings/java
export JAVA_HOME=...   # JDK 17 (any 9+ works; bytecode targets Java 8)
export LD_LIBRARY_PATH="$(pwd)/../../target/release:${LD_LIBRARY_PATH}"
gradle test
```

The Java binding uses JNA (no native toolchain needed at build time). The
native `aimux_ffi` library comes from the matching GitHub Release and is loaded
from the JNA search path. See
[RFC-0013](../rfc/0013-java-bindings.md) for details.
