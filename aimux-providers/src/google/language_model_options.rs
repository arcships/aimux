//! `google-language-model-options.ts`: the `providerOptions.google` schema
//! (`googleLanguageModelOptions`), checked before a request is built.

use aimux_core::error::AiMuxError;
use aimux_core::shared::JsonObject;
use serde_json::Value;

/// Validate the provider options against `googleLanguageModelOptions`.
pub(super) fn validate_provider_options(values: &JsonObject) -> Result<(), AiMuxError> {
    let invalid = |name: &str| {
        AiMuxError::invalid_argument(format!("Invalid Google provider option: {name}"))
    };
    for name in [
        "structuredOutputs",
        "audioTimestamp",
        "streamFunctionCallArguments",
    ] {
        if values.get(name).is_some_and(|value| !value.is_boolean()) {
            return Err(invalid(name));
        }
    }
    if values
        .get("cachedContent")
        .is_some_and(|value| !value.is_string())
    {
        return Err(invalid("cachedContent"));
    }
    for (name, allowed) in [
        (
            "threshold",
            &[
                "HARM_BLOCK_THRESHOLD_UNSPECIFIED",
                "BLOCK_LOW_AND_ABOVE",
                "BLOCK_MEDIUM_AND_ABOVE",
                "BLOCK_ONLY_HIGH",
                "BLOCK_NONE",
                "OFF",
            ][..],
        ),
        (
            "mediaResolution",
            &[
                "MEDIA_RESOLUTION_UNSPECIFIED",
                "MEDIA_RESOLUTION_LOW",
                "MEDIA_RESOLUTION_MEDIUM",
                "MEDIA_RESOLUTION_HIGH",
            ][..],
        ),
        ("serviceTier", &["standard", "flex", "priority"][..]),
        ("sharedRequestType", &["priority", "flex", "standard"][..]),
        ("requestType", &["shared"][..]),
    ] {
        if values
            .get(name)
            .is_some_and(|value| !value.as_str().is_some_and(|value| allowed.contains(&value)))
        {
            return Err(invalid(name));
        }
    }
    if values.get("responseModalities").is_some_and(|value| {
        !value.as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| matches!(item.as_str(), Some("TEXT" | "IMAGE")))
        })
    }) {
        return Err(invalid("responseModalities"));
    }
    if let Some(value) = values.get("thinkingConfig") {
        let config = value.as_object().ok_or_else(|| invalid("thinkingConfig"))?;
        if config
            .get("thinkingBudget")
            .is_some_and(|value| !value.is_number())
            || config
                .get("includeThoughts")
                .is_some_and(|value| !value.is_boolean())
            || config.get("thinkingLevel").is_some_and(|value| {
                !matches!(value.as_str(), Some("minimal" | "low" | "medium" | "high"))
            })
        {
            return Err(invalid("thinkingConfig"));
        }
    }
    if let Some(value) = values.get("labels")
        && !value
            .as_object()
            .is_some_and(|items| items.values().all(Value::is_string))
    {
        return Err(invalid("labels"));
    }
    if let Some(value) = values.get("safetySettings")
        && !value.as_array().is_some_and(|items| {
            items.iter().all(|item| {
                matches!(
                    item["category"].as_str(),
                    Some(
                        "HARM_CATEGORY_UNSPECIFIED"
                            | "HARM_CATEGORY_HATE_SPEECH"
                            | "HARM_CATEGORY_DANGEROUS_CONTENT"
                            | "HARM_CATEGORY_HARASSMENT"
                            | "HARM_CATEGORY_SEXUALLY_EXPLICIT"
                            | "HARM_CATEGORY_CIVIC_INTEGRITY"
                    )
                ) && matches!(
                    item["threshold"].as_str(),
                    Some(
                        "HARM_BLOCK_THRESHOLD_UNSPECIFIED"
                            | "BLOCK_LOW_AND_ABOVE"
                            | "BLOCK_MEDIUM_AND_ABOVE"
                            | "BLOCK_ONLY_HIGH"
                            | "BLOCK_NONE"
                            | "OFF"
                    )
                )
            })
        })
    {
        return Err(invalid("safetySettings"));
    }
    if let Some(value) = values.get("imageConfig") {
        let config = value.as_object().ok_or_else(|| invalid("imageConfig"))?;
        for (name, allowed) in [
            (
                "aspectRatio",
                &[
                    "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9", "1:8",
                    "8:1", "1:4", "4:1",
                ][..],
            ),
            ("imageSize", &["1K", "2K", "4K", "512"][..]),
            (
                "personGeneration",
                &[
                    "PERSON_GENERATION_UNSPECIFIED",
                    "ALLOW_ALL",
                    "ALLOW_ADULT",
                    "ALLOW_NONE",
                ][..],
            ),
            (
                "prominentPeople",
                &[
                    "PROMINENT_PEOPLE_UNSPECIFIED",
                    "ALLOW_PROMINENT_PEOPLE",
                    "BLOCK_PROMINENT_PEOPLE",
                ][..],
            ),
        ] {
            if config
                .get(name)
                .is_some_and(|value| !value.as_str().is_some_and(|value| allowed.contains(&value)))
            {
                return Err(invalid(name));
            }
        }
        if let Some(value) = config.get("imageOutputOptions") {
            let output = value
                .as_object()
                .ok_or_else(|| invalid("imageOutputOptions"))?;
            if output
                .get("mimeType")
                .is_some_and(|value| !matches!(value.as_str(), Some("image/jpeg" | "image/png")))
                || output
                    .get("compressionQuality")
                    .is_some_and(|value| !value.is_number())
            {
                return Err(invalid("imageOutputOptions"));
            }
        }
    }
    if let Some(value) = values.get("retrievalConfig") {
        let config = value
            .as_object()
            .ok_or_else(|| invalid("retrievalConfig"))?;
        if let Some(value) = config.get("latLng")
            && !value.as_object().is_some_and(|coordinates| {
                coordinates.get("latitude").is_some_and(Value::is_number)
                    && coordinates.get("longitude").is_some_and(Value::is_number)
            })
        {
            return Err(invalid("retrievalConfig.latLng"));
        }
    }
    Ok(())
}
