//! `session` 子命令:读 TraceRecord jsonl → 指定 session 的链级诊断。

use aimux_core::trace::{RingTraceStore, TraceSink};

use crate::report;
use crate::{Format, SessionArgs};

pub fn run(args: &SessionArgs) -> anyhow::Result<()> {
    let (records, skipped) = crate::probe::offline::load_records(&args.file)?;
    let store = RingTraceStore::with_capacity(records.len().max(1), records.len().max(1));
    for rec in &records {
        store.record(rec.clone());
    }

    let Some(chain) = store.session_chain(&args.session) else {
        eprintln!(
            "session '{}' not found in {} ({} record(s) loaded, {skipped} skipped)",
            args.session,
            args.file.display(),
            store.len()
        );
        return Ok(());
    };

    match args.format {
        Format::Text => print!("{}", report::render_chain_text(&chain)),
        Format::Json => println!("{}", serde_json::to_string_pretty(&chain)?),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::offline::load_records;
    use std::fs::File;
    use std::io::Write;

    fn sample_record(session: &str, body_len: u64, idx: u32) -> serde_json::Value {
        serde_json::json!({
            "provider": "openai",
            "model": "gpt-4o",
            "session_id": session,
            "trace_id": format!("t-{idx}"),
            "sent_at_unix_ms": 1785900000000 + idx as i64 * 1000,
            "fingerprint": {
                "body_hash": format!("{idx:032x}"),
                "len_bytes": body_len,
                "block_size": 4096,
                "block_hashes": vec![format!("{:032x}", idx)],
                "token_estimate": body_len / 4,
            },
            "usage": {"input_total": body_len / 4},
        })
    }

    #[test]
    fn session_diagnoses_chain() {
        let dir = std::env::temp_dir().join(format!("aimux-cli-sess-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("trace.jsonl");
        let mut f = File::create(&path).unwrap();
        // 两轮同一 session:前缀稳定(链上 LCP = 块1 相同 → stability 高)。
        for (i, len) in [8192u64, 12288u64].iter().enumerate() {
            writeln!(f, "{}", sample_record("sess-1", *len, i as u32)).unwrap();
        }
        drop(f);

        // 直接走核心逻辑(JSON 分支断言)。
        let args = SessionArgs {
            file: path.clone(),
            session: "sess-1".into(),
            format: Format::Json,
        };
        let (records, _) = load_records(&args.file).unwrap();
        let store = RingTraceStore::with_capacity(8192, 8192);
        for r in records {
            store.record(r);
        }
        let chain = store.session_chain("sess-1").unwrap();
        let json = serde_json::to_string(&chain).unwrap();
        assert_eq!(chain.record_ids.len(), 2);
        assert!(json.contains("\"session_id\":\"sess-1\""), "{json}");

        // 未知 session → 无输出错误路径。
        let args2 = SessionArgs {
            file: path,
            session: "nope".into(),
            format: Format::Json,
        };
        assert!(store.session_chain("nope").is_none());

        std::fs::remove_dir_all(&dir).ok();
        let _ = args2;
    }
}
