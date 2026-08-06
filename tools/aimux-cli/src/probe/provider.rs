//! `provider` 子命令:在线探测指定 provider 的缓存能力。
//!
//! 直接调 provider 发测试请求(固定模板 + 每轮追加对话内容,验证前缀命中),
//! 挂 `TraceLayer` + 内置 auditor,输出缓存能力报告。**消耗真实 API 费用**——
//! 默认 4 次请求,可用 `--max-requests` 限制。

use std::sync::Arc;

use aimux_core::trace::{RingTraceStore, TraceFilter, TraceLayer, VerdictKind};
use aimux_providers::ProviderOptions;

use crate::report;
use crate::{Format, ProviderArgs};

/// 默认测试 system 模板:固定长文本(>1024 token 以触发多数 provider 的缓存
/// 门槛;内容为确定性文本,相同前缀跨轮稳定)。可被 `--prompt` 覆盖。
const DEFAULT_SYSTEM: &str = r#"You are a careful technical analyst. Keep answers precise and cite numbers.
Context: the quick brown fox jumps over the lazy dog; the five boxing wizards jump quickly;
pack my box with five dozen liquor jugs; how vexingly quick daft zebras jump; sphinx of black
quartz judge my vow; the jay pig fox zebra and my wolves quack; the public was amazed by the
quickness of the jumping fox; the vixen's crafty attack broke the cobweb's spell; watching the
jabbering dragonfly pass over the heather's edge; the glib jocks quiz nymphs for vexing blobs
of jade; the bawdy soldiers spin a pregnant yarn; the quick brown fox jumps over the lazy dog
once more; a mad boxer shot a quick gloved jab to the jaw of his dizzy opponent; the job
requires extra pluck and zeal from every young wage earner; two driven jocks help fax my big
quiz; five quacking zephyrs jolt my wax bed; the seven dwarfs are always happy; crazy Fredericka
bought many very exquisite opal jewels; the wizard's jinx made the zebra stagger backward;
each of these pangrams uses every letter of the alphabet at least once; I quickly explained
that many big jobs involve few hazards; she held the book tightly in her hands as the wind
whipped through the open window; the flight of the bumblebee is aerodynamically impossible
but nobody told the bee; a large fawn jumped quickly over white zinc boxes; an orange fox and
a blue rabbit raced across the green field; the pianist played a beautiful melody that echoed
through the concert hall; the scientist presented her groundbreaking research on quantum
entanglement to the international conference; the mountain climber reached the summit just
before the storm arrived; the chef prepared a seven-course meal using only locally sourced
ingredients; the historian discovered a forgotten manuscript in the monastery library;
the engineer designed a bridge that could withstand earthquakes of magnitude eight;
the astronomer photographed a distant galaxy with her powerful telescope; the marathon runner
crossed the finish line after four hours of relentless effort; the poet composed verses about
the changing seasons and the passage of time; the teacher explained the theory of relativity
using simple analogies that her students could understand; the detective examined every clue
at the crime scene with meticulous attention to detail; the gardener tended to her roses
every morning before the sun grew too hot; the programmer debugged the elusive error that
had caused the system to crash at midnight; the sailor navigated by the stars across the
vast and lonely ocean; the sculptor chiseled away at the marble block for months until the
figure emerged; the archaeologist carefully brushed away the dust from the ancient artifact;
the beekeeper inspected the hives and marveled at the industrious insects; the librarian
restored the damaged pages of a sixteenth-century manuscript; the oceanographer studied
the migration patterns of humpback whales; the electrician rewired the old building with
modern safety standards; the photographer waited hours for the perfect light to capture
the waterfall; the musician practiced the difficult passage until her fingers memorized it;
the pilot navigated through the storm using only instruments; the volunteer organized the
community garden project; the zoologist documented the behavior of the elusive snow leopard;
the cartographer mapped the uncharted territory with painstaking precision;
the architect envisioned a building that harmonized with its natural surroundings;
the chemist carefully measured the reagents for the experiment; the dancer rehearsed the
choreography until every movement was precise; the forester tracked the health of the
ancient redwoods; the geologist studied the layers of rock to understand the region's
history; the illustrator brought the children's book to life with vibrant colors;
the jeweler inspected each gemstone for clarity and cut; the knight polished his armor
before the tournament; the linguist deciphered the ancient script carved into the stone;
the mathematician pondered the elegant proof late into the night; the nurse comforted the
patient with gentle words; the ophthalmologist examined the patient's eyes with care;
the playwright revised the third act until the dialogue rang true; the quarterback threw a
perfect spiral across the field; the radiologist analyzed the scan for any anomalies;
the surgeon steadied her hands before the delicate procedure; the tailor measured the
fabric twice before cutting; the umpire called the game with authority; the veterinarian
treated the injured puppy with patience; the watchmaker repaired the antique clock with
steady hands; the xylophonist practiced her scales every evening; the yachtsman trimmed
the sails as the wind shifted; the zookeeper fed the pandas their bamboo breakfast;
every letter of the alphabet appeared in these sentences, ensuring a rich and stable
token sequence for prefix caching tests; do not be alarmed by the repetition, it is
intentional and harmless; remember to verify cache hits by comparing claimed tokens
against the client-side prefix length; a stable prefix is the key to reliable caching;
the quick brown fox jumps over the lazy dog and returns to the starting point;
consistency matters more than novelty when measuring cache behavior; the end of the
system prompt marks the beginning of the conversation; additional tokens appended
after this point extend the prefix without breaking it; thus successive requests
with the same system text will share a long common prefix; this is exactly the
condition under which provider-side prompt caching should activate; observe the
usage fields to confirm the cache read tokens; compare with the client LCP bound;
report any discrepancy as a suspect overclaim; the test sequence is complete."#;

/// 构造 provider 模型:原生单 key provider(openai/anthropic/mistral/xai/cohere/
/// google)直接构造;其余走注册表(compat,如 deepseek/groq/moonshotai 等)。
/// azure/bedrock/vertex 需额外参数(资源名/region/凭证),CLI 第一版不直接
/// 支持,可用其 OpenAI-compat 注册表镜像或后续扩展。
fn build_model(
    provider: &str,
    api_key: String,
    model_id: &str,
    base_url: Option<&str>,
) -> anyhow::Result<Arc<dyn aimux_core::language_model::LanguageModel>> {
    macro_rules! native {
        ($provider_mod:ident, $config:ident, $provider_type:ident) => {{
            let mut cfg = aimux_providers::$provider_mod::$config::new(api_key.clone());
            if let Some(url) = base_url {
                cfg = cfg.with_base_url(url);
            }
            let p = aimux_providers::$provider_mod::$provider_type::new(cfg);
            Arc::from(p.model(model_id))
        }};
    }

    match provider {
        "openai" => Ok(native!(openai, OpenAIConfig, OpenAIProvider)),
        "anthropic" => Ok(native!(anthropic, AnthropicConfig, AnthropicProvider)),
        "mistral" => Ok(native!(mistral, MistralConfig, MistralProvider)),
        "xai" => Ok(native!(xai, XAIConfig, XAIProvider)),
        "cohere" => Ok(native!(cohere, CohereConfig, CohereProvider)),
        "google" => Ok(native!(google, GoogleConfig, GoogleProvider)),
        _ => {
            let mut options = ProviderOptions::default();
            if let Some(url) = base_url {
                options.base_url = Some(url.to_string());
            }
            let model = aimux_providers::provider(provider, Some(api_key), model_id, Some(options))
                .map_err(|e| anyhow::anyhow!("provider '{provider}': {e}"))?;
            Ok(Arc::from(model))
        }
    }
}

/// 解析 `api_key` 参数:`env:VAR` 引用环境变量,否则按字面 key 使用。
fn resolve_api_key(spec: &str) -> anyhow::Result<String> {
    if let Some(var) = spec.strip_prefix("env:") {
        std::env::var(var).map_err(|_| anyhow::anyhow!("environment variable {var} is not set"))
    } else {
        Ok(spec.to_string())
    }
}

fn verdict_summary(stats: &[aimux_core::trace::TraceStats]) -> (u64, u64) {
    let mut trusted = 0;
    let mut overclaim = 0;
    for s in stats {
        trusted += s.verdict_counts.get("Trusted").copied().unwrap_or(0);
        overclaim += s
            .verdict_counts
            .get(report::kind_name(VerdictKind::SuspectOverclaim))
            .copied()
            .unwrap_or(0);
    }
    (trusted, overclaim)
}

pub async fn run(args: &ProviderArgs) -> anyhow::Result<Option<serde_json::Value>> {
    if args.max_requests == 0 {
        anyhow::bail!("--max-requests must be >= 1");
    }
    let api_key = resolve_api_key(&args.api_key)?;
    let model = build_model(
        &args.provider,
        api_key,
        &args.model,
        args.base_url.as_deref(),
    )?;

    // 探测层:单客户端直连 → strict 模式。
    let store = Arc::new(RingTraceStore::new());
    let traced = Arc::new(TraceLayer::new(model.clone(), store.clone()).with_rules_auditor(true));

    let system = args
        .prompt
        .clone()
        .unwrap_or_else(|| DEFAULT_SYSTEM.to_string());
    let rounds: Vec<Vec<aimux_core::message::ModelMessage>> = (0..args.max_requests)
        .map(|i| {
            let mut msgs = vec![
                aimux_core::message::ModelMessage::system(system.clone()),
                aimux_core::message::ModelMessage::user(format!(
                    "Question {i}: what is the capital of Atlantis?"
                )),
            ];
            // 追加历史轮次(前缀延续:每轮在前面基础上加 assistant+user)。
            for j in 0..i {
                msgs.push(aimux_core::message::ModelMessage::assistant(format!(
                    "The capital of Atlantis is Poseidonia (round {j})."
                )));
                msgs.push(aimux_core::message::ModelMessage::user(format!(
                    "Follow-up {i}.{j}: how deep is the canal?"
                )));
            }
            msgs
        })
        .collect();

    eprintln!(
        "probing {}/{} with {} request(s) — this consumes real API tokens",
        args.provider, args.model, args.max_requests
    );

    let mut per_round = Vec::new();
    for (i, msgs) in rounds.iter().enumerate() {
        let options = aimux_core::generate::GenerateTextOptions {
            session_id: Some("aimux-cli-probe".to_string()),
            max_output_tokens: Some(32),
            ..Default::default()
        };
        let started = std::time::Instant::now();
        let result = aimux_core::generate::generate_text(
            &*traced,
            aimux_core::message::ModelPrompt::Messages(msgs.clone()),
            options,
        )
        .await;
        let elapsed_ms = started.elapsed().as_millis();
        match result {
            Ok(r) => {
                let claimed = r.usage.input_tokens.cache_read.unwrap_or(0) as u64;
                per_round.push((i, Ok((claimed, elapsed_ms, r.usage))));
            }
            Err(e) => per_round.push((i, Err(e.to_string()))),
        }
    }

    // 全部 round 失败(网络/provider 错误)→ 非零退出(脚本可用性):
    // 没有收集到任何缓存证据,不应以成功状态结束。
    if per_round.iter().all(|(_, r)| r.is_err()) {
        anyhow::bail!(
            "all {} probe request(s) failed — no cache evidence collected",
            per_round.len()
        );
    }

    let stats = store.aggregate(&TraceFilter {
        provider: Some(args.provider.clone()),
        model: None,
        session_id: Some("aimux-cli-probe".to_string()),
        since_unix_ms: None,
    });

    match args.format {
        Format::Text => {
            println!("\n== cache probe: {} / {} ==", args.provider, args.model);
            for (i, round) in &per_round {
                match round {
                    Ok((claimed, elapsed_ms, usage)) => {
                        println!(
                            "  round {i}: cache_read={claimed:>6} tokens  input_total={:>6}  {elapsed_ms}ms",
                            usage.input_tokens.total.unwrap_or(0)
                        );
                    }
                    Err(e) => println!("  round {i}: ERROR {e}"),
                }
            }
            // 命中演变:第二轮起应有 cache_read > 0(前缀已建立)。
            let with_claim: Vec<(usize, u64)> = per_round
                .iter()
                .filter_map(|(i, r)| match r {
                    Ok((claimed, _, _)) => Some((*i, *claimed)),
                    Err(_) => None,
                })
                .collect();
            if with_claim.len() >= 2 {
                let first_claim = with_claim[0].1;
                let later = with_claim[1..].iter().map(|(_, c)| *c).max().unwrap_or(0);
                if first_claim == 0 && later > 0 {
                    println!(
                        "\n  ✅ prefix caching works: round 1 wrote, later rounds read (max {later} tokens)"
                    );
                } else if later == 0 && first_claim > 0 {
                    println!(
                        "\n  ⚠ cache reads on the FIRST request (claimed={first_claim}) but none later — verify against the client LCP bound"
                    );
                } else if later == 0 {
                    println!(
                        "\n  ℹ no cache reads observed — the provider may not report cache, or the deployment routes requests across nodes (cluster)"
                    );
                } else {
                    println!(
                        "\n  ⚠ cache reads on the FIRST request (claimed={first_claim}) — verify against the client LCP bound"
                    );
                }
            }
            print!("{}", report::render_stats_text(&stats));
            let (trusted, overclaim) = verdict_summary(&stats);
            println!(
                "\n  verdicts: {trusted} trusted, {overclaim} suspect-overclaim (client LCP is the ground truth)"
            );
        }
        Format::Json => {
            let report_json = serde_json::json!({
                "provider": args.provider,
                "model": args.model,
                "rounds": per_round.iter().map(|(i, r)| match r {
                    Ok((claimed, elapsed_ms, usage)) => serde_json::json!({
                        "round": i,
                        "cache_read_tokens": claimed,
                        "input_total_tokens": usage.input_tokens.total,
                        "elapsed_ms": elapsed_ms,
                    }),
                    Err(e) => serde_json::json!({ "round": i, "error": e }),
                }).collect::<Vec<_>>(),
                "stats": stats,
            });
            println!("{}", serde_json::to_string_pretty(&report_json)?);
            return Ok(Some(report_json));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本地 mock OpenAI chat completion 服务器:第一次请求报 0 命中,
    /// 之后请求报前缀命中(模拟真实缓存行为)。
    fn start_mock_server() -> (
        std::net::SocketAddr,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::AtomicU64;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hit_count = Arc::new(AtomicU64::new(0));
        let hits = hit_count.clone();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let mut buf = [0u8; 65536];
                let _n = stream.read(&mut buf).ok();
                // 模拟:首个请求写缓存(0 命中),后续请求前缀命中。
                let n = hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let cached = if n == 0 { 0 } else { 512 };
                let body = format!(
                    r#"{{
                        "id": "chatcmpl-mock",
                        "model": "gpt-4o",
                        "choices": [{{"message": {{"role": "assistant", "content": "ok"}}, "finish_reason": "stop"}}],
                        "usage": {{
                            "prompt_tokens": 2048,
                            "completion_tokens": 5,
                            "total_tokens": 2053,
                            "prompt_tokens_details": {{"cached_tokens": {cached}}}
                        }}
                    }}"#
                );
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (addr, hit_count)
    }

    #[tokio::test]
    async fn provider_probe_end_to_end_with_mock() {
        let (addr, _hits) = start_mock_server();
        let args = ProviderArgs {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            api_key: "sk-test".into(),
            base_url: Some(format!("http://{addr}")),
            max_requests: 3,
            prompt: Some("short probe prompt".into()), // 覆盖默认模板,测 mock 即可
            format: Format::Json,
        };
        let report = run(&args).await.expect("probe run failed");
        let report = report.expect("Json format must return a report");
        let rounds = report["rounds"].as_array().expect("rounds array");
        assert_eq!(rounds.len(), 3);
        // 命中演变行为:首轮 0,后续前缀命中 512。
        assert_eq!(
            rounds[0]["cache_read_tokens"], 0,
            "first request writes cache"
        );
        assert_eq!(
            rounds[1]["cache_read_tokens"], 512,
            "second request reads cache"
        );
        assert_eq!(
            rounds[2]["cache_read_tokens"], 512,
            "third request reads cache"
        );
        assert_eq!(report["stats"][0]["requests"], 3);
    }

    #[test]
    fn api_key_env_resolution() {
        // env: 引用缺省变量 → 明确错误。
        assert!(resolve_api_key("env:AIMUX_CLI_TEST_MISSING_KEY").is_err());
        // 字面 key 原样。
        assert_eq!(resolve_api_key("sk-literal").unwrap(), "sk-literal");
        // env: 引用存在的变量(edition 2024:set_var 为 unsafe)。
        // SAFETY: 单线程测试,无并发 env 读取。
        unsafe {
            std::env::set_var("AIMUX_CLI_TEST_KEY", "sk-env");
        }
        assert_eq!(resolve_api_key("env:AIMUX_CLI_TEST_KEY").unwrap(), "sk-env");
        unsafe {
            std::env::remove_var("AIMUX_CLI_TEST_KEY");
        }
    }
}
