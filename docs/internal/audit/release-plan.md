# aimux 全语言库发布计划

> 统一发布工程规划：7 个 Rust crate + 7 种语言绑定，覆盖 crates.io / npm / PyPI / SPM / Maven Central / pub.dev / Go module proxy 七大生态。
>
> 状态：规划稿（2026-07-31 更新，新增 Go 绑定章节）。基于对 `Cargo.toml`、`bindings/*/`、`.github/workflows/ci.yml`、`rfc/0001`、`rfc/0008`、`rfc/0011`、`docs/PROJECT-OVERVIEW.md`、`docs/audit-002`、`docs/audit-003` 的交叉核对得出。

---

## 一、发布目标

把 aimux 从"代码就绪"推进到"可被各语言开发者一条命令安装使用"：

```bash
cargo add aimux-core              # Rust
npm install @aimux/node            # Node.js
pip install aimux                  # Python
# SPM: .package(url: "...", from: "0.1.0")   # Swift
# Maven: implementation("io.aimux:aimux:0.1.0") # Kotlin/Android
go get github.com/arcships/aimux/bindings/go@v0.1.0  # Go
dart pub add aimux                 # Flutter/Dart
```

核心原则：

1. **单一 Rust 核心，多语言分发**——7 个 Rust crate 编译出二进制，7 种语言包只是它的外壳。
2. **跨平台二进制开箱即用**——用户安装后不需装 Rust 工具链、不需手动编译（Go 例外：cgo 静态链接单 binary，但编译期需 `libaimux_ffi.a`）。
3. **版本号统一对齐**——所有 crate 与语言包共享一个版本号，同一次发布。
4. **CI/CD 全自动发布**——打 git tag 触发，人工只做 review 与密钥托管（Go 最简：打 tag 即发布，无需 secret）。

---

## 二、当前状态评估

### 2.1 代码成熟度（已审计通过）

依据 `docs/audit-002`（结构化测试）与 `docs/audit-003`（类型化封装）：5 个语言绑定的**代码层均已达到可发布质量门槛**，无 PoC/半成品/不可用绑定，无假绿测试。

| 绑定 | 路径 | 接入方式 | e2e 测试 | wrapper 测试 | 代码成熟度 |
|------|------|------|:---:|:---:|:---:|
| Node.js | `bindings/node/` | 原生 napi-rs | 4/4 | 7/7 | ⭐⭐⭐⭐⭐ |
| Python | `bindings/python/` | 原生 PyO3 | 4/4 | 11/11 | ⭐⭐⭐⭐ |
| Swift | `bindings/swift/` | C ABI | 9/9 | 9/9 | ⭐⭐⭐⭐⭐ |
| Kotlin | `bindings/kotlin/` | C ABI/JNA | 4/4 | 4/4 | ⭐⭐⭐⭐ |
| Flutter | `bindings/flutter/` | C ABI/dart:ffi | 4/4 | 5/5 | ⭐⭐⭐ |
| Go | `bindings/go/` | C ABI/cgo 静态链接 | 6/6 | 31 个测试全绿 | ⭐⭐⭐⭐ |
| C ABI | `aimux-ffi/` | C ABI 基座 | — | — | ⭐⭐⭐⭐⭐ |

> Go 绑定是 2026-07-31 新增的第七种语言绑定，RFC-0011 已落地 PoC（cgo 静态链接 `libaimux_ffi.a`，单 binary 7.5MB，31 个测试通过）。代码层与其它 C ABI 绑定同级，但发布机制独特——见第十一章。

### 2.2 发布工程成熟度（几乎为零）

这是从"代码就绪"到"可上架"之间的**主要鸿沟**：

| 发布要素 | 现状 | 缺口 |
|------|------|------|
| 发布 CI/CD | ❌ `ci.yml` 只有 build+test+artifact，**无任何 publish job** | 🔴 全部缺失 |
| 版本号 | `0.1.0` 静态写死在各 `Cargo.toml`/`package.json`/`pyproject.toml` | 🔴 无 semver 流程 |
| 跨平台二进制 | Node 4 target、ffi 4 target 已建，Swift/Kotlin/Flutter 缺移动端 target | 🔴 移动端不全 |
| 签名/公证 | 无 | 🔴 Apple notarization / Android signing / npm provenance / PyPI trusted publishing 均未配 |
| LICENSE | ❌ **根目录无 LICENSE 文件**（`Cargo.toml` 声明 MIT 但仓库无文件） | 🔴 必须补 |
| 仓库元数据 | `Cargo.toml` 的 `repository` 仍是占位符 `yourusername/aimux`（实际是 `arcships/aimux`） | 🟡 需修正 |
| Flutter 发布 | `publish_to: none`（明确禁止发布） | 🔴 需开放 |
| crates.io | 未规划 | 🔴 Rust 核心发布策略空白 |

### 2.3 双路径架构（已定型，不需改动）

```
           ┌─ 原生路径 ──→ aimux-core + aimux-providers（直接映射 + async）
绑定层 ────┤   Node (napi-rs) / Python (PyO3)
           │
           └─ C ABI 路径 ──→ aimux-ffi（opaque handle + JSON + push callback）
               Swift / Kotlin / Flutter / Go / C / C++
```

- **原生路径**（Node/Python）：绕过 aimux-ffi，DX 最好，二进制直接是 napi `.node` / Python `.so` wheel。
- **C ABI 路径**（Swift/Kotlin/Flutter/Go/C/C++）：走 aimux-ffi。其中 **Go 走 cgo 静态链接 `libaimux_ffi.a`**，产物是单 binary（Rust 核心编进可执行文件），与 Swift/Kotlin/Flutter 的动态库分发方式不同。

---

## 三、发布平台矩阵

| 语言/生态 | 发布平台 | 包名 | 工具链 | 目标平台 | 二进制分发机制 |
|------|------|------|------|------|------|
| **Rust 核心** | crates.io | `aimux-core` 等 7 个 | cargo | 跨平台源码 | 源码 crate（用户本地编译） |
| **Node.js** | npm | `aimux`（root）+ `@aimux/node-*` | napi-rs v3 | linux-x64-gnu / linux-arm64-gnu / macos-x64 / macos-arm64 / win32-x64-msvc | napi 多平台 optionalDependencies |
| **Python** | PyPI | `aimux` | PyO3 + maturin | manylinux x86_64/aarch64 / macOS universal2 / Windows AMD64 | platform wheel（abi3） |
| **Swift** | Swift Package Manager | `Aimux` | SPM + xcframework | macOS arm64/x86_64 + iOS arm64 | BinaryTarget（xcframework） |
| **Kotlin** | Maven Central | `io.aimux:aimux` | Gradle + JNA | Android arm64/armv7/x86_64 + JVM linux/mac/win | per-platform `.aar`/`.jar` |
| **Flutter** | pub.dev | `aimux` | dart:ffi | iOS / Android / macOS / Windows / Linux | 纯 Dart 包 + 外部 native 库 |
| **Go** | Go module proxy | `github.com/arcships/aimux/bindings/go` | cgo + `libaimux_ffi.a` | linux-x86_64-musl / linux-aarch64-musl / macos-x64 / macos-arm64 / win32-x64-msvc | 预编译 `.a` 随 module 或 GitHub Release |
| **C / C++** | GitHub Release | — | cargo + cbindgen | 同 ffi 矩阵 | 预编译 `.so`/`.dylib`/`.dll` + `.h` |

### 目标平台完整矩阵（Rust target triple）

```
桌面端：
  x86_64-unknown-linux-gnu        Linux x64（动态库路径）
  x86_64-unknown-linux-musl       Linux x64（Go 全静态 ELF）
  aarch64-unknown-linux-gnu       Linux arm64
  aarch64-unknown-linux-musl      Linux arm64（Go 全静态 ELF）
  x86_64-apple-darwin             macOS Intel
  aarch64-apple-darwin            macOS Apple Silicon
  x86_64-pc-windows-msvc          Windows x64

移动端（C ABI 路径）：
  aarch64-apple-ios               iOS arm64
  aarch64-linux-android           Android arm64
  armv7-linux-androideabi         Android armv7
  x86_64-linux-android            Android x64（模拟器）
```

当前 CI 只覆盖前 4 个桌面 target（linux-x64 / macos-arm64 / macos-x64 / win-x64）。移动端 4 个 target 完全缺失，Go 的 musl target 也未加入。

---

## 四、版本管理策略

### 4.1 统一版本号

所有 crate 与语言包**共享一个版本号**，同一次发布同步更新。

- 版本源：workspace `Cargo.toml` 的 `[workspace.package] version`（当前 `0.1.0`）。
- 各语言包的版本号从同一来源同步（见下文各语言章节）。
- 发布动作：打 git tag `v0.1.0`，CI 从 tag 提取版本号，触发全平台构建+发布。

### 4.2 语义化版本（SemVer）

- `0.x.y` 阶段：API 可能变动，minor 版本允许 breaking change。
- `1.0.0`：API 稳定后锁定，此后遵循严格 SemVer。
- wire schema 已有 `specVersion` 字段锁定跨边界契约（与库版本号正交）。

### 4.3 变更日志

- 用 `git-cliff` 或 `release-plz` 从 conventional commits 自动生成 CHANGELOG（参考仓库内 `reference/rig/release-plz.toml`）。
- Rust crate 的 CHANGELOG 由 release-plz 管理；语言包的 CHANGELOG 手动或脚本同步。

---

## 五、Rust 核心发布计划（crates.io）

### 5.1 待发布的 7 个 crate

| crate | 类型 | 发布必要性 |
|------|------|------|
| `aimux-core` | 核心抽象 | ✅ 必发（其他 crate 依赖） |
| `aimux-stream` | SSE/NDJSON 解析 | ✅ 必发 |
| `aimux-provider-utils` | HTTP 工具 | ✅ 必发 |
| `aimux-providers` | 172 厂商实现 | ✅ 必发（用户直接依赖） |
| `aimux-ffi` | C ABI 基座 | ✅ 必发（C ABI 绑定用户依赖） |

### 5.2 发布前必修项

1. **补 LICENSE 文件**——根目录建 `LICENSE`（MIT 全文），各 crate `Cargo.toml` 已声明 `license = "MIT"`。
2. **修正 repository 字段**——`Cargo.toml` 的 `repository = "https://github.com/yourusername/aimux"` 改为 `https://github.com/arcships/aimux`。
3. **补元数据字段**——各 crate `Cargo.toml` 补 `keywords`、`categories`、`readme`、`documentation`：
   ```toml
   keywords = ["llm", "ai", "openai", "anthropic", "provider"]
   categories = ["api-bindings", "asynchronous"]
   ```
4. **确认无 `publish = false`**——当前无（已核查），可发布。

### 5.3 发布流程

```bash
# 1. 检查
cargo publish --dry-run -p aimux-core

# 2. 按依赖顺序发布（核心在前）
cargo publish -p aimux-stream
cargo publish -p aimux-provider-utils
cargo publish -p aimux-core           # 依赖上面两个
cargo publish -p aimux-providers      # 依赖 core/stream/utils
cargo publish -p aimux-ffi            # 依赖 core/providers

# 3. CI 自动化（见第十二章）
```

**密钥**：`CARGO_REGISTRY_TOKEN`（crates.io API token，存 GitHub Actions secret）。

---

## 六、Node.js 发布计划（npm）

### 6.1 现状

- `bindings/node/package.json` 已配置 napi-rs v3，`prepublishOnly: "napi prepublish -t npm"` 钩子就位。
- CI 已构建 4 target 的 `.node` 文件并上传 artifact。
- 包名 `aimux`（root）+ `@aimux/node`（napi package name）。

### 6.2 发布机制（napi-rs 标准流程）

napi-rs 的发布模式：**一个 root 包 + 每个平台一个 optionalDependencies 包**。

```
aimux@0.1.0                      # root，纯 JS + .d.ts，无 native
├── @aimux/node-linux-x64-gnu    # optionalDependency
├── @aimux/node-linux-arm64-gnu
├── @aimux/node-darwin-x64
├── @aimux/node-darwin-arm64
└── @aimux/node-win32-x64-msvc
```

用户 `npm install aimux` 时，npm 根据 `os`/`cpu` 字段自动只下载当前平台的包。

### 6.3 发布前必修项

1. **补 `files` 字段**——`package.json` 需声明发布内容：
   ```json
   "files": ["index.js", "index.d.ts", "src/index.ts", "*.node"]
   ```
2. **补 `os`/`cpu` 字段**——root 包需声明支持的平台组合。
3. **修 `seed: bigint` 崩溃**（audit-003 L3）——ts-rs 把 Rust `u64` 映射为 TS `bigint`，`JSON.stringify` 对 bigint 抛异常。发布前在 wrapper 层做 `Number(seed)` 转换。
4. **补 README**——`bindings/node/` 下需有面向 npm 用户的 README。
5. **配置 npm provenance**——npm publish 时加 `--provenance` 标志，生成可验证的来源证明（需 GitHub Actions OIDC）。

### 6.4 CI 发布流程

```yaml
# .github/workflows/release.yml（新增）
node-release:
  needs: [build-all-platforms]
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: actions/setup-node@v4
      with:
        node-version: 20
        registry-url: 'https://registry.npmjs.org'
    - run: npm install
      working-directory: bindings/node
    - run: npx napi prepublish -t npm   # 生成 optionalDependencies + 拉取各平台包
      working-directory: bindings/node
    - run: npm publish --provenance --access public
      working-directory: bindings/node
      env:
        NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
```

---

## 七、Python 发布计划（PyPI）

### 7.1 现状

- `bindings/python/pyproject.toml` 配置 maturin 后端，`requires-python >= 3.8`，abi3 wheel。
- CI 在 3 OS 上 `maturin develop` + pytest，但**只测不发布**。
- 包名 `aimux`。

### 7.2 发布机制（maturin + trusted publishing）

maturin 构建 platform wheel（非 universal），每个 OS+arch 一个 wheel。用 **PyPI OIDC trusted publishing** 免存 token：

```
aimux-0.1.0-cp38-abi3-linux_x86_64.whl
aimux-0.1.0-cp38-abi3-linux_aarch64.whl
aimux-0.1.0-cp38-abi3-macosx_10_9_x86_64.macosx_11_0_arm64.whl
aimux-0.1.0-cp38-abi3-win_amd64.whl
```

### 7.3 发布前必修项

1. **补 PyPI trusted publisher**——在 PyPI 后台为 `arcships/aimux` 仓库配置 trusted publisher（environment + workflow 文件名 + job 名），无需存 token。
2. **Python wrapper 默认化**（cross-lang-dx-plan 标"进行中"）——当前需 `from aimux.wrapper import ...`，应默认从顶层导出。
3. **补 README + long_description**——`pyproject.toml` 加 `[project] readme = "README.md"`。
4. **补 classifiers**——PyPI 分类标签。
5. **修 `AiMuxError` 用 `Any`**（audit-003 L4）——非阻塞，建议首版后修。
6. **用 cibuildwheel 或 maturin CI action 构建多平台 wheel**。

### 7.4 CI 发布流程

```yaml
python-release:
  strategy:
    matrix:
      include:
        - { os: ubuntu-latest, target: x86_64 }
        - { os: ubuntu-latest, target: aarch64 }
        - { os: macos-latest,  target: universal2 }
        - { os: windows-latest, target: x86_64 }
  runs-on: ${{ matrix.os }}
  permissions:
    id-token: write   # OIDC trusted publishing 必需
  steps:
    - uses: actions/checkout@v4
    - uses: PyO3/maturin-action@v1
      with:
        command: publish
        args: --no-sdist --out dist
        working-directory: bindings/python
    - uses: pypa/gh-action-pypi-publish@release/v1
      with:
        packages-dir: bindings/python/dist
```

---

## 八、Swift 发布计划（Swift Package Manager）

### 8.1 现状

- `bindings/swift/Package.swift` 用 `systemLibrary` 引用 C 头文件，依赖本地 `libaimux_ffi.dylib`。
- CI 仅 macOS，`swift build`/`swift test` 带 `|| echo "needs xcframework (PoC)"` 兜底——**实际常失败**。
- 是发布链条中**最不成熟的一环**。

### 8.2 发布机制（SPM BinaryTarget + xcframework）

Swift 发布需要 **xcframework**（包含多架构 slice 的二进制框架）：

```
AimuxFFI.xcframework/
├── ios-arm64/           libaimux_ffi.a (iOS arm64)
├── macos-arm64_x86_64/  libaimux_ffi.dylib (universal macOS)
└── Info.plist
```

SPM `Package.swift` 用 `binaryTarget` 引用打包好的 xcframework（托管在 GitHub Release）：

```swift
.binaryTarget(
    name: "AimuxFFI",
    url: "https://github.com/arcships/aimux/releases/download/v0.1.0/AimuxFFI.xcframework.zip",
    checksum: "..."  # SPM 校验和
)
```

### 8.3 发布前必修项

1. 🔴 **构建 xcframework**——用 `cargo build` 编译 iOS + macOS target，用 `xcodebuild -create-xcframework` 打包：
   ```bash
   cargo build -p aimux-ffi --release --target aarch64-apple-ios
   cargo build -p aimux-ffi --release --target aarch64-apple-darwin
   cargo build -p aimux-ffi --release --target x86_64-apple-darwin
   xcodebuild -create-xcframework \
     -library target/aarch64-apple-ios/release/libaimux_ffi.a \
     -library target/aarch64-apple-darwin/release/libaimux_ffi.dylib \
     -output AimuxFFI.xcframework
   ```
2. 🔴 **改 Package.swift**——从 `systemLibrary`（需本地装库）改为 `binaryTarget`（自动下载 xcframework）。
3. **Apple 代码签名 + notarization**——对 xcframework 签名并公证（发布到 iOS App Store 必须）。
4. **补 iOS CI target**——CI 矩阵加 `aarch64-apple-ios`。
5. **移除 `|| echo` 兜底**——CI 中 Swift 构建失败应阻塞而非忽略。

### 8.4 发布流程

xcframework 打包为 zip 上传 GitHub Release，更新 `Package.swift` 的 `checksum`，打 git tag。SPM 用户通过 `.package(url:)` 引用，自动拉取。

---

## 九、Kotlin 发布计划（Maven Central）

### 9.1 现状

- `bindings/kotlin/build.gradle.kts` 配置 JNA，`group = "io.aimux"`，`version = "0.1.0"`。
- CI 仅 Linux x64，`gradle test` 带 `|| echo "skipped in CI PoC"`——**测试常被跳过**。
- 无 Android target、无 `.aar`、无 Maven 发布配置。

### 9.2 发布机制（Maven Central + per-platform artifact）

Maven Central 发布需通过 **Central Publisher Portal**（原 Sonatype）：

1. **注册 namespace** `io.aimux`——需验证对 `aimux.io` 域名的所有权，或用 GitHub namespace `io.github.arcships`。
2. **GPG 签名**——所有 artifact 需 GPG 签名。
3. **per-platform 分发**——像 napi-rs 那样，每个平台一个 artifact：
   ```
   io.aimux:aimux:0.1.0                    # 纯 Kotlin（JNA 接口 + 类型）
   io.aimux:aimux-native:0.1.0:linux-x86_64  # libaimux_ffi.so
   io.aimux:aimux-native:0.1.0:linux-aarch64
   io.aimux:aimux-native:0.1.0:darwin        # .dylib
   io.aimux:aimux-native:0.1.0:windows        # .dll
   io.aimux:aimux-native:0.1.0:android-arm64  # .so
   ```

### 9.3 发布前必修项

1. 🔴 **注册 Maven Central namespace**——`io.aimux`（需域名验证）或 `io.github.arcships`（验证 GitHub 仓库即可，更快）。
2. 🔴 **配置 Gradle `maven-publish` + signing 插件**——`build.gradle.kts` 加：
   ```kotlin
   plugins {
       `maven-publish`
       signing
   }
   ```
3. 🔴 **产出 `.aar`**（Android）——当前 CI 只把 `.so` 塞进 `src/main/resources`，非标准 Android 打包。需 `com.android.library` plugin 产出 `.aar`。
4. 🔴 **补 Android CI target**——`aarch64-linux-android` / `armv7-linux-androideabi`。
5. **移除 `|| echo` 兜底**——测试失败应阻塞。
6. **修测试质量**（audit-002 L2/L3/L4）——流式测试缩短超时、显式检查 onError、改 JSON 解析断言。
7. **补 javadoc/sources jar**——Maven Central 要求。

### 9.4 CI 发布流程

```yaml
kotlin-release:
  needs: [build-ffi-android, build-ffi-desktop]
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: actions/setup-java@v4
      with: { java-version: 17, distribution: temurin }
    - run: ./gradlew publishToSonatype closeAndReleaseStagingRepository
      working-directory: bindings/kotlin
      env:
        ORG_GRADLE_PROJECT_sonatypeUsername: ${{ secrets.SONATYPE_USER }}
        ORG_GRADLE_PROJECT_sonatypePassword: ${{ secrets.SONATYPE_PASS }}
        ORG_GRADLE_PROJECT_signingKey: ${{ secrets.GPG_PRIVATE_KEY }}
        ORG_GRADLE_PROJECT_signingPassword: ${{ secrets.GPG_PASSPHRASE }}
```

---

## 十、Flutter 发布计划（pub.dev）

### 10.1 现状

- `bindings/flutter/pubspec.yaml` 配 `publish_to: none`（**明确禁止发布**）。
- **完全不在 CI 矩阵中**——无 flutter/dart job。
- dart:ffi 手写，纯 Dart 包，但运行时需 `libaimux_ffi` native 库。

### 10.2 核心难题：native 库如何随 Dart 包分发

dart:ffi 是纯 Dart，但 pub.dev 上的纯 Dart 包**不携带 native 二进制**。Flutter 用户需自行获取 native 库。两个方案：

**方案 A：Flutter plugin 包（推荐）**
- 改为 Flutter plugin（`pubspec.yaml` 加 `flutter.plugin.platforms`），每个平台（iOS/Android/macOS/Windows/Linux）声明 native 库来源。
- iOS/Android：native 库打包进 plugin 的 podspec/gradle。
- 桌面端：通过 CMake/Makefile 链接预编译库或源码编译。

**方案 B：纯 Dart 包 + 文档引导**
- 保持纯 Dart 包，README 指引用户从 GitHub Release 下载对应平台的 native 库。
- DX 差，仅适合 PoC。

### 10.3 发布前必修项

1. 🔴 **定义 native 库分发方案**——选方案 A（Flutter plugin）或 B（手动下载）。
2. 🔴 **改 `publish_to`**——从 `none` 改为 `https://pub.dev`（或删除该行默认 pub.dev）。
3. 🔴 **加 Flutter CI**——CI 矩阵加 dart/flutter 测试 job。
4. **补 `StreamPart` 完整类型化**（audit-003 遗留）——当前是 raw Map + 访问器，非 17 变体完整类型化（其他 4 语言均已完整）。
5. **补 README + 截图/示例**——pub.dev 需要。

### 10.4 发布流程

```bash
cd bindings/flutter
dart pub publish           # 需 Google 账号 + pub.dev 验证
```

---

## 十一、Go 发布计划（Go module proxy）

### 11.1 现状

- `bindings/go/` 是 2026-07-31 新增的第七种语言绑定，RFC-0011 已落地 PoC。
- cgo 静态链接 `libaimux_ffi.a`，产物是**单 binary**（Rust 核心编进可执行文件，7.5MB，无需分发 `.so/.dll/.dylib`）。
- 31 个测试全绿（7 单元 + 6 E2E + 16 往返 + 契约子测试），覆盖工具调用/多角色/ToolChoice/流式/往返全场景。
- **完全不在 CI 矩阵中**——RFC-0011 路线图标阶段 4"CI 矩阵"为⏳ 待做。

### 11.2 发布机制（Go module proxy，无中心化 registry）

Go 没有类似 npm/PyPI/Maven Central 的中心化包仓库，而是通过 **module proxy（proxy.golang.org）** 分发：

```
开发者打 git tag v0.1.0
    │
    ▼
Go module proxy 自动索引该 tag（go get 拉取时触发）
    │
    ▼
用户 go get github.com/arcships/aimux/bindings/go@v0.1.0
```

**"发布"动作 = 打 git tag**，无需专用 publish job。但有一个关键前提：module path 必须指向真实存在的仓库路径。

### 11.3 核心难题：cgo 编译期依赖 `libaimux_ffi.a`

这是 Go 绑定发布与其它所有语言绑定的**本质区别**，也是最大难点：

- 普通 Go module：`go get` 拉源码 → `go build` 直接编译，零外部依赖。
- aimux-go：`go get` 拉源码 → `go build` 时 cgo 需要 `libaimux_ffi.a`，**用户本地无此文件则编译失败**。

RFC-0011 §8 风险 #1 已明确此问题但未给最终方案。三个候选方案：

| 方案 | 做法 | 优点 | 缺点 |
|------|------|------|------|
| **A：随 module 分发预编译 `.a`** | 各平台 `.a` 提交进仓库或随 tag | 用户 `go get` 即用 | Go module 不擅长二进制；`.a` 体积大（82MB 未去重）；跨平台需多份 |
| **B：构建脚本自动 `cargo build`** | `go generate` 脚本调 cargo | 单一源 | 用户需装 Rust 工具链，破坏"单 binary 无需 Rust"优势 |
| **C：从 GitHub Release 下载 `.a`** | 构建脚本按平台下载预编译 `.a` | 用户无需 Rust；仓库干净 | 需联网；需可靠 download 脚本；离线构建受限 |

**推荐方案 C**（与第十二章 C/C++ 制品共用 GitHub Release 的 `.a`）：CI 为每个 target 预编译 `libaimux_ffi.a` 上传 Release，Go 包内附 `go generate` 脚本按 `GOOS/GOARCH` 下载对应 `.a`。兼顾"用户无需 Rust"和"仓库干净"。

### 11.4 发布前必修项

1. 🔴 **修正 module path**——`go.mod` 当前是 `github.com/aimux/aimux-go`（不存在的仓库），实际仓库是 `github.com/arcships/aimux`。必须改为：
   ```
   module github.com/arcships/aimux/bindings/go
   ```
   否则 `go get` 会去 `github.com/aimux/aimux-go` 找仓库而失败。这是 Go 发布的**首要阻塞项**。
2. 🔴 **定 `.a` 分发方案**（上节方案 A/B/C 三选一，推荐 C）。
3. 🔴 **CI 加 Go job**——构建 5 平台 `.a` + `go test`。RFC-0011 §7.2 已给 target 矩阵：
   ```
   x86_64-unknown-linux-musl    （全静态 ELF）
   aarch64-unknown-linux-musl   （全静态 ELF）
   x86_64-apple-darwin
   aarch64-apple-darwin
   x86_64-pc-windows-msvc
   ```
4. 🔴 **补 README**——`bindings/go/` 无 README，需面向 Go 用户的使用说明。
5. 🟡 **修 cgo LDFLAGS 路径**——当前 `LDFLAGS` 写死 `${SRCDIR}/../../target/release`，依赖本地 cargo 构建。需适配方案 C 的下载位置。

### 11.5 发布流程

Go 不需要专用 publish job，跟随主 release tag：

```yaml
# .github/workflows/release.yml 的 go 部分
go-build-test:
  strategy:
    matrix:
      target: [x86_64-unknown-linux-musl, aarch64-unknown-linux-musl,
               x86_64-apple-darwin, aarch64-apple-darwin, x86_64-pc-windows-msvc]
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
      with: { targets: ${{ matrix.target }} }
    - run: cargo build -p aimux-ffi --release --target ${{ matrix.target }}
    - uses: actions/setup-go@v5
      with: { go-version: '1.23' }
    - run: go test ./...
      working-directory: bindings/go
      env:
        CGO_LDFLAGS: -L${{ github.workspace }}/target/${{ matrix.target }}/release
    - uses: actions/upload-artifact@v4
      with:
        name: libaimux_ffi-${{ matrix.target }}.a
        path: target/${{ matrix.target }}/release/libaimux_ffi.a
```

打 tag `v0.1.0` 后：
1. CI 构建 5 平台 `.a` 上传 GitHub Release（供方案 C 下载）。
2. Go module proxy 自动索引 tag——用户 `go get` 即可拉取源码。
3. 用户首次构建时 `go generate` 从 Release 下载对应平台 `.a`。

**无需任何 secret/token**——Go module proxy 公开索引，这是 Go 相对其它语言发布最简单的一面。

## 十二、C / C++ 分发

C/C++ 无标准包管理生态（vcpkg/conan 未在规划内），以 **GitHub Release 制品**形式分发：

```
GitHub Release v0.1.0/
├── aimux-ffi.h                    # C 头文件
├── libaimux_ffi-linux-x64.so
├── libaimux_ffi-linux-arm64.so
├── libaimux_ffi-macos-universal.dylib
├── aimux_ffi-windows-x64.dll
└── aimux_ffi-src.tar.gz           # 源码（可选，供 vcpkg/conan 打包）
```

`bindings/c/example.c`、`example.cpp` 已有示例。用户从 Release 下载库 + 头文件，自行链接。

---

## 十三、aimux-ffi 二进制制品矩阵

aimux-ffi 是 Swift/Kotlin/Flutter/C 的共享基座。它的跨平台二进制矩阵是**移动端绑定的共同前提**：

```
制品                      target triple               消费方
──────────────────────────────────────────────────────────────
libaimux_ffi.so           x86_64-unknown-linux-gnu     Kotlin(JVM) / C
libaimux_ffi.so           aarch64-unknown-linux-gnu    Kotlin(JVM) / C
libaimux_ffi.dylib         x86_64-apple-darwin          Swift / Kotlin(JVM) / C
libaimux_ffi.dylib         aarch64-apple-darwin         Swift / Kotlin(JVM) / C
aimux_ffi.dll              x86_64-pc-windows-msvc       Kotlin(JVM) / C
libaimux_ffi.a             x86_64-unknown-linux-musl    Go（cgo 静态链接）
libaimux_ffi.a             aarch64-unknown-linux-musl   Go（cgo 静态链接）
libaimux_ffi.a             x86_64-apple-darwin          Go（cgo 静态链接）
libaimux_ffi.a             aarch64-apple-darwin         Go（cgo 静态链接）
libaimux_ffi.a             x86_64-pc-windows-msvc       Go（cgo 静态链接）
libaimux_ffi.a             aarch64-apple-ios            Swift(xcframework) / Flutter(iOS)
libaimux_ffi.so            aarch64-linux-android        Kotlin(.aar) / Flutter(Android)
libaimux_ffi.so            armv7-linux-androideabi      Kotlin(.aar) / Flutter(Android)
libaimux_ffi.so            x86_64-linux-android          Kotlin(.aar) / Flutter(模拟器)
```

当前 CI 只产前 4 个桌面动态库制品。Go 的 5 个 `.a` 静态库制品和移动端 4 个制品均需补齐。

---

## 十四、CI/CD 发布流程总设计

### 13.1 整体流程

```
开发者打 tag v0.1.0
    │
    ▼
GitHub Actions 触发 release.yml
    │
    ├─ Job: rust-publish         → cargo publish × 7 crate → crates.io
    ├─ Job: ffi-build-matrix      → 13 target 编译 → 上传 artifact
    ├─ Job: node-release         → napi prepublish + npm publish → npm
    ├─ Job: python-release        → maturin publish × 4 wheel → PyPI
    ├─ Job: go-build-test         → 5 平台 .a + go test → GitHub Release（无 publish job）
    ├─ Job: swift-release         → xcodebuild xcframework → GitHub Release → SPM
    ├─ Job: kotlin-release        → gradle publish → Maven Central
    └─ Job: flutter-release       → dart pub publish → pub.dev
```

### 13.2 触发条件

```yaml
on:
  push:
    tags: ['v*']
  workflow_dispatch:   # 手动触发
```

### 13.3 所需 GitHub Secrets / 环境配置

| Secret | 用途 | 获取方式 |
|------|------|------|
| `CARGO_REGISTRY_TOKEN` | crates.io 发布 | crates.io → Account Settings → API Tokens |
| `NPM_TOKEN` | npm 发布 | npm → Access Tokens（Automation 类型） |
| — | PyPI 发布 | OIDC trusted publishing（无需 secret，配 PyPI 后台即可） |
| `SONATYPE_USER` / `SONATYPE_PASS` | Maven Central 发布 | Central Portal 账号 |
| `GPG_PRIVATE_KEY` / `GPG_PASSPHRASE` | Maven 签名 | 本地 `gpg --export-secret-keys` |
| `PUB_DEV_TOKEN` | pub.dev 发布 | `dart pub login` 获取（或 OIDC） |

### 13.4 发布顺序

1. **Rust 核心**（crates.io）——其他绑定编译依赖它，最先发。
2. **aimux-ffi 全平台二进制**——Swift/Kotlin/Flutter/Go 依赖这些制品。
3. **Node + Python + Go**（Node/Python 原生路径不依赖 ffi 制品；Go 虽走 C ABI 但静态链接，只需 `.a` 制品就位即可，无需签名/registry）。
4. **Swift + Kotlin**（C ABI 移动端，依赖 ffi 制品 + 签名公证）。
5. **Flutter**（C ABI 移动端，依赖 ffi 制品 + native 库分发方案）。
6. **GitHub Release**（C/C++ 制品 + xcframework zip）。

---

## 十五、签名与安全

| 平台 | 签名/来源证明 | 状态 |
|------|------|------|
| crates.io | API token | 待配 |
| npm | `--provenance`（OIDC 来源证明） | 待配 |
| PyPI | OIDC trusted publishing | 待配 |
| Maven Central | GPG 签名 | 待配 |
| Swift/iOS | Apple Developer 代码签名 + notarization | 待配 |
| Android | APK/AAB signing（如发到 Play） | 待配 |
| pub.dev | Google 账号验证 | 待配 |
| Go module proxy | 无需签名（git tag 即来源证明） | ✅ 无需配置 |

**Apple notarization**（仅 macOS/iOS 分发需要）：
```bash
codesign --deep --options runtime --sign "Developer ID Application: ..." AimuxFFI.xcframework
xcrun notarytool submit AimuxFFI.xcframework.zip --apple-id ... --team-id ... --wait
xcrun stapler staple AimuxFFI.xcframework
```

---

## 十六、发布前必修项总清单

按优先级排序（🔴 阻塞首版发布 / 🟡 建议首版前修 / 🟢 可首版后迭代）：

### 全局（影响所有平台）
- 🔴 补根目录 `LICENSE` 文件（MIT 全文）
- 🔴 修正 `Cargo.toml` 的 `repository` 占位符 → `arcships/aimux`
- 🔴 补各 crate `Cargo.toml` 的 `keywords`/`categories`/`readme` 元数据
- 🔴 建 `release.yml` 发布 CI（当前只有 build+test CI）
- 🔴 统一版本号同步机制（workspace version → 各语言包）
- 🟡 配置 CHANGELOG 自动生成（release-plz 或 git-cliff）

### Node.js
- 🔴 `package.json` 补 `files`/`os`/`cpu` 字段
- 🔴 修 `seed: bigint` 的 `JSON.stringify` 崩溃
- 🟡 补 `bindings/node/README.md`
- 🟡 配置 npm provenance

### Python
- 🔴 wrapper 默认化（顶层导出）
- 🟡 `pyproject.toml` 补 `readme`/`classifiers`
- 🟢 修 `AiMuxError` 的 `Any` 类型

### Swift
- 🔴 构建 xcframework（iOS + macOS slice）
- 🔴 `Package.swift` 从 `systemLibrary` 改 `binaryTarget`
- 🔴 CI 加 `aarch64-apple-ios` target
- 🔴 移除 `swift build || echo` 兜底
- 🟡 Apple 代码签名 + notarization

### Kotlin
- 🔴 注册 Maven Central namespace（`io.github.arcships` 起步）
- 🔴 配置 `maven-publish` + `signing` 插件
- 🔴 产出 Android `.aar`（当前只有 .so 塞 resources）
- 🔴 CI 加 Android target（arm64/armv7）
- 🔴 移除 `gradle test || echo` 兜底
- 🟡 修测试质量（L2/L3/L4）
- 🟡 补 javadoc/sources jar

### Flutter
- 🔴 定义 native 库分发方案（Flutter plugin vs 手动下载）
- 🔴 `pubspec.yaml` 改 `publish_to`
- 🔴 CI 加 Flutter/Dart job
- 🟡 补 `StreamPart` 完整类型化（17 变体）

### Go
- 🔴 修正 `go.mod` 的 module path → `github.com/arcships/aimux/bindings/go`（当前指向不存在的 `github.com/aimux/aimux-go`）
- 🔴 定 `libaimux_ffi.a` 分发方案（推荐方案 C：GitHub Release 下载 + `go generate` 脚本）
- 🔴 CI 加 Go job（5 平台 `.a` 构建 + `go test`）
- 🔴 补 `bindings/go/README.md`
- 🟡 修 cgo `LDFLAGS` 路径硬编码（适配 `.a` 下载位置）

### Rust 核心
- 🟡 确认 7 个 crate 的发布顺序与依赖关系
- 🟡 `aimux-ffi` 作为 cdylib 发布到 crates.io 的可行性确认

---

## 十七、里程碑与路线图

### M0 — 发布基础设施（~1 周）
- [ ] 补 LICENSE / 修 repository / 补 Cargo 元数据
- [ ] 建版本同步脚本（workspace version → package.json / pyproject / pubspec / build.gradle）
- [ ] 建 `release.yml` 骨架（tag 触发）

### M1 — Rust 核心 + Node + Python + Go 首发（~1-2 周）
- [ ] crates.io 发布 7 个 crate
- [ ] npm 发布 Node 包（5 平台 optionalDependencies）
- [ ] PyPI 发布 Python 包（4 wheel + trusted publishing）
- [ ] Go：修正 module path + `.a` 分发方案 + 5 平台 CI + 打 tag（module proxy 自动索引）
- **Node/Python 是原生路径，无移动端依赖，最快出。Go 虽走 C ABI 但静态链接单 binary，只需 `.a` 制品就位，无需签名/registry，可与前三者同步出**。

### M2 — aimux-ffi 全平台二进制（~1 周）
- [ ] CI 加 4 个移动端 target（iOS arm64 / Android arm64 / armv7 / x64）
- [ ] CI 加 Go 的 2 个 musl target（linux 全静态 ELF）
- [ ] 构建 xcframework
- [ ] GitHub Release 上传全平台制品（含 Go 的 `.a`）

### M3 — Swift + Kotlin 首发（~2 周）
- [ ] Swift：xcframework BinaryTarget + 签名公证
- [ ] Kotlin：Maven Central namespace 注册 + GPG + `.aar`
- **依赖 M2 的 ffi 制品**。

### M4 — Flutter 首发（~1-2 周）
- [ ] native 库分发方案落地
- [ ] pub.dev 发布
- **距离最远，最后发**。

### M5 — 多模态绑定（RFC-0008 落地后）
- [ ] 当前任何发布物只含文本生成模态
- [ ] Embedding/Speech/Image/Transcription 等模态待 RFC-0008 评审通过后加入发布物
- **不阻塞首版发布**。

---

## 十八、风险与未决项

| 风险 | 影响 | 缓解 |
|------|------|------|
| Maven Central namespace 验证耗时 | 阻塞 Kotlin 首发 | 用 `io.github.arcships` 起步（验证 GitHub 仓库即可，比域名验证快） |
| Apple notarization 需 Apple Developer 账号 | 阻塞 Swift/iOS 首发 | 需 $99/年开发者账号，提前申请 |
| Flutter native 库分发方案未定 | 阻塞 Flutter 首发 | M4 前必须决策方案 A/B |
| Go cgo `.a` 分发方案未定 | 阻塞 Go 首发 | 推荐方案 C（GitHub Release 下载），M1 前定 |
| Go module path 指向不存在仓库 | `go get` 直接失败 | M1 必修：改为 `github.com/arcships/aimux/bindings/go` |
| 多模态 RFC-0008 未落地 | 首版只含文本模态 | 不阻塞，后续版本叠加 |
| 版本同步脚本缺失 | 7 语言版本可能漂移 | M0 必做同步脚本 |
| 7 语言绑定的类型同步自动化不均 | Rust 类型变更后跨语言漂移 | Node 全自动(ts-rs)；其余手动或配 codegen |

---

## 十九、总结

aimux 的**代码层已就绪**（双路径架构定型、7 语言绑定审计通过、172 厂商 + 2650 cassette 测试），但**发布工程层几乎从零开始**：

1. **无发布 CI**——现有 CI 只 build+test，需新建 `release.yml`。
2. **无版本管理**——版本号静态写死，需统一同步机制。
3. **移动端二进制缺失**——Swift 缺 xcframework、Kotlin 缺 `.aar`、Flutter 连 CI 都没有。
4. **无签名/公证**——Apple/Android/Maven GPG/npm provenance/PyPI trusted publishing 均未配（Go module proxy 例外，无需签名）。
5. **Rust 核心未发 crates.io**——7 个 crate 的发布策略空白。
6. **Go module path 错误**——`go.mod` 指向不存在的仓库，`go get` 会失败。

**优先级**：Rust 核心 + Node + Python + Go（前四者最快出，Go 虽走 C ABI 但静态链接单 binary 无需签名/registry）→ aimux-ffi 全平台二进制 → Swift + Kotlin（C ABI 移动端）→ Flutter（最后）。

预计首版发布（M0-M3）需 **4-6 周**，Flutter 视 native 库分发方案决策而定。
