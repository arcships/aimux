use std::sync::Arc;

use aimux_core::Provider;
use aimux_core::decision_model::{DecideOptions, DecisionModel as DecisionModelTrait};
use pyo3::prelude::*;

use crate::error::{serialize_result, to_py_err, wire_json};

#[pyclass]
pub struct DecisionModel {
    pub(crate) inner: Arc<dyn DecisionModelTrait>,
}

#[pymethods]
impl DecisionModel {
    /// The DecisionModelV4 properties as JSON: `specification_version`,
    /// `provider`, `model_id`, `supported_question_types`.
    pub fn describe(&self) -> PyResult<String> {
        serialize_result(&serde_json::json!({
            "specification_version": self.inner.specification_version(),
            "provider": self.inner.provider(),
            "model_id": self.inner.model_id(),
            "supported_question_types": self.inner.supported_question_types(),
        }))
    }

    /// `opts_json` is `DecideOptions` JSON; returns `DecideResult` JSON.
    pub fn decide(&self, py: Python<'_>, opts_json: &str) -> PyResult<String> {
        let options: DecideOptions = wire_json("opts_json", opts_json)?;
        let result = py
            .allow_threads(|| {
                crate::runtime().block_on(aimux_core::decision_model::decide(
                    self.inner.as_ref(),
                    options,
                ))
            })
            .map_err(|error| to_py_err(&error))?;
        serialize_result(&result)
    }
}

/// Create a Jev model; endpoint overrides the complete POST URL.
#[pyfunction]
#[pyo3(signature = (api_key, model_id, endpoint=None, probability_source=None))]
pub fn jev_decision(
    api_key: &str,
    model_id: &str,
    endpoint: Option<&str>,
    probability_source: Option<&str>,
) -> PyResult<DecisionModel> {
    let mut config = aimux_providers::JevConfig::new(api_key);
    if let Some(endpoint) = endpoint {
        config = config.with_endpoint(endpoint);
    }
    if let Some(source) = probability_source {
        config.probability_source = source.parse().map_err(|error| to_py_err(&error))?;
    }
    let model = aimux_providers::JevProvider::new(config)
        .decision_model(model_id)
        .unwrap_or_else(|| {
            Err(aimux_core::AiMuxError::no_such_model(
                model_id,
                "decisionModel",
            ))
        })
        .map_err(|error| to_py_err(&error))?;
    Ok(DecisionModel { inner: model })
}
