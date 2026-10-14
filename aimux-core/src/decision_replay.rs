//! Decision replay using the existing unified JSONL recording format.
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::AiMuxError;
use crate::decision_model::{
    DecideOptions, DecideResult, DecisionCallOptions, DecisionModel, DecisionQuestionType,
    DecisionResult, DecisionUsage, decide,
};
use crate::recording::{OutcomeStatus, Recording, RecordingOperation};

/// Appended to a replay error when recordings written by aimux 0.5 or
/// earlier (no `provider_id`) are involved: their decision options and
/// results use the pre-V4 shapes, which this version does not read.
const OLDER_RECORDING: &str = "written by aimux 0.5 or earlier, whose decision options and \
                               results this version does not read";

/// What identifies a request: the call options a model receives. Operation
/// settings (retries, timeout) are not part of it.
fn key(options: &Value) -> Value {
    json!({
        "state": options["state"],
        "questions": options["questions"],
        "headers": options.get("headers").cloned().unwrap_or(Value::Null),
        "providerOptions": options.get("providerOptions").cloned().unwrap_or(Value::Null),
    })
}

/// Offline replay with exact input matching; transport controls are ignored.
/// The result `decide` returned is recorded by Core, so replay needs no wire
/// decoder. Recordings hold no capability snapshot: the offline model accepts
/// every question type, and a request must match a recorded input exactly.
pub struct MockDecisionReplayModel {
    provider: String,
    model_id: String,
    recordings: Vec<Recording>,
}

impl MockDecisionReplayModel {
    /// Build from unified recordings, selecting the named provider/model.
    /// # Errors
    /// Returns an error if no decision recording is available.
    pub fn new(
        provider: impl Into<String>,
        model_id: impl Into<String>,
        recordings: Vec<Recording>,
    ) -> Result<Self, AiMuxError> {
        let provider = provider.into();
        let model_id = model_id.into();
        if !recordings.iter().any(|rec| {
            rec.input.operation == RecordingOperation::Decision
                && rec.provider.provider == provider
                && rec.provider.model_id == model_id
        }) {
            return Err(AiMuxError::InvalidArgument(
                "decision replay: no matching provider/model recording".into(),
            ));
        }
        Ok(Self {
            provider,
            model_id,
            recordings,
        })
    }

    /// Load decisions from a unified JSONL file, including mixed operation files.
    /// # Errors
    /// Returns read/parse errors or an error if the file contains no decisions.
    pub fn from_jsonl(path: impl AsRef<std::path::Path>) -> Result<Self, AiMuxError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| AiMuxError::InvalidArgument(format!("decision replay: {e}")))?;
        let recordings: Vec<Recording> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).map_err(|e| AiMuxError::JsonParse(e.to_string()))
            })
            .collect::<Result<_, _>>()?;
        let first = recordings
            .iter()
            .find(|rec| rec.input.operation == RecordingOperation::Decision)
            .ok_or_else(|| {
                AiMuxError::InvalidArgument("decision replay: no decisions in file".into())
            })?;
        Self::new(
            first.provider.provider.clone(),
            first.provider.model_id.clone(),
            recordings,
        )
    }
}

#[async_trait]
impl DecisionModel for MockDecisionReplayModel {
    fn provider(&self) -> &str {
        &self.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn supported_question_types(&self) -> &[DecisionQuestionType] {
        DecisionQuestionType::ALL
    }
    async fn do_decide(&self, options: &DecisionCallOptions) -> Result<DecisionResult, AiMuxError> {
        let requested =
            key(&serde_json::to_value(options).map_err(|e| AiMuxError::JsonParse(e.to_string()))?);
        let rec = self
            .recordings
            .iter()
            .find(|rec| {
                rec.complete
                    && rec.input.operation == RecordingOperation::Decision
                    && rec.provider.provider == self.provider
                    && rec.provider.model_id == self.model_id
                    && crate::replay::redaction_aware_eq(&key(&rec.input.options), &requested)
            })
            .ok_or_else(|| {
                let older = self
                    .recordings
                    .iter()
                    .filter(|rec| {
                        rec.input.operation == RecordingOperation::Decision
                            && rec.provider.provider == self.provider
                            && rec.provider.model_id == self.model_id
                            && rec.provider.provider_id.is_empty()
                    })
                    .count();
                let mut message = "decision replay: no exact matching complete recording".to_string();
                if older > 0 {
                    message.push_str(&format!(
                        "; {older} recording(s) of this model were {OLDER_RECORDING} and never match"
                    ));
                }
                AiMuxError::InvalidArgument(message)
            })?;
        if rec.outcome.status != OutcomeStatus::Success {
            return Err(rec
                .outcome
                .error_value
                .clone()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_else(|| {
                    AiMuxError::InvalidResponseData(
                        rec.outcome
                            .error
                            .clone()
                            .unwrap_or_else(|| "recorded decision failed".into()),
                    )
                }));
        }
        let recorded: DecideResult =
            serde_json::from_value(rec.outcome.decision_result.clone().ok_or_else(|| {
                AiMuxError::InvalidResponseData("decision replay: missing recorded result".into())
            })?)
            .map_err(|e| AiMuxError::InvalidResponseData(format!("decision replay result: {e}")))?;
        Ok(DecisionResult {
            answers: recorded.answers,
            rounding: recorded.rounding,
            usage: Some(DecisionUsage {
                input_tokens: recorded.usage.input_tokens,
                output_tokens: recorded.usage.output_tokens,
            }),
            warnings: recorded.warnings,
            provider_metadata: recorded.provider_metadata,
            response: Some(recorded.response),
        })
    }
}

/// Rebuild a decision request from a unified recording and call a supplied model.
/// A live model sends HTTP; a MockDecisionReplayModel stays offline.
/// # Errors
/// Rejects other operations and returns request/provider validation errors.
pub async fn replay_decision_with_model(
    recording: &Recording,
    model: &dyn DecisionModel,
) -> Result<DecideResult, AiMuxError> {
    if recording.input.operation != RecordingOperation::Decision {
        return Err(AiMuxError::InvalidArgument(
            "decision replay requires a decision recording".into(),
        ));
    }
    let unread = crate::replay::snake_case_option_keys(&recording.input.options);
    if !unread.is_empty() {
        let older = if recording.provider.provider_id.is_empty() {
            format!("the recording was {OLDER_RECORDING}: ")
        } else {
            String::new()
        };
        return Err(AiMuxError::JsonParse(format!(
            "decision replay: {older}this version reads the options camelCase and would \
             ignore {}",
            unread.join(", ")
        )));
    }
    let options: DecideOptions =
        serde_json::from_value(recording.input.options.clone()).map_err(|e| {
            if recording.provider.provider_id.is_empty() {
                AiMuxError::JsonParse(format!(
                    "decision replay: the recording was {OLDER_RECORDING}: {e}"
                ))
            } else {
                AiMuxError::JsonParse(e.to_string())
            }
        })?;
    decide(model, options).await
}
