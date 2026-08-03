# aimux 发布密钥配置手册（内部）

> 发布流水线 `release.yml` 的剩余渠道（PyPI / Maven Central / Go 二进制）需要
> 账号与密钥。本文是逐步操作手册。已配置完成的渠道（crates.io、npm）标注跳过。
>
> 更新时间：2026-08-03（v0.2.0 发布期）

## 总览

| 渠道 | 需要的凭证 | 放哪里 | 状态 |
|---|---|---|---|
| crates.io | `CARGO_REGISTRY_TOKEN` | GitHub env `release` | ✅ 已配置（0.1.5 发布过） |
| npm | `NPM_TOKEN` | GitHub env `release` | ✅ 已配置（0.1.5 发布过） |
| PyPI | Trusted Publisher（OIDC，**不需要 token**） | PyPI 网站配置 | ⏳ 待配置 |
| Maven Central | `OSSRH_USERNAME` / `OSSRH_PASSWORD` / `SIGNING_KEY` / `SIGNING_PASSWORD` | GitHub env `release` | ⏳ 待配置 |
| GitHub Release | 无需（workflow 自带 `GITHUB_TOKEN`） | — | ✅ |

> **注意**：`release.yml` 的 `rust-publish` / `node-publish` / `jvm-publish` 都声明了
> `environment: release`，所以 GitHub secrets **必须配在
> Settings → Environments → release → Environment secrets** 下，而不是
> 默认的 "Secrets and variables → Actions"（配错位置 job 读不到，表现为空值）。

---

## 1. PyPI（Trusted Publisher，推荐，无需 token）

`python-release` job 用 maturin + OIDC 发布，失败日志里的 `OIDC: http status 422`
就是 **PyPI 上还没有配置 Trusted Publisher**。

1. **注册 PyPI 账号**（已有则跳过）
   - 打开 https://pypi.org/account/register/ ，邮箱注册并验证。

2. **配置 Trusted Publisher**
   - 登录 https://pypi.org/ → 右上角头像 → **Account settings** → **Publishing** 区
     → **Add a new publisher**。
   - 填写（**逐字匹配，不能错**）：
     - Distribution project name：`aimux`
     - GitHub owner：`arcships`
     - GitHub repository：`aimux`
     - Workflow name：`release.yml`
     - Environment name：**留空**（`python-release` job 没有 `environment` 字段）
   - 保存后，publisher 显示 "Pending" 状态是正常的——**首次成功发布后自动变为 Active**。

3. **验证**
   - 触发发布：`gh workflow run release.yml --ref master -f python=true`
   - 成功后 `pip install aimux==0.2.0` 可安装。

> 不需要 PyPI API token，也不需要改 workflow。

---

## 2. Maven Central（Sonatype OSSRH）

`jvm-publish` job 失败日志：`Could not read PGP secret key` + 空白的
`ORG_GRADLE_PROJECT_signingKey`——**4 个 secrets 都没配**。

### 2.1 注册 Sonatype 账号

1. 打开 https://central.sonatype.com/ → **Sign in**（支持 GitHub/Google OAuth 或注册）。
2. 登录后右上角头像 → **View account** → 记下用户名/邮箱。

### 2.2 申请 `io.aimux` namespace（首次才需要）

`bindings/java` 和 `bindings/kotlin` 的 `build.gradle.kts` 里 `group = "io.aimux"`，
所以必须拥有 `io.aimux` 这个 namespace：

1. central.sonatype.com → **Publishing** → **Namespaces** → **Add Namespace**。
2. 输入 `io.aimux` → 按提示验证所有权（GitHub 仓库验证最方便：按要求在
   `arcships/aimux` 下加一个 CNAME/TXT 文件或创建指定名称的 repo）。
3. 等待审核通过（一般几小时内，有邮件通知）。

### 2.3 生成发布凭证（User Token）

1. central.sonatype.com → 头像 → **View account** → **User Token** 区
   → **Reset User Token**（或首次直接生成）。
2. 会得到一对 **username / password**（形如 `XXXXXXXX/YYYYYYYY`，冒号分隔）。
   - 这两个值就是 `OSSRH_USERNAME` / `OSSRH_PASSWORD`（不是你的登录密码！）

### 2.4 生成 PGP 签名密钥

发布到 Maven Central 的产物必须用 PGP 签名：

1. **安装 GnuPG**（Windows：https://www.gnupg.org/download/ ；macOS：`brew install gnupg`）。
2. 生成密钥（建议 RSA 4096，不设过期）：
   ```bash
   gpg --full-generate-key
   # 选择: (1) RSA and RSA, 4096, 0 = key does not expire
   # 填写姓名/邮箱（建议用注册 Sonatype 的邮箱），设置口令（passphrase，要记住）
   ```
3. 找到密钥 ID：
   ```bash
   gpg --list-secret-keys --keyid-format LONG
   # 输出里的 sec rsa4096/1A2B3C4D5E6F7089 —— 最后 16 位十六进制就是 key id
   ```
4. **导出私钥（ASCII armored，含 BEGIN/END 行）**：
   ```bash
   gpg --armor --export-secret-keys 1A2B3C4D5E6F7089
   # 输出形如:
   # -----BEGIN PGP PRIVATE KEY BLOCK-----
   # ...很多行...
   # -----END PGP PRIVATE KEY BLOCK-----
   ```
   复制**全部内容**（含首尾标记行）——这就是 `SIGNING_KEY`。
5. **上传公钥到 keyserver**（Maven Central 校验签名必需）：
   ```bash
   gpg --keyserver keyserver.ubuntu.com --send-keys 1A2B3C4D5E6F7089
   gpg --keyserver keys.openpgp.org --send-keys 1A2B3C4D5E6F7089
   ```

### 2.5 配置 GitHub secrets

路径：GitHub 仓库 → **Settings** → **Environments** → **release** →
**Environment secrets** → **Add secret**（注意不是默认的 Actions secrets）：

| Secret 名 | 值 |
|---|---|
| `OSSRH_USERNAME` | Sonatype User Token 的用户名部分 |
| `OSSRH_PASSWORD` | Sonatype User Token 的密码部分 |
| `SIGNING_KEY` | 2.4 步导出的完整 PGP 私钥（多行，含 `-----BEGIN/END PGP PRIVATE KEY BLOCK-----`） |
| `SIGNING_PASSWORD` | 生成密钥时设置的口令 |

### 2.6 验证

```bash
gh workflow run release.yml --ref master -f jvm=true
# 成功后 Maven Central 上出现 io.aimux:aimux-java:0.2.0 与 aimux-kotlin:0.2.0
# （首次发布在 Staging 仓库，需在 central.sonatype.com 手动 Release 一次；
#   之后的发布自动处理——取决于 build.gradle.kts 的自动发布配置）
```

---

## 3. crates.io（若 `CARGO_REGISTRY_TOKEN` 缺失才需要）

> 0.1.5 已成功发布过，此 token 应已存在。仅当 `rust-publish` job 报 401/403 时执行：

1. https://crates.io/ → GitHub 登录 → 头像 → **Account settings** → **API Tokens** → **New token**。
2. 权限建议选 "New crate + existing crate"（`publish-new` + `publish-update`）。
3. 复制 token（只显示一次），配到 GitHub env `release` 的 `CARGO_REGISTRY_TOKEN`。

---

## 4. npm（若 `NPM_TOKEN` 缺失才需要）

> 0.1.5 已成功发布过，此 token 应已存在。仅当 `node-publish` job 报 401/403 时执行：

1. https://www.npmjs.com/signup 注册（若已有账号，确保是 `@arcships` org 的成员）。
2. 头像 → **Access Tokens** → **Generate New Token**：
   - 类型选 **Granular Access Token**，权限：`@arcships/aimux` 包的 **Read and write**；
   - 或 **Classic Token** 勾选 **Publish** 权限。
3. 复制 `npm_` 开头的 token，配到 GitHub env `release` 的 `NPM_TOKEN`。

---

## 5. 补发剩余渠道

配置完成后，按需触发（每个 `-f` 只跑对应 job）：

```bash
# PyPI（3 个 OS 的 wheel）
gh workflow run release.yml --ref master -f python=true

# Maven Central（Java + Kotlin）
gh workflow run release.yml --ref master -f jvm=true

# Go 静态库 + C ABI 库 + GitHub Release（无需任何 key，随时可跑）
gh workflow run release.yml --ref master -f artifacts=true
```

注意：**不要**同时勾选 `rust=true` / `node=true` 重跑（crates.io / npm 已发布，
npm 重复 publish 同版本会报错；crates.io 有幂等跳过但也无必要）。
