//! `offline` 子命令:读 TraceRecord jsonl → 审计统计报告。
//!
//! 薄 client:反序列化 → 灌入 `RingTraceStore`(复用其 aggregate 统计)→ 渲染。

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use aimux_core::trace::{RingTraceStore, TraceFilter, TraceRecord, TraceSink};

use crate::report;
use crate::{Format, OfflineArgs};

/// 读 jsonl,每行一个 TraceRecord。坏行跳过并计数(容错:文件可能被截断)。
pub(crate) fn load_records(path: &Path) -> anyhow::Result<(Vec<TraceRecord>, usize)> {
    let file =
        File::open(path).map_err(|e| anyhow::anyhow!("cannot open {}: {e}", path.display()))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut skipped = 0usize;
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TraceRecord>(&line) {
            Ok(rec) => records.push(rec),
            Err(e) => {
                skipped += 1;
                eprintln!("warning: line {} is not a TraceRecord: {e}", idx + 1);
            }
        }
    }
    Ok((records, skipped))
}

/// 把记录灌入 RingTraceStore(复用 core 的聚合/链查询,CLI 不重复实现)。
fn index_records(records: &[TraceRecord]) -> RingTraceStore {
    let store = RingTraceStore::with_capacity(records.len().max(1), records.len().max(1));
    for rec in records {
        store.record(rec.clone());
    }
    store
}

pub fn run(args: &OfflineArgs) -> anyhow::Result<()> {
    let (records, skipped) = load_records(&args.file)?;
    if records.is_empty() {
        eprintln!(
            "no TraceRecords in {} ({} line(s) skipped)",
            args.file.display(),
            skipped
        );
        return Ok(());
    }
    let store = index_records(&records);

    let filter = TraceFilter {
        provider: args.provider.clone(),
        model: None,
        session_id: args.session.clone(),
        since_unix_ms: None,
    };
    let stats = store.aggregate(&filter);

    match args.format {
        Format::Text => {
            print!("{}", report::render_stats_text(&stats));
            if skipped > 0 {
                eprintln!("({skipped} malformed line(s) skipped)");
            }
        }
        Format::Json => {
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn sample_record(session: &str, claimed: u64) -> TraceRecord {
        // 最小 TraceRecord(其余字段默认/空指纹——offline 只消费统计字段)。
        serde_json::from_str(&format!(
            r#"{{
                "provider": "openai",
                "model": "gpt-4o",
                "session_id": "{session}",
                "trace_id": "t-{session}-{claimed}",
                "sent_at_unix_ms": 1785900000000,
                "monotonic_sent_ms": 0,
                "fingerprint": {{
                    "body_hash": "",
                    "len_bytes": 8192,
                    "block_size": 4096,
                    "block_hashes": [],
                    "token_estimate": 2048
                }},
                "usage": {{
                    "input_total": 2048,
                    "cache_read": {claimed}
                }},
                "verdict": {{
                    "kind": "Trusted",
                    "confidence": "High",
                    "violated": [],
                    "expected_max": 2048,
                    "claimed": {claimed},
                    "lcp_bytes": 8192,
                    "notes": []
                }},
                "scope_key": 1
            }}"#
        ))
        .unwrap()
    }

    #[test]
    fn offline_reads_and_aggregates() {
        let dir = std::env::temp_dir().join(format!("aimux-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("trace.jsonl");
        let mut f = File::create(&path).unwrap();
        for rec in [sample_record("s1", 512), sample_record("s1", 1024)] {
            writeln!(f, "{}", serde_json::to_string(&rec).unwrap()).unwrap();
        }
        // 一行坏数据(容错)。
        writeln!(f, "{{not json").unwrap();
        drop(f);

        let args = OfflineArgs {
            file: path.clone(),
            provider: None,
            session: None,
            format: Format::Json,
        };
        // 捕获 stdout 断言 JSON 输出。
        let out = run_captured(&args);
        assert!(out.contains("\"requests\":2"), "aggregate count: {out}");
        assert!(out.contains("\"claimed_cache_read_total\":1536"), "{out}");
        assert!(out.contains("\"provider\":\"openai\""), "{out}");

        // session 过滤。
        let args2 = OfflineArgs {
            file: path,
            provider: None,
            session: Some("nope".into()),
            format: Format::Json,
        };
        let out2 = run_captured(&args2);
        assert!(
            out2.contains("[]"),
            "no matches for unknown session: {out2}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 复用 run 的聚合逻辑返回 JSON(测试断言用)。
    fn run_captured(args: &OfflineArgs) -> String {
        let (records, _skipped) = load_records(&args.file).unwrap();
        let store = index_records(&records);
        let filter = TraceFilter {
            provider: args.provider.clone(),
            model: None,
            session_id: args.session.clone(),
            since_unix_ms: None,
        };
        let stats = store.aggregate(&filter);
        serde_json::to_string(&stats).unwrap()
    }
}
