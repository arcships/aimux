//! `GET {base_url}/models` for the providers whose catalogue is the
//! OpenAI-shaped `{ "data": [{ "id", "owned_by", "created" }] }`.

use aimux_core::AiMuxError;
use aimux_core::model_catalogue::{ModelLimits, RuntimeModel};
use aimux_provider_utils::{HttpRequest, ResponseHandler};
use serde_json::Value;

use super::EndpointConfig;

#[derive(serde::Deserialize)]
struct ModelsList {
    #[serde(default)]
    data: Vec<ModelListEntry>,
}

/// One entry of an OpenAI-shaped `/models` list, as aimux reads it.
///
/// The limit keys are read leniently (`Value`, then `as_u64`): they are not
/// part of OpenAI's list schema, and a vendor sending one as a string or a
/// float must not fail the whole list.
#[derive(serde::Deserialize)]
pub(crate) struct ModelListEntry {
    id: String,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    created: Option<u64>,
    /// The context window: OpenRouter (`GET /api/v1/models`) and other
    /// aggregators that copy its list shape.
    #[serde(default)]
    context_length: Option<Value>,
    /// vLLM's `ModelCard.max_model_len`, its input plus output limit.
    #[serde(default)]
    max_model_len: Option<Value>,
    /// OpenRouter's `top_provider.max_completion_tokens`.
    #[serde(default)]
    top_provider: Option<Value>,
}

impl From<ModelListEntry> for RuntimeModel {
    fn from(entry: ModelListEntry) -> Self {
        let context = entry
            .context_length
            .as_ref()
            .and_then(Value::as_u64)
            .or_else(|| entry.max_model_len.as_ref().and_then(Value::as_u64));
        let output = entry
            .top_provider
            .as_ref()
            .and_then(|top| top.get("max_completion_tokens"))
            .and_then(Value::as_u64);
        let limits = (context.is_some() || output.is_some()).then_some(ModelLimits {
            context,
            output,
            input: None,
        });
        RuntimeModel {
            id: entry.id,
            owned_by: entry.owned_by,
            created: entry.created,
            limits,
        }
    }
}

/// One `GET {base_url}/models` exchange: no retry, no recording. Discovery is
/// not a Core operation and the AI SDK has no equivalent, so a failure is
/// reported to the caller as it happened.
///
/// # Errors
///
/// Returns the header-resolution error (a missing key is `LoadApiKey`),
/// `ApiCall` for HTTP/transport failures and `JsonParse` when the body does
/// not deserialize into the models list.
pub(crate) async fn list_data_models(
    config: &EndpointConfig,
    failed_response_handler: ResponseHandler<AiMuxError>,
) -> Result<Vec<RuntimeModel>, AiMuxError> {
    let exchange = config.exchange(None).await?;
    let resp = aimux_provider_utils::get_from_api(
        exchange.with_transport(HttpRequest {
            url: exchange.url("/models"),
            headers: exchange.headers(),
            ..Default::default()
        }),
        aimux_provider_utils::create_json_response_handler(),
        failed_response_handler,
    )
    .await?;
    let parsed: ModelsList = resp.value;
    Ok(parsed.data.into_iter().map(RuntimeModel::from).collect())
}
