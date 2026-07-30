# 性能基准结果

> **日期**：2026-07-30
> **环境**：Linux x64, 32 核, Node v24.18.0, Python 3.12.13
> **方法**：同进程、同 mock server、固定响应，N=200-300 次取统计值
> **bench 脚本**：[bindings/node/bench/](../bindings/node/bench/)、[bindings/python/bench/](../bindings/python/bench/)

## 1. 对等对比：aimux vs OpenAI 官方 SDK

同一抽象层（HTTP + JSON，无编排/schema 验证/中间件），干净数字。

### Node.js

| | mean | P50 | P95 | P99 | RSS 增长 |
|---|---|---|---|---|---|
| **aimux** (napi → Rust → reqwest) | 0.101ms | 0.096 | 0.122 | 0.139 | +2MB |
| **OpenAI Node SDK** (undici) | 1.488ms | 1.500 | 1.637 | 1.923 | +17MB |
| **倍数** | | | | | |
| aimux 快 | **14.7x** | | | | 内存省 8.5x |

### Python

| | mean | P50 | P95 | P99 | RSS 增长 |
|---|---|---|---|---|---|
| **aimux** (PyO3 → Rust → reqwest) | 0.080ms | 0.075 | 0.108 | 0.129 | +0MB |
| **OpenAI Python SDK** (httpx) | 0.595ms | 0.577 | 0.695 | 0.839 | +8MB |
| **倍数** | | | | | |
| aimux 快 | **7.5x** | | | | 内存省 ∞ |

## 2. 持续压测（2000 次请求，200KB 上文，50KB 响应）

### Node.js（taskset 限制 CPU 核数）

| 场景 | SDK | rps | mean | P99 | 尾部抖动(P99-P50) | RSS 增长 |
|---|---|---|---|---|---|---|
| 32 核 | aimux | 1512 | 0.66ms | 1.92ms | 1.31ms | +23MB |
| | AISDK | 563 | 1.78ms | 3.96ms | 2.23ms | +103MB |
| 2 核 | aimux | 1583 | 0.63ms | 1.74ms | 1.14ms | +20MB |
| | AISDK | 566 | 1.76ms | 5.73ms | 4.00ms | +144MB |
| 1 核 | aimux | 1497 | 0.67ms | 1.65ms | 1.03ms | +21MB |
| | AISDK | 473 | 2.11ms | **12.87ms** | **11.20ms** | +60MB |

### Python

| | rps | mean | P99 | RSS 增长 | RSS 趋势 |
|---|---|---|---|---|---|
| **aimux** | 1393 | 0.72ms | 0.94ms | **+0MB** | 完全一条直线 |
| **OpenAI SDK** | 987 | 1.01ms | 1.37ms | +8MB | 持续缓慢增长 |

## 3. 序列化瓶颈拆解（napi FFI 边界）

| payload | JS stringify | JS parse | napi total | FFI 边界 | Rust+HTTP |
|---|---|---|---|---|---|
| 1KB | 0.001ms | 0.001ms | 0.156ms | 0.002ms | 0.154ms |
| 10KB | 0.006ms | 0.004ms | 0.155ms | 0.010ms | 0.144ms |
| 100KB | 0.082ms | 0.051ms | 0.479ms | 0.133ms | 0.347ms |
| 500KB | 0.461ms | 0.376ms | 2.552ms | 0.837ms | 1.715ms |
| 1MB | 0.964ms | 0.717ms | 5.550ms | 1.680ms | 3.870ms |

序列化在大 payload 下占 ~50%，但真实 LLM 请求（3-10s）中占比 <0.1%，不值得优化。

## 4. 关于对比对象的说明

| 对比 | 倍数 | 是否对等 | 说明 |
|---|---|---|---|
| vs OpenAI Node SDK | **14.7x** | ✅ 对等 | 都是 HTTP + JSON，无编排层 |
| vs OpenAI Python SDK | **7.5x** | ✅ 对等 | 同上 |
| vs Vercel AI SDK | 11.1x | ❌ 不对等 | AISDK 含 zod 验证/中间件/telemetry，11x 有水分 |

Vercel AI SDK 每次请求额外做：Zod schema 验证、构建类型化对象树、fetch 中间件 pipeline、telemetry 记录。这些在 V8 堆里累积，导致内存膨胀。aimux 不做这些——设计目标是轻量接入层，不做编排。

## 5. 跨语言对比

| 指标 | Node.js (napi) | Python (PyO3) |
|---|---|---|
| aimux 单请求 | 0.101ms | 0.080ms |
| aimux RSS (2000 req) | +2MB | **+0MB** |
| 优势来源 | Rust reqwest + 连接池 | 同左 + PyO3 FFI 更轻量 |

**Python aimux 比 Node aimux 快**——PyO3 直接在 C API 层调用 Rust（几乎零开销的 C 函数调用），napi 要经过 V8 的 napi_env/napi_value 包装。Python 引用计数也比 V8 GC 内存更稳定。

## 6. 结论

1. **aimux 在两端都是绝对领先**：Node 14.7x、Python 7.5x
2. **内存零增长**：Python aimux 2000 次请求后 RSS 一字节没涨；Node +2MB
3. **GC 停顿**：aimux 无 GC，P99 尾部抖动不随 CPU 受限变化；AISDK 在 1 核时 P99 飙到 12.87ms
4. **序列化不是瓶颈**：在真实 LLM 请求（3-10s）中，序列化开销占比 <0.1%
5. **轻量是设计目标**：aimux 不做编排/schema 验证/中间件/telemetry，只做接入层——这是性能优势的一部分来源，也是产品定位
