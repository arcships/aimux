# Provider 统计与提取结果

数据来自 6 个本地参考仓库中的 7 类目录/常量源，按 canonical provider ID 去重。

## 汇总

- 原始 provider 记录：**650**
- 去重后 provider：**325**
- aimux 原始模块：**172**
- 与 aimux 模块直接匹配：**116**
- 未与 aimux 模块直接匹配：**209**
- 含 endpoint 元数据：**190**
- 含 model 元数据：**262**

## 来源记录数

| 来源 | 记录数 |
|---|---:|
| litellm_constants | 110 |
| litellm_prices | 122 |
| mastra_registry | 160 |
| new_api | 57 |
| pydantic_ai | 17 |
| rust_genai | 29 |
| tokenhub | 155 |

## 兼容层级（自动提取/推断）

| 层级 | 数量 |
|---|---:|
| L1 | 9 |
| L2 | 65 |
| L3 | 27 |
| L4 | 5 |
| unknown | 219 |

## 输出文件

- `providers.json`：325 个去重 provider 的完整结构化数据。
- `providers.csv`：便于筛选和表格处理的扁平清单。
- `raw-provider-records.jsonl`：650 条未去重来源记录，保留来源追踪。

## 开发入口

新增或重做 provider 时，按 [RFC-0006：Provider 开发规范](../rfc/0006-provider-development.md) 核验本次能力所需的身份与协议，选择实现路径并完成对应测试。清单字段只用于候选筛选和来源追踪，不能替代厂商官方协议文档。

> “未与 aimux 模块直接匹配”只表示 canonical ID 没有同名模块，不等同于确认缺失实现；别名和聚合入口仍需人工核对。
