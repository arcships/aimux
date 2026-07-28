# Skills/File 上传与 Codemod 迁移

> 参考源码：`reference/ai/packages/ai/src/upload-skill/`、`upload-file/`、`packages/provider/src/skills/`、`files/`、`shared/`、`packages/codemod/`
> 复核状态：✅ 已复核（详见文末复核记录）

## 1. Skills/File 上传 surface

### 概述

Skills 和 Files 是 provider 规范中的两个**可选上传通道**，与 8 种模型类型（language/embedding/image/reranking/transcription/speech/video/realtime）并列。在 `ProviderV4` 接口中作为可选方法挂载：`files?()`（L88）、`skills?()`（L94）。即 provider 共 8 个 surface：6 模型 + 2 上传通道，后者均为可选。

### SkillsV4 接口

[skills-v4.ts:7-28](../reference/ai/packages/provider/src/skills/v4/skills-v4.ts#L7)：

```ts
interface SkillsV4 {
  specificationVersion: 'v4';
  provider: string;
  uploadSkill(params: SkillsV4UploadSkillCallOptions): PromiseLike<SkillsV4UploadSkillResult>;
}
```

调用选项 [skills-v4-upload-skill-call-options.ts:22-36](../reference/ai/packages/provider/src/skills/v4/skills-v4-upload-skill-call-options.ts#L22)：`files`、`displayTitle`、`providerOptions`。结果 [skills-v4-upload-skill-result.ts:5-39](../reference/ai/packages/provider/src/skills/v4/skills-v4-upload-skill-result.ts#L5)：含 `providerReference`、`warnings`、`name`/`description`/`latestVersion`。

### uploadSkill AI 函数

[upload-skill.ts:17-53](../reference/ai/packages/ai/src/upload-skill/upload-skill.ts#L17)：

```ts
uploadSkill({ api: SkillsV4 | ProviderV4, files: UploadSkillFile[], displayTitle?, providerOptions? }): Promise<UploadSkillResult>
```

工作流程：
1. 解析 `api`：若 `'uploadSkill' in api` 直接用，否则尝试 `api.skills()`，否则抛 "provider does not support skills"（L27-36）
2. 规范化 `files`：简写 `Uint8Array | string` 统一成 `{ type: 'data', data }`（L38-44）
3. 调用 `skillsApi.uploadSkill({ files, displayTitle, providerOptions })` 原样返回（L46-50）

结果类型把 provider 规范的 `providerReference`/`warnings` 重映射为 SDK 本地 `ProviderReference`/`Warning`。

### FilesV4 接口

[files-v4.ts:7-25](../reference/ai/packages/provider/src/files/v4/files-v4.ts#L7)：平行结构，唯一方法 `uploadFile(options): PromiseLike<FilesV4UploadFileResult>`。

### uploadFile AI 函数

[upload-file.ts:27-89](../reference/ai/packages/ai/src/upload-file/upload-file.ts#L27)：

```ts
uploadFile({ api: FilesV4 | ProviderV4, data, mediaType?, filename?, providerOptions? }): Promise<UploadFileResult>
```

工作流程：
1. 简写 `data` 规范化为 tagged 形状（L52-55）
2. 自动推断 `mediaType`：`text` 变体→`text/plain`；`data` 变体先调 `detectMediaType`，再回退 `isLikelyText` 启发式（前 512 字节扫描非可打印字节），最终回退 `application/octet-stream`（L57-62, L113-141）
3. 同样的 `api.files()` 解析模式（L64-73）
4. 调用 `filesApi.uploadFile(...)` 后包成 `DefaultUploadFileResult`（L82-111）

### SharedV4ProviderReference 在消息中的引用

[shared-v4-provider-reference.ts:20-22](../reference/ai/packages/provider/src/shared/v4/shared-v4-provider-reference.ts#L20)：`Record<string, string> & { type?: never }`——provider 名→provider 内文件/skill id 的映射。`type?: never` 防止与 tagged file-data 形状混淆。

引用链路：
- 上传后进入 `SharedV4FileDataReference`（`{ type: 'reference', reference }`），成为 `SharedV4FileData` 联合的一支
- [file-part-data.ts:92-122](../reference/ai/packages/ai/src/prompt/file-part-data.ts#L92) 的 `convertToLanguageModelV4FilePart` 把 `{ type: 'reference', reference }` 透传给 language model prompt
- UI 消息文件 part 上的 `providerReference?: ProviderReference` 字段（"When present, takes precedence over `url` in model messages"）允许 UI 消息直接携带引用，无需重新上传
- 无法解析时抛 `NoSuchProviderReferenceError`

## 2. codemod 迁移 CLI

### 入口与命令

[package.json:44-46](../reference/ai/packages/codemod/package.json#L44) bin：`codemod → ./dist/bin/codemod.js`。[codemod.ts:20-105](../reference/ai/packages/codemod/src/bin/codemod.ts#L20) 用 `commander` 构建 5 个命令：

| 命令 | 说明 |
|------|------|
| `upgrade` | 运行全部迁移脚本 |
| `v4`/`v5`/`v6`/`v7` | 按版本运行 |
| `<codemod> <source>` | 运行单个 codemod |

全局选项：`--dry/-d`、`--print/-p`、`--verbose`、`-j/--jscodeshift`（L22-31）。

### 迁移脚本

[upgrade.ts:6-123](../reference/ai/packages/codemod/src/lib/upgrade.ts#L6) 维护 `bundle` 数组（共 116 项，含 v4/v5/v6/v7），按前缀过滤出 `v4Bundle`…`v7Bundle`。`runCodemods` 用 `cli-progress` 进度条依次跑，收集 `errors` 与 `notImplementedErrors`（L134-180）。

每个脚本目录约定：`src/codemods/v{N}/{name}.ts` 默认导出一个 jscodeshift transformer。常见脚本：

| 脚本 | 作用 |
|------|------|
| `v4/remove-await-streamtext` | `removeAwaitFn('streamText')` — 剥掉 streamText 的 await |
| `v4/remove-openai-facade` | `removeFacade({ packageName:'openai', className:'OpenAI', createFnName:'createOpenAI' })` — `new OpenAI(...)` → `createOpenAI(...)` |
| `v4/replace-baseurl` | `baseUrl` → `baseURL` 重命名（自定义 transformer，L30-48） |
| `v6/add-await-converttomodelmessages` | `addAwaitFn('convertToModelMessages')` — 加 await |

运行示例：`npx @ai-sdk/codemod upgrade`、`npx @ai-sdk/codemod v4`、`npx @ai-sdk/codemod v4/replace-baseurl src/`。

### AST transformer 机制

依赖 [jscodeshift ^17.3.0](../reference/ai/packages/codemod/package.json#L70)（基于 recast + babel AST）。

- `transform()`（[transform.ts:95-138](../reference/ai/packages/codemod/src/lib/transform.ts#L95)）通过 `execSync` 调用本地 `jscodeshift` CLI，固定 `--parser tsx`，忽略 `node_modules`/`.*/`/`dist`/`build`/`*.min.js`/`*.bundle.js`（L18-52）
- stdout 正则解析 `ERR ... Transformation error` 与 `Not Implemented ...` 两类错误（L60-93）

三个可复用 helper：

| helper | 文件 | 作用 |
|--------|------|------|
| `createTransformer` | [create-transformer.ts:32-55](../reference/ai/packages/codemod/src/codemods/lib/create-transformer.ts#L32) | 构造 `TransformContext { j, root, hasChanges, messages }`，`hasChanges` 为真才 `root.toSource({ quote: 'single' })` |
| `addAwaitFn` | [add-await-fn.ts:3-48](../reference/ai/packages/codemod/src/codemods/lib/add-await-fn.ts#L3) | 从 `ai` 包找指定函数导入（含别名），给未 await 的调用包 `AwaitExpression` |
| `removeAwaitFn` | [remove-await-fn.ts:3-47](../reference/ai/packages/codemod/src/codemods/lib/remove-await-fn.ts#L3) | 反向操作，剥掉 `AwaitExpression` 还原 `CallExpression` |
| `removeFacade` | [remove-facade.ts:9-71](../reference/ai/packages/codemod/src/codemods/lib/remove-facade.ts#L9) | `@ai-sdk/{pkg}` 的 `new ClassName(...)` 重写为 `createFnName(...)` 调用并替换导入符号 |

新增 codemod 用 `pnpm scaffold <name>`（[scaffold-codemod.ts:42-83](../reference/ai/packages/codemod/scripts/scaffold-codemod.ts#L42)）生成模板并自动追加到 `bundle`。测试用 vitest + `__testfixtures__/{name}.input.ts|.output.ts` 快照对比。

---

## 复核记录

**复核员**：Vera | **复核日期**：2026-07-25

| # | 声明 | 结论 | 证据 |
|---|------|------|------|
| 1 | SkillsV4 接口 skills-v4.ts L7-28，单一方法 uploadSkill | ✅ | L7 interface 开始，L15 specVersion='v4'，L20 provider，L25-27 uploadSkill，L28 闭合 |
| 2 | uploadSkill AI 函数 L17-53，三步流程 | ✅（已修正） | 函数@L17-53 ✅；三步流程修正为 **L27-50**（调用步在 L46-50，原声明 L27-44 漏了调用步） |
| 3 | FilesV4 接口 files-v4.ts L7-25 | ✅ | L7 type 开始（实为 type 别名非 interface），L11 specVersion，L22-24 uploadFile，L25 闭合 |
| 4 | uploadFile AI 函数 L27-89，mediaType 自动推断 | ✅ | 函数@L27-89 ✅；mediaType@L57-62 ✅；isLikelyText@L113-141 ✅ |
| 5 | SharedV4ProviderReference L20-22 | ✅ | 逐字匹配 `Record<string,string> & { type?: never }` |
| 6 | codemod 5 命令，bundle ~120 项 | ✅（已修正） | 5 命名命令+默认动作@L33-103 ✅；upgrade.ts 路径修正为 `src/lib/upgrade.ts`；bundle 精确 **116 项** |
| 7 | jscodeshift ^17.3.0，transform.ts L95-138 | ✅ | package.json@L70 ✅；transform@L95-138 execSync + --parser tsx@L27 ✅ |
| 8 | addAwaitFn/removeAwaitFn/removeFacade 三个 helper | ✅（已修正） | 四函数均存在且功能正确；结束行号各 +1：createTransformer L32-55、addAwaitFn L3-48、removeAwaitFn L3-47、removeFacade L9-71 |
