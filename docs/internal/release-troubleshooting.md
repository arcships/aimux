# aimux 发布故障排查手册（内部）

> 0.2.1 首次全通道发布（2026-08-04）过程中遇到的每一个坑、根因、修复方式。
> 未来发布遇到类似问题时先查这里。
>
> 更新时间：2026-08-04（v0.2.1 发布后）

---

## 总览：发布链路全景

```
开发者打 tag v0.2.1  →  release.yml 触发
    │
    ├─ rust-publish        → crates.io × 5 crate（幂等，跳过已发版本）
    ├─ node-build × 6      → 平台 .node 制品
    ├─ node-publish        → npm @arcships/aimux（依赖 node-build）
    ├─ python-release × 3  → PyPI arcships-aimux（OIDC trusted publishing）
    ├─ go-build × 4        → GitHub Release（libaimux_ffi.a 静态库）
    ├─ ffi-build × 3       → GitHub Release（C ABI 动态库）
    ├─ flutter-ffi-mobile × 2 → Android .so + iOS xcframework 制品
    ├─ jvm-publish         → Maven Central（ai.arcships:aimux-java/-kotlin）
    ├─ flutter-publish     → pub.dev aimux（依赖 flutter-ffi-mobile）
    └─ github-release      → 聚合所有二进制制品
```

---

## 一、PowerShell 编码陷阱（Windows 开发环境）

### 症状

Java/Kotlin 源码批量替换包名后，36 个文件出现乱码（`鈥` `鍥` `鐨` 等不可读字符）。

### 根因

PowerShell 5.1 的 `Get-Content` / `Set-Content` 管道默认用系统 ANSI（GBK）编码读写。
源码文件是 UTF-8，其中的多字节字符（em dash `—`、引号等）被 GBK 误读后写回，
造成**永久损坏**（不是显示问题，是字节被替换了）。

同样的问题还损坏过：
- `package.json`（BOM + em dash 乱码）
- `pubspec.yaml`
- `build.gradle.kts`
- `SIGNING_KEY` secret（通过 `Get-Content | gh secret set` 管道传递时损坏）

### 修复

从 git HEAD 取原始字节，用 Python 字节级替换：

```python
import subprocess
data = subprocess.run(['git', 'show', 'HEAD:' + old_path],
                      capture_output=True).stdout
data = data.replace(b'io.aimux', b'ai.arcships.aimux')
with open(new_path, 'wb') as f:
    f.write(data)
```

### 规则

> **在 Windows 上处理含非 ASCII 字符的文件时，永远不要用 PowerShell 的
> Get-Content/Set-Content 管道。用以下替代之一：**
> 1. Python `pathlib.Path.read_text(encoding='utf-8')` / `write_text(encoding='utf-8')`
> 2. git 管道 `git show HEAD:path`（字节级）
> 3. `gh secret set` 传值时用 Python `subprocess.run(input=...)`，不用 PowerShell 管道

---

## 二、Maven Central 发布

### 坑 1：SIGNING_KEY 损坏 — "Could not read PGP secret key"

**症状**：`gradle publish` 报 `Error while evaluating property 'signatory.keyId'` +
`Could not read PGP secret key`。

**根因**：GPG 私钥通过 PowerShell 管道 `Get-Content key.asc | gh secret set` 传递时，
PowerShell 的 GBK 编码损坏了 ASCII armored 密钥中的多字节字符。

**修复**：用 Python 字节级导出 + 设置：

```python
import subprocess
key = subprocess.run(
    ['gpg', '--armor', '--export-secret-keys', '065B22BF824FC0A5'],
    capture_output=True, check=True).stdout
subprocess.run(['gh', 'secret', 'set', 'SIGNING_KEY', '--env', 'release'],
               input=key.decode('ascii'), capture_output=True, text=True)
```

### 坑 2：Sonatype 旧 OSSRH 端点返回 402 Payment Required

**症状**：`gradle publish` 报
`Could not PUT 'https://s01.oss.sonatype.org/...'. Received status code 402: Payment Required`。

**根因**：Sonatype 已废弃旧 OSSRH 端点（`s01.oss.sonatype.org`），
迁移到了 Central Publisher Portal。旧端点返回 402 表示服务已下线。

**修复**：`build.gradle.kts` 的发布仓库 URL 改为 Central Portal 的 OSSRH Staging API
兼容服务：

```kotlin
// Release versions
maven {
    url = uri("https://ossrh-staging-api.central.sonatype.com/service/local/staging/deploy/maven2/")
}
// SNAPSHOT versions
maven {
    url = uri("https://central.sonatype.com/repository/maven-snapshots/")
}
```

**关键**：按版本后缀路由仓库（`version.toString().endsWith("SNAPSHOT")`），
否则 release 版本发到 snapshots 仓库会返回 403 Forbidden。

### 坑 3：Snapshots 仓库对 release 版本返回 403 Forbidden

**症状**：`gradle publish` 同时向 staging 和 snapshots 两个仓库发布，
snapshots 仓库对非 SNAPSHOT 版本（0.2.1）返回 403。

**根因**：`build.gradle.kts` 同时配置了两个 maven 仓库（staging + snapshots），
`gradle publish` 会向所有配置的仓库发布。snapshots 仓库只接受 `-SNAPSHOT` 后缀的版本。

**修复**：按版本后缀条件配置仓库（见坑 2 的代码），只配置一个。

### 坑 4：上传后需手动 POST 确认部署

**症状**：`gradle publish` 成功（BUILD SUCCESSFUL），但 central.sonatype.com/publishing
看不到部署。

**根因**：OSSRH Staging API 兼容服务是"仅暂存"——上传后不会自动移入 Central Publisher Portal。
必须从**同一 IP**（同一 CI runner）POST 一个确认请求。

**修复**：release.yml 的 jvm-publish job 末尾加：

```bash
curl -sS -u "$OSSRH_USERNAME:$OSSRH_PASSWORD" \
  -X POST "https://ossrh-staging-api.central.sonatype.com/manual/upload/defaultRepository/ai.arcships" \
  -w '\nHTTP %{http_code}\n'
# 200 = 成功；部署出现在 central.sonatype.com/publishing
```

**然后**：在 central.sonatype.com/publishing 手动点 Publish（或配置自动发布）。

### 坑 5：Authentication 用 Portal User Token，不是 OSSRH 凭证

Central Portal 的 OSSRH Staging API 要求用 **Central Portal User Token** 认证，
不是旧的 OSSRH 账号密码。如果迁移了 namespace，必须在 central.sonatype.com 重新生成
User Token，用新的 username/password 对作为 `OSSRH_USERNAME` / `OSSRH_PASSWORD`。

---

## 三、Flutter / pub.dev 发布

### 坑 1：Flutter 3.44 SPM 不支持插件的 binaryTarget

**症状**：用 `ios/aimux/Package.swift` + `binaryTarget` 引用 xcframework 时，
`flutter build ios` 报 `Undefined symbol: _aimux_openai_new`（全部 Rust 符号缺失）。

**根因**：Flutter 3.44 的 SwiftPM 集成只支持源码型插件，`binaryTarget`（vendored
framework/xcframework）会被忽略——`.a` / `.framework` 根本不进 app 链接。

**修复**：放弃 SPM 路径，回退 podspec-only（CocoaPods fallback）。
Flutter 对没有 `Package.swift` 的插件会自动降级用 CocoaPods（会打印 SPM 警告但不影响构建）。

### 坑 2：dartPluginClass-only 插件不进 Podfile / Gradle

**症状**：pod install 成功但 Podfile 里没有 aimux；Android 的 jniLibs 不进 APK。

**根因**：只声明 `dartPluginClass`（不声明 `pluginClass`）的插件被 Flutter 工具视为
纯 Dart 插件——不会生成原生集成（不进 Podfile、不进 Gradle build）。

**修复**：pubspec.yaml 的 ios 和 android 平台**同时声明** `pluginClass` 和 `dartPluginClass`：

```yaml
flutter:
  plugin:
    platforms:
      ios:
        pluginClass: AimuxPlugin      # 空 Swift 类，让 Flutter 加进 Podfile
        dartPluginClass: AimuxPlugin
      android:
        pluginClass: AimuxPlugin      # 空 Java 类，让 Flutter 加进 Gradle
        dartPluginClass: AimuxPlugin
```

配套空原生类：
- iOS：`ios/Classes/AimuxPlugin.swift`（实现 `FlutterPlugin`，register 方法为空）
- Android：`android/src/main/java/.../AimuxPlugin.java`（实现 `FlutterPlugin`，方法为空）

### 坑 3：podspec 缺 `s.dependency 'Flutter'`

**症状**：iOS 构建报 `Unable to resolve module dependency: 'Flutter'`。

**根因**：`AimuxPlugin.swift` 里 `import Flutter`，但 podspec 没声明 Flutter 依赖。

**修复**：podspec 加 `s.dependency 'Flutter'`（Flutter 插件模板的标准要求）。

### 坑 4：iOS xcframework 链接 — force_load 挂错层

**症状**：`flutter build ios --debug --no-codesign --simulator` 构建成功，
但 `nm Runner | grep aimux` 全空——Rust 符号完全不在 app 二进制里。

**根因**（经 gpt-5.6-sol + glm-5.2 会诊 + 完整 xcodebuild 日志确认）：
`use_frameworks!` 下 aimux pod 被构建为**动态 framework**（`aimux.framework`）。
`pod_target_xcconfig` 的 `-force_load` 作用于 **pod 自己的链接**（`clang -dynamiclib ... -force_load ... -o aimux`），
符号进了 `aimux.framework`，而不是 Runner。而 `aimux.framework` 从未被 embed
（`Runner.app/Frameworks/` 为空），所以运行时 `DynamicLibrary.process()` 找不到任何符号。

**修复**：force_load 移到 `user_target_xcconfig`（作用于 Runner 的链接）：

```ruby
s.user_target_xcconfig = {
  'OTHER_LDFLAGS' => [
    '-force_load',
    '$(PODS_XCFRAMEWORKS_BUILD_DIR)/aimux/aimux_ffi.framework/aimux_ffi',
  ],
}
```

符号直接进 app 主二进制 → `dlsym(RTLD_DEFAULT)` 必中。

**验证时注意**：debug 模拟器构建把 app 代码编进 `Runner.debug.dylib`（不是 `Runner`），
`nm Runner.app/Runner` 查错文件了——必须扫描 `Runner.app` 里所有可执行文件。

### 坑 5：CocoaPods 的 xcframework 解包脚本未挂载

**症状**：链接报 `Framework 'aimux_ffi' not found`。

**根因**：Flutter 项目自定义了 xcconfig（base configuration），
CocoaPods 的自动 xcframework 解包脚本（`Pods-*-frameworks.sh`）没被挂载到 target。
vendored framework 从未进入链接输入。

**修复**：podspec 加 `script_phase`（before_compile），自己完成 slice 暂存：

```ruby
s.script_phase = {
  :name => 'Stage aimux_ffi xcframework slice',
  :script => 'bash "$PODS_TARGET_SRCROOT/embed-xcframework.sh"',
  :execution_position => :before_compile,
  :input_files => ['${PODS_TARGET_SRCROOT}/aimux_ffi.xcframework/Info.plist'],
  :output_files => ['${PODS_XCFRAMEWORKS_BUILD_DIR}/aimux/aimux_ffi.framework/aimux_ffi'],
}
```

`embed-xcframework.sh` 按 `SDK_NAME` 选择 slice（真机/模拟器），复制到
`PODS_XCFRAMEWORKS_BUILD_DIR/aimux/`（该目录已在 app 的 FRAMEWORK_SEARCH_PATHS 里）。

### 坑 6：xcframework 必须用 framework 型 slice（不是裸 .a）

**症状**：用 `xcodebuild -create-xcframework -library` 生成的裸 `.a` slice，
CocoaPods 链接报 `Framework 'aimux_ffi' not found`。

**根因**：CocoaPods 的 `vendored_frameworks` 期望 framework 型 slice
（`.framework` 目录结构），裸 `.a` 的 xcframework 不被正确解析。

**修复**：`scripts/build-ios-xcframework.sh` 用 `-framework` 参数
（把 `.a` 包成 `.framework` 目录结构后再 create-xcframework）。

### 坑 7：CI 符号检查查错文件

**症状**：CI 的 `nm -g Runner.app/Runner | grep aimux` 总是空，
但构建实际成功了。

**根因**：Flutter debug 模拟器构建把 app 代码编进 `Runner.debug.dylib`
（链接命令 `-o Binary/Runner.debug.dylib`），`Runner.app/Runner` 只是薄壳。

**修复**：扫描 `Runner.app` 里所有可执行文件：

```bash
find build/ios/iphonesimulator/Runner.app -type f -perm -111 |
while read f; do
  nm -g "$f" 2>/dev/null | grep -q 'aimux_openai_new' && echo "FOUND in $f"
done
```

### 坑 8：CI 管道吞退出码（假阳性 pass）

**症状**：CI job 显示 pass，但实际构建失败或符号检查未执行。

**根因**：`flutter build ... | tee log || true` 和 `grep ... | head` 等管道
吞掉了 `flutter build` 的真实退出码。`bash -e` 只看管道最后一个命令的退出码。

**修复**：
- 符号检查放在诊断步骤**之前**（避免诊断的 `grep` exit 2 中断后续检查）
- 诊断步骤加 `|| true`（不阻断）
- `set -o pipefail` + `tee`（保留退出码）

### 坑 9：pub.dev publisher 未绑定

**症状**：包发布成功但 pub.dev 页面没有 verified publisher 徽章
（`publisher: null`）。

**根因**：`dart pub publish` 用 OAuth 凭证发布，不携带 publisher 选择。
publisher 绑定需要在 pub.dev 网页操作。

**修复**：发布后到 https://pub.dev/packages/<package>/admin
→ "Select a publisher" 下拉 → 选择 verified publisher → 确认。
（**不可逆操作**：转移后只有 publisher 成员能上传新版本）

### 坑 10：flutter-ffi-mobile artifact 路径

**症状**：flutter-publish 的 "Embed iOS xcframework" 步骤失败：
`test -f mobile-artifacts/ios/aimux_ffi.xcframework/Info.plist` 找不到文件。

**根因**：flutter-ffi-mobile (ios) job 的 `upload-artifact` 上传的是 `staging/` 内容
（`path: staging/`），下载后 artifact 在根目录（`mobile-artifacts/aimux_ffi.xcframework`），
没有 `ios/` 前缀。Android 保留了 `android/` 前缀是因为 `staging/android/` 是上传根。

**修复**：embed 步骤的路径改为 `mobile-artifacts/aimux_ffi.xcframework`（无 `ios/` 前缀）。

### 坑 11：SwiftPM 拒绝混合语言 target

**症状**：SPM 路径下 `target ... contains mixed language source files; feature not supported`。

**根因**：SwiftPM 不允许同一个 target 同时包含 `.swift` 和 `.c` 文件。

（此问题在放弃 SPM 路径后不再相关，但记录以防未来重试 SPM 时踩坑。）

---

## 四、Node.js / npm 发布

### 坑 1：committed index.js 过期 — TS2305

**症状**：`npm run build:typed`（tsc）报
`error TS2305: Module '"../index.js"' has no exported member 'initLogging'`。

**根因**：`bindings/node/index.js` 是 napi 的构建产物，commit 在仓库里。
RFC-0014 在 `lib.rs` 新增了 `init_logging`，typed wrapper（`index.ts`）引用了它，
但 committed index.js 是旧版（没有 `initLogging` 导出）。
node-publish job 只跑 `tsc` + 下载 platform binaries，没有重新 `napi build`。

**修复**：node-publish job 在 tsc 前加 `napi build` 步骤（带 rust-toolchain）：

```yaml
- uses: dtolnay/rust-toolchain@stable
- name: Build napi index.js + d.ts (regenerate from current lib.rs)
  working-directory: bindings/node
  run: npm run build
```

**长期建议**：考虑把 `index.js` / `index.d.ts` 从仓库移除（`.gitignore`），
改为 CI 构建时生成——避免 committed 产物过期问题。

---

## 五、PyPI 发布

### 无坑（OIDC trusted publishing 一次通过）

配置要点（release-keys-guide.md §1 已详述）：
- PyPI 上配置 Trusted Publisher（GitHub owner/repo/workflow name）
- Environment name **留空**（python-release job 没有 environment 字段）
- 不需要 API token

---

## 六、crates.io 发布

### 无坑（幂等发布）

release.yml 的 `rust-publish` job 对每个 crate 先 curl 查 crates.io 是否已存在该版本，
已存在则跳过——所以重跑安全。

---

## 七、GitHub Release

### 坑 1：workflow_dispatch 没有 tag ref

**症状**：手动触发 `gh workflow run release.yml` 时，`softprops/action-gh-release`
找不到 tag。

**根因**：`workflow_dispatch` 运行没有关联的 git tag（不像 `push: tags` 触发）。

**修复**（已在 release.yml 中）：从 `Cargo.toml` 解析版本号，构造 tag name：

```yaml
- name: Resolve version
  run: |
    VERSION=$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
    echo "tag=v${VERSION}" >> "$GITHUB_OUTPUT"
```

### 坑 2：tag 被提前占用

**症状**：想重打 `v0.2.1` tag 触发全量发布，但 tag 已存在（workflow_dispatch 的
github-release job 自动创建了）。

**修复**：直接用 `workflow_dispatch` 分通道跑（不需要 tag），或删旧 release + tag 后重打。

---

## 八、CI 调试方法论

### 教训 1：诊断命令不要阻断主流程

诊断 `grep` / `find` 返回非零退出码时，`bash -e` 会中断整个步骤——
符号检查可能根本没跑到。诊断命令一律加 `|| true`，符号检查放在最后。

### 教训 2：管道吞退出码

`cmd | tee log || true` 会让 `bash -e` 看到 `true` 的退出码（0），
即使 `cmd` 失败了 job 也显示 pass。用 `set -o pipefail` 或去掉 `|| true`。

### 教训 3：查对文件

Flutter debug 构建的产物是 `Runner.debug.dylib`，不是 `Runner.app/Runner`。
nm 检查必须扫描整个 `Runner.app`，不能只查主可执行文件。

### 教训 4：用 `xcodebuild -showBuildSettings` + `-v` 日志

不确定 build setting 是否生效时，用 `xcodebuild -showBuildSettings` 查解析后的值，
用 `flutter build -v` 查实际链接命令——不要猜。

---

## 九、发布前检查清单

发布新版本前逐项确认：

- [ ] 版本号全链路一致（Cargo.toml / package.json / pyproject.toml / pubspec.yaml / build.gradle.kts × 2）
- [ ] docs 中版本引用已更新（README + docs/api/*.md + release-keys-guide.md）
- [ ] CHANGELOG 已更新
- [ ] `git status` 干净（无未提交改动，无 CRLF 噪声）
- [ ] CI 全绿（含 Flutter example build iOS + Android）
- [ ] `flutter pub publish --dry-run` 通过
- [ ] GitHub secrets 有效（SIGNING_KEY 未损坏——用 python 字节级验证）
- [ ] Sonatype namespace 已获批
- [ ] pub.dev publisher 已绑定（首次发布后手动 transfer）
- [ ] PyPI trusted publisher 已配置

---

## 十、快速重发布命令

```bash
# 全通道（打 tag 触发）
git tag v0.2.x
git push origin v0.2.x

# 分通道（workflow_dispatch，幂等）
gh workflow run release.yml -f rust=true -f node=true -f python=true \
  -f jvm=true -f flutter=true -f artifacts=true

# 单通道试跑
gh workflow run release.yml -f jvm=true   # 只发 Maven
gh workflow run release.yml -f flutter=true -f artifacts=true  # 只发 pub.dev + 制品

# 查看发布状态
gh run list --limit 5
gh run watch <run-id>

# 验证发布结果
python -c "import urllib.request, json; print(json.load(urllib.request.urlopen('https://crates.io/api/v1/crates/aimux-core'))['versions'][0]['num'])"
python -c "import urllib.request, json; print(json.load(urllib.request.urlopen('https://registry.npmjs.org/@arcships%2Faimux'))['dist-tags']['latest'])"
python -c "import urllib.request, json; print(json.load(urllib.request.urlopen('https://pub.dev/api/packages/aimux'))['versions'][-1]['version'])"
python -c "import urllib.request, json; print(json.load(urllib.request.urlopen('https://pypi.org/pypi/arcships-aimux/json'))['info']['version'])"
```
