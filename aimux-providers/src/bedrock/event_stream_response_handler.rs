//! The event stream response handler of the Converse stream
//! (amazon-bedrock-event-stream-response-handler.ts).

use aimux_core::error::AiMuxError;
use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;

pub(super) fn bedrock_event_stream_response_handler()
-> aimux_provider_utils::ResponseHandler<BoxStream<'static, Result<Bytes, AiMuxError>>> {
    aimux_provider_utils::ResponseHandler::new_with_source(|input, body_source| async move {
        let headers = aimux_provider_utils::extract_response_headers::extract_response_headers(
            input.response.headers(),
        );
        let output_headers = headers.clone();
        let url = input.url;
        let signal = input.abort_signal;
        let stream = input.response.bytes_stream().map(move |result| {
            result.map_err(|error| {
                AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
                    response_headers: Some(headers.clone()),
                    is_retryable: true,
                    cause: Some(aimux_core::ErrorCause::source(&error)),
                    ..aimux_core::ApiCallError::new(
                        error.to_string(),
                        url.clone(),
                        body_source.materialize(),
                    )
                }))
            })
        });
        let value: BoxStream<'static, Result<Bytes, AiMuxError>> = match signal {
            Some(signal) => Box::pin(async_stream::stream! {
                futures::pin_mut!(stream);
                loop {
                    tokio::select! {
                        biased;
                        () = signal.cancelled() => {
                            yield Err(AiMuxError::from_abort_signal(&signal));
                            break;
                        }
                        item = stream.next() => match item {
                            Some(item) => yield item,
                            None => break,
                        }
                    }
                }
            }),
            None => Box::pin(stream),
        };
        Ok(aimux_provider_utils::ResponseHandlerOutput {
            value,
            raw_value: None,
            response_headers: output_headers,
        })
    })
    .streaming()
}
