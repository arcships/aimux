//! 人可读报告渲染(text)与 JSON 直出。CLI 只消费 core 的数据类型,
//! 渲染逻辑是本 crate 的全部"业务"。

use aimux_core::trace::{SessionChainView, TraceStats, VerdictKind};

/// 渲染聚合统计(offline 子命令)。
pub fn render_stats_text(stats: &[TraceStats]) -> String {
    if stats.is_empty() {
        return "no trace records matched".to_string();
    }
    let mut out = String::new();
    for s in stats {
        out.push_str(&format!("\n== {} / {} ==\n", s.provider, s.model));
        out.push_str(&format!("  requests:                   {}\n", s.requests));
        out.push_str(&format!(
            "  input tokens total:         {}\n",
            s.input_tokens_total
        ));
        out.push_str(&format!(
            "  claimed cache read:         {} ({:.2}%)\n",
            s.claimed_cache_read_total,
            s.reported_hit_rate.unwrap_or(0.0) * 100.0
        ));
        out.push_str(&format!(
            "  claimed cache write:        {}\n",
            s.claimed_cache_write_total
        ));
        match s.client_upper_bound_hit_rate {
            Some(ub) => out.push_str(&format!(
                "  client LCP upper bound:     {:.2}%\n",
                ub * 100.0
            )),
            None => out.push_str("  client LCP upper bound:     <no LCP evidence>\n"),
        }
        if s.ttft_p50_ms.is_some() || s.ttft_p95_ms.is_some() {
            out.push_str(&format!(
                "  ttft p50/p95 (ms):          {}/{}\n",
                s.ttft_p50_ms
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
                s.ttft_p95_ms
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
            ));
        }
        out.push_str(&format!("  errors:                     {}\n", s.errors));
        if !s.verdict_counts.is_empty() {
            out.push_str("  verdicts:\n");
            for (kind, count) in &s.verdict_counts {
                out.push_str(&format!("    {kind:<22} {count}\n"));
            }
        }
        // 掺水嫌疑(High)高亮提示。
        let overclaim_high = s
            .verdict_counts
            .get(kind_name(VerdictKind::SuspectOverclaim))
            .copied()
            .unwrap_or(0);
        if overclaim_high > 0 {
            out.push_str(&format!(
                "  ⚠ {overclaim_high} overclaim suspect(s) — reported hits exceed the client LCP bound\n"
            ));
        }
    }
    out
}

/// 渲染会话链诊断(session 子命令)。
pub fn render_chain_text(chain: &SessionChainView) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "\nsession {}\n  records: {}\n  prefix stability: {:.3}\n",
        chain.session_id,
        chain.record_ids.len(),
        chain.prefix_stability
    ));
    if chain.breaks.is_empty() {
        out.push_str("  breaks: none (append-only chain)\n");
    } else {
        out.push_str(&format!("  breaks: {}\n", chain.breaks.len()));
        for b in &chain.breaks {
            out.push_str(&format!(
                "    at {} (after {}): lcp={}B kind={:?} expected={}\n",
                b.at_record_id, b.prev_record_id, b.lcp_bytes, b.kind, b.expected_break
            ));
        }
    }
    if chain.prefix_stability < 0.5 && !chain.breaks.is_empty() {
        out.push_str(
            "  ⚠ low prefix stability — the session's prompt prefix keeps changing;\n\
               provider-side cache hits beyond the shared prefix are suspect.\n",
        );
    }
    out
}

pub fn kind_name(kind: VerdictKind) -> &'static str {
    match kind {
        VerdictKind::Trusted => "Trusted",
        VerdictKind::SuspectOverclaim => "SuspectOverclaim",
        VerdictKind::SuspectUnderclaim => "SuspectUnderclaim",
        VerdictKind::Unknown => "Unknown",
    }
}
