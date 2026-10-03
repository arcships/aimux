//! `GET {base_url}/models` for the providers whose catalogue is the
//! OpenAI-shaped `{ "data": [{ "id", "owned_by", "created" }] }`.

use aimux_core::AiMuxError;
use aimux_core::model_catalogue::RuntimeModel;
use aimux_provider_utils::{HttpRequest, ResponseHandler};

use super::EndpointConfig;

#[derive(serde::Deserialize)]
struct ModelsList {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(serde::Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    created: Option<u64>,
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
    Ok(parsed
        .data
        .into_iter()
        .map(|entry| RuntimeModel {
            id: entry.id,
            owned_by: entry.owned_by,
            created: entry.created,
        })
        .collect())
}
