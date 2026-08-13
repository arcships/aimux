//! RFC-0023 P1 端到端集成测试:层 A 接入(generate_text/stream_text) +
//! 流式终结观测 + JsonlRecorder 落盘。
//!
//! 全局录制器是进程级单例,所有场景放在同一测试内串行执行(init→验证→替换)。

use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::recording::{self, JsonlRecorder, OutcomeStatus, Recording};
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};
use async_trait::async_trait;
use futures::StreamExt;
use std::sync::Arc;

/// Mock model:非流式固定回话;流式按参数决定行为。
#[derive(Default)]
struct EchoModel {
    /// do_generate 立即失败。
    gen_fails: bool,
    /// 流中途发 Error part。
    part_error: bool,
    /// do_stream 立即失败。
    stream_fails: bool,
}

#[async_trait]
impl LanguageModel for EchoModel {
    fn provider(&self) -> &str {
        "mock-echo"
    }

    fn model_id(&self) -> &str {
        "echo-1"
    }

    async fn do_generate(
        &self,
        _options: &CallOptions,
    ) -> Result<GenerateResult, aimux_core::error::AiMuxError> {
        if self.gen_fails {
            return Err(aimux_core::error::AiMuxError::ApiCall(
                aimux_core::error::ApiCallError {
                    message: "gen boom".into(),
                    ..Default::default()
                },
            ));
        }
        Ok(GenerateResult {
            content: vec![GenerateContent::Text {
                text: "pong".into(),
                provider_metadata: None,
            }],
            finish_reason: FinishReason {
                unified: FinishReasonUnified::Stop,
                raw: None,
            },
            usage: Usage::default(),
            warnings: vec![],
            provider_metadata: None,
            response: aimux_core::types::ResponseMetadata::default(),
            request_body: None,
            response_headers: None,
        })
    }

    async fn do_stream(
        &self,
        _options: &CallOptions,
    ) -> Result<StreamResult, aimux_core::error::AiMuxError> {
        if self.stream_fails {
            return Err(aimux_core::error::AiMuxError::InvalidResponseData(
                "boom".into(),
            ));
        }
        if self.part_error {
            let parts = vec![
                Ok(StreamPart::StreamStart { warnings: vec![] }),
                Ok(StreamPart::Error {
                    error: aimux_core::error::AiMuxError::InvalidResponseData("mid-stream".into()),
                }),
            ];
            return Ok(StreamResult {
                stream: Box::pin(futures::stream::iter(parts)),
                request_body: None,
                response_headers: None,
            });
        }
        let parts = vec![
            Ok(StreamPart::StreamStart { warnings: vec![] }),
            Ok(StreamPart::TextDelta {
                id: "1".into(),
                delta: "hi".into(),
                provider_metadata: None,
            }),
            Ok(StreamPart::Finish {
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: None,
                },
                usage: Usage::default(),
                provider_metadata: None,
            }),
        ];
        Ok(StreamResult {
            stream: Box::pin(futures::stream::iter(parts)),
            request_body: None,
            response_headers: None,
        })
    }
}

fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Runtime::new().unwrap().block_on(f)
}

fn wait_sorted_line(dir: &std::path::Path) -> Recording {
    let path = dir.join("recordings.jsonl");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(s) = std::fs::read_to_string(&path)
            && let Some(line) = s.lines().next()
            && !line.trim().is_empty()
            && let Ok(rec) = serde_json::from_str::<Recording>(line.trim())
        {
            return rec;
        }
        if std::time::Instant::now() > deadline {
            panic!("recording not written: {}", path.display());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn layer_a_records_all_outcome_paths() {
    let pid = std::process::id();
    let dir_ok = std::env::temp_dir().join(format!("aimux-e2e-ok-{pid}"));
    let dir_err = std::env::temp_dir().join(format!("aimux-e2e-err-{pid}"));
    let dir_finish = std::env::temp_dir().join(format!("aimux-e2e-fin-{pid}"));
    let dir_part_err = std::env::temp_dir().join(format!("aimux-e2e-pe-{pid}"));
    let dir_cancel = std::env::temp_dir().join(format!("aimux-e2e-cancel-{pid}"));
    for d in [&dir_ok, &dir_err, &dir_finish, &dir_part_err, &dir_cancel] {
        let _ = std::fs::remove_dir_all(d);
    }

    // ① generate 成功 → Success + finish_reason。
    recording::init_recording(Some(Arc::new(JsonlRecorder::new(&dir_ok))));
    run(async {
        let r = aimux_core::generate::generate_text(
            &EchoModel::default(),
            "ping",
            aimux_core::generate::GenerateTextOptions::default(),
        )
        .await;
        assert_eq!(r.unwrap().text, "pong");
    });
    let rec = wait_sorted_line(&dir_ok);
    assert!(rec.call_id.starts_with("call-"));
    assert_eq!(rec.provider.provider, "mock-echo");
    assert_eq!(rec.outcome.status, OutcomeStatus::Success);
    assert_eq!(rec.outcome.finish_reason.as_deref(), Some("stop"));

    // ② generate 失败 → Error。
    recording::init_recording(Some(Arc::new(JsonlRecorder::new(&dir_err))));
    run(async {
        let r = aimux_core::generate::generate_text(
            &EchoModel {
                gen_fails: true,
                ..Default::default()
            },
            "ping",
            aimux_core::generate::GenerateTextOptions::default(),
        )
        .await;
        assert!(r.is_err());
    });
    let rec = wait_sorted_line(&dir_err);
    assert_eq!(rec.outcome.status, OutcomeStatus::Error);
    assert!(rec.outcome.error.is_some());

    // ③ 流式正常终结(消费完) → Success。
    recording::init_recording(Some(Arc::new(JsonlRecorder::new(&dir_finish))));
    run(async {
        let result = aimux_core::generate::stream_text(
            &EchoModel::default(),
            "ping",
            aimux_core::generate::GenerateTextOptions::default(),
        )
        .await
        .unwrap();
        let parts: Vec<_> = result.stream.collect::<Vec<_>>().await;
        assert_eq!(parts.len(), 3);
    });
    let rec = wait_sorted_line(&dir_finish);
    assert_eq!(rec.outcome.status, OutcomeStatus::Success);

    // ④ 流式 Error part → Error。
    recording::init_recording(Some(Arc::new(JsonlRecorder::new(&dir_part_err))));
    run(async {
        let result = aimux_core::generate::stream_text(
            &EchoModel {
                part_error: true,
                ..Default::default()
            },
            "ping",
            aimux_core::generate::GenerateTextOptions::default(),
        )
        .await
        .unwrap();
        let parts: Vec<_> = result.stream.collect::<Vec<_>>().await;
        assert_eq!(parts.len(), 2);
    });
    let rec = wait_sorted_line(&dir_part_err);
    assert_eq!(rec.outcome.status, OutcomeStatus::Error);
    assert!(rec.outcome.error.unwrap_or_default().contains("mid-stream"));

    // ⑤ 流式提前 drop(不消费完)→ Cancelled。
    recording::init_recording(Some(Arc::new(JsonlRecorder::new(&dir_cancel))));
    run(async {
        let result = aimux_core::generate::stream_text(
            &EchoModel::default(),
            "ping",
            aimux_core::generate::GenerateTextOptions::default(),
        )
        .await
        .unwrap();
        drop(result.stream); // 未 poll 完即抛弃 → Drop 终结 Cancelled
    });
    let rec = wait_sorted_line(&dir_cancel);
    assert_eq!(rec.outcome.status, OutcomeStatus::Cancelled);

    recording::init_recording(None);
    let _ = std::fs::remove_dir_all(&dir_ok);
}
