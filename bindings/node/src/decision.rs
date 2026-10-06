use std::sync::Arc;

use aimux_core::Provider;
use aimux_core::decision_model::{DecisionCallOptions, DecisionModel as DecisionModelTrait};
use napi_derive::napi;

use crate::error::{AiMuxBindingError, AimuxResult, parse_wire_json, serialize_result};

#[napi]
pub struct DecisionModel {
    inner: Arc<dyn DecisionModelTrait>,
}

#[napi]
impl DecisionModel {
    #[napi(ts_return_type = "Promise<string>")]
    pub async fn decide(
        &self,
        opts_json: String,
        bridge: Option<&crate::AbortBridge>,
    ) -> AimuxResult<String> {
        AimuxResult(
            async {
                let mut options: DecisionCallOptions = parse_wire_json("opts_json", &opts_json)?;
                options.abort_signal = bridge.map(|bridge| bridge.core_signal());
                let result = aimux_core::decision_model::decide(self.inner.as_ref(), options)
                    .await
                    .map_err(|error| AiMuxBindingError::from(&error))?;
                serialize_result(&result)
            }
            .await,
        )
    }
}

/// Create a Jev model; endpoint overrides the complete POST URL.
#[napi]
pub async fn jev_decision(
    api_key: String,
    model_id: String,
    endpoint: Option<String>,
) -> AimuxResult<DecisionModel> {
    AimuxResult(
        async {
            let mut config = aimux_providers::JevConfig::new(api_key);
            if let Some(endpoint) = endpoint {
                config = config.with_endpoint(endpoint);
            }
            let model = aimux_providers::JevProvider::new(config)
                .decision_model(&model_id)
                .map_err(|error| AiMuxBindingError::from(&error))?;
            Ok(DecisionModel {
                inner: Arc::from(model),
            })
        }
        .await,
    )
}
