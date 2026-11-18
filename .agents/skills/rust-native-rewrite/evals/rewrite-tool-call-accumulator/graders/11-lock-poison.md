---
type: llm
focus: { source: file, path: src/lib.rs }
---

如果还有日志锁，锁 poison 被转成错误返回：没有 .lock().unwrap()、.lock().expect(...)，也没有 PoisonError::into_inner；如果日志被整个去掉，也算通过
