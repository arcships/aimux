//! `transcription/google-transcription-model-options.ts`.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::google::options::no_null;
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

#[derive(Deserialize)]
enum Mode {
    #[serde(rename = "SMART")]
    Smart,
    #[serde(rename = "VERBATIM")]
    Verbatim,
}

impl Mode {
    fn name(&self) -> &'static str {
        match self {
            Self::Smart => "SMART",
            Self::Verbatim => "VERBATIM",
        }
    }
}

/// Speech recognition options shared by unary and live transcription.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TranscriptionOptions {
    #[serde(default, deserialize_with = "no_null")]
    language_codes: Option<Vec<String>>,
    #[serde(default, deserialize_with = "no_null")]
    custom_vocabulary: Option<Vec<String>>,
    #[serde(default, deserialize_with = "no_null")]
    word_timestamp: Option<bool>,
    #[serde(default, deserialize_with = "no_null")]
    diarization: Option<bool>,
    #[serde(default, deserialize_with = "no_null")]
    mode: Option<Mode>,
}

impl TranscriptionOptions {
    pub(crate) fn parse(
        namespace: crate::google::options::Namespace,
        provider_options: Option<&SharedProviderOptions>,
    ) -> Result<Self, AiMuxError> {
        namespace
            .parse(provider_options)
            .map(Option::unwrap_or_default)
    }

    /// Google's `AudioTranscriptionConfig` (the live setup).
    pub(crate) fn audio_transcription_config(&self) -> Value {
        let mut config = json!({});
        if let Some(value) = &self.language_codes {
            config["languageCodes"] = json!(value);
        }
        if let Some(value) = &self.custom_vocabulary {
            config["customVocabulary"] = json!(value);
        }
        if let Some(value) = self.word_timestamp {
            config["wordTimestamp"] = json!(value);
        }
        if let Some(value) = self.diarization {
            config["diarization"] = json!(value);
        }
        if let Some(value) = &self.mode {
            config["mode"] = json!(value.name());
        }
        config
    }

    /// The Interactions API `transcription_config` (snake_case wire).
    /// Diarization and word timestamps are expressed inside the `mode` object.
    pub(super) fn interactions_config(&self) -> Option<Value> {
        let mut config = Map::new();
        if let Some(value) = &self.language_codes {
            config.insert("language_codes".into(), json!(value));
        }
        if let Some(value) = &self.custom_vocabulary {
            config.insert("custom_vocabulary".into(), json!(value));
        }
        let diarization = self.diarization == Some(true);
        let word_timestamp = self.word_timestamp == Some(true);
        if self.mode.is_some() || diarization || word_timestamp {
            let mode = self.mode.as_ref().map_or("VERBATIM", Mode::name);
            let mut mode = json!({"type": mode.to_lowercase()});
            if diarization {
                mode["diarization_mode"] = json!("speaker");
            }
            if word_timestamp {
                mode["timestamp_granularities"] = json!(["word"]);
            }
            config.insert("mode".into(), mode);
        }
        (!config.is_empty()).then_some(Value::Object(config))
    }
}

/// Parses a Google duration offset such as `"1s"` or `"9.400s"` to seconds,
/// like `Number.parseFloat`: the longest numeric prefix counts.
pub(crate) fn parse_offset_seconds(offset: Option<&str>) -> Option<f64> {
    let value = offset?.trim_start();
    (1..=value.len())
        .rev()
        .filter(|end| value.is_char_boundary(*end))
        .find_map(|end| value[..end].parse::<f64>().ok())
        .filter(|value| value.is_finite())
}
