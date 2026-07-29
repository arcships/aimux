# aimux bindings

aimux 的多语言绑定。所有绑定共享同一个 Rust 核心（aimux-core + aimux-providers），通过两条路径接入：

```
           ┌─ 原生路径 ──→ aimux-core + aimux-providers (直接映射 Rust 类型 + async)
绑定层 ────┤
           └─ C ABI 路径 ──→ aimux-ffi (opaque handle + JSON + push callback)
```

| 绑定 | 路径 | 工具 | 状态 | 目录 |
|------|------|------|------|------|
| **Node.js** | 原生 | napi-rs v3 | ✅ 可用 | [node/](node/) |
| **Python** | 原生 | PyO3 + maturin | ✅ 可用 | [python/](python/) |
| **Swift** | C ABI | Swift Package (module.modulemap) | ✅ PoC | [swift/](swift/) |
| **Kotlin** | C ABI | JNA | ✅ PoC | [kotlin/](kotlin/) |
| **Flutter** | C ABI | dart:ffi 手写 | ✅ PoC | [flutter/](flutter/) |
| **C / C++** | C ABI | 直接链接 aimux-ffi.h | ✅ PoC | [c/](c/) |
| **Kotlin** | C ABI | JNA | ✅ PoC | [kotlin/](kotlin/) |
| **Flutter** | 原生 | flutter_rust_bridge | ✅ PoC | [flutter/](flutter/) |
| **C / C++** | C ABI | 直接链接 aimux-ffi.h | ✅ PoC | [c/](c/) |

## 构建

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
# 先构建 aimux-ffi dylib
cargo build -p aimux-ffi --release
cp target/release/libaimux_ffi.dylib /usr/local/lib/

cd bindings/swift
swift build
swift test
```

### Kotlin
```bash
# 先构建 aimux-ffi .so
cargo build -p aimux-ffi --release
mkdir -p bindings/kotlin/src/main/resources
cp target/release/libaimux_ffi.so bindings/kotlin/src/main/resources/

cd bindings/kotlin
export LD_LIBRARY_PATH="$PWD/src/main/resources:$LD_LIBRARY_PATH"
gradle test
```

### Flutter
```bash
# 先构建 aimux-ffi .so
cargo build -p aimux-ffi --release

cd bindings/flutter
dart pub get
LD_LIBRARY_PATH=../../target/release dart test
```

### C / C++
```bash
# 先构建 aimux-ffi
cargo build -p aimux-ffi --release

# C
gcc -o example bindings/c/example.c \
    -I aimux-ffi -L target/release -laimux_ffi -lpthread -ldl -lm

# C++
g++ -std=c++17 -o example_cpp bindings/c/example.cpp \
    -I aimux-ffi -L target/release -laimux_ffi -lpthread -ldl -lm
```

## 契约测试

共享 JSON 夹具驱动所有语言，确保 wire format 一致：

```bash
# Rust 端
cargo test -p aimux-core --test contract_test

# Node 端
node --experimental-strip-types contract-tests/run-node.ts
```

夹具位于 [contract-tests/fixtures/wire-format.json](../contract-tests/fixtures/wire-format.json)。
