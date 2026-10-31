//! The providerOptions and providerMetadata namespace of xAI.
//!
//! `@ai-sdk/xai` reads `providerOptions.xai` and writes response metadata
//! under `xai`, whatever the provider is named. This module is the one place
//! that spells the key.

use aimux_core::error::AiMuxError;
use aimux_core::shared::{JsonObject, SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The providerOptions / providerMetadata key (and the provider-reference key
/// of uploaded files).
pub(crate) const NAMESPACE: &str = "xai";

/// The xAI options in a providerOptions container (a call's map, a part's
/// object).
pub(crate) fn xai_options(provider_options: Option<&SharedProviderOptions>) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}

/// `{ "xai": payload }`.
pub(crate) fn xai_metadata(payload: Value) -> ProviderMetadata {
    provider_namespace(NAMESPACE, payload).expect("metadata payload is an object")
}

/// `parseProviderOptions({ provider: 'xai', providerOptions, schema })` for
/// the media-family schemas (speech, transcription, video).
///
/// # Errors
///
/// `InvalidArgument` "invalid xai provider options" when the namespace does
/// not match the schema, with the serde error as its cause.
pub(crate) fn parse<T: serde::de::DeserializeOwned>(
    provider_options: Option<&SharedProviderOptions>,
) -> Result<Option<T>, AiMuxError> {
    aimux_provider_utils::parse_provider_options(NAMESPACE, provider_options)
}

/// `deserialize_with` helpers for the zod refinements of the media-family
/// schemas. A `nullish_*` helper accepts `null` as unset (`.nullish()`); an
/// `optional_*` one rejects it (`.optional()`), as zod does.
pub(crate) mod de {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer};
    use serde_json::Number;

    /// A JavaScript integer (`z.number().int()`): `2.0` is one.
    fn integer<E: serde::de::Error>(number: &Number) -> Result<i64, E> {
        number
            .as_i64()
            .or_else(|| {
                number
                    .as_f64()
                    .filter(|value| value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0)
                    // The filter keeps the value within the safe-integer range.
                    .map(|value| value as i64)
            })
            .ok_or_else(|| E::custom("expected int, received number"))
    }

    fn int_in<E: serde::de::Error>(
        number: &Number,
        allowed: impl Fn(i64) -> bool,
    ) -> Result<i64, E> {
        let value = integer(number)?;
        if allowed(value) {
            Ok(value)
        } else {
            Err(E::custom(format!("invalid value {value}")))
        }
    }

    fn nullish_int<'de, D: Deserializer<'de>>(
        deserializer: D,
        allowed: impl Fn(i64) -> bool,
    ) -> Result<Option<i64>, D::Error> {
        Option::<Number>::deserialize(deserializer)?
            .map(|number| int_in(&number, allowed))
            .transpose()
    }

    fn optional_int<'de, D: Deserializer<'de>>(
        deserializer: D,
        allowed: impl Fn(i64) -> bool,
    ) -> Result<Option<i64>, D::Error> {
        int_in(&Number::deserialize(deserializer)?, allowed).map(Some)
    }

    /// `z.union([z.literal(8000), …, z.literal(48000)]).nullish()`.
    pub(crate) fn sample_rate<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        nullish_int(d, |v| {
            [8000, 16000, 22050, 24000, 44100, 48000].contains(&v)
        })
    }

    /// `z.union([z.literal(32000), …, z.literal(192000)]).nullish()`.
    pub(crate) fn bit_rate<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        nullish_int(d, |v| [32000, 64000, 96000, 128000, 192000].contains(&v))
    }

    /// `z.union([z.literal(0), z.literal(1), z.literal(2)]).nullish()`.
    pub(crate) fn streaming_latency<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<i64>, D::Error> {
        nullish_int(d, |v| (0..=2).contains(&v))
    }

    /// `z.number().int().min(2).max(8).nullish()`.
    pub(crate) fn channels<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        nullish_int(d, |v| (2..=8).contains(&v))
    }

    /// `z.number().int().min(0).max(5000).optional()`.
    pub(crate) fn endpointing<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        optional_int(d, |v| (0..=5000).contains(&v))
    }

    /// `z.number().int().min(1).max(5000).optional()`.
    pub(crate) fn smart_turn_timeout<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<i64>, D::Error> {
        optional_int(d, |v| (1..=5000).contains(&v))
    }

    /// `z.number().int().positive().max(2_592_000).optional()`.
    pub(crate) fn storage_expires_after<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<i64>, D::Error> {
        optional_int(d, |v| (1..=2_592_000).contains(&v))
    }

    /// `z.number().int().min(3_600).max(2_592_000).optional()`.
    pub(crate) fn public_url_expires_after<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<i64>, D::Error> {
        optional_int(d, |v| (3_600..=2_592_000).contains(&v))
    }

    /// `z.number().min(0).max(1).optional()`.
    pub(crate) fn smart_turn<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
        let value = f64::deserialize(d)?;
        if (0.0..=1.0).contains(&value) {
            Ok(Some(value))
        } else {
            Err(D::Error::custom("expected a number from 0 to 1"))
        }
    }

    /// `z.number().positive().nullish()`.
    pub(crate) fn nullish_positive<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<f64>, D::Error> {
        match Option::<f64>::deserialize(d)? {
            Some(value) if value <= 0.0 => Err(D::Error::custom("expected a number > 0")),
            other => Ok(other),
        }
    }

    /// `z.string().min(1)`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct NonEmpty(pub(crate) String);

    impl<'de> Deserialize<'de> for NonEmpty {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let text = String::deserialize(deserializer)?;
            if text.is_empty() {
                return Err(D::Error::custom(
                    "too small: expected string to have >=1 characters",
                ));
            }
            Ok(Self(text))
        }
    }

    /// `z.array(z.string().min(1))` with `min..=max` items, `.optional()`.
    pub(crate) fn non_empty_strings<'de, D: Deserializer<'de>>(
        deserializer: D,
        min: usize,
        max: usize,
    ) -> Result<Option<Vec<String>>, D::Error> {
        let items = Vec::<NonEmpty>::deserialize(deserializer)?;
        if !(min..=max).contains(&items.len()) {
            return Err(D::Error::custom(format!(
                "expected an array of {min} to {max} items"
            )));
        }
        Ok(Some(items.into_iter().map(|item| item.0).collect()))
    }

    /// `z.union([z.string(), z.array(z.string())]).nullish()`: a string
    /// becomes a one-item list.
    pub(crate) fn keyterm<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Keyterm {
            One(String),
            Many(Vec<String>),
        }
        Ok(match Option::<Keyterm>::deserialize(d)? {
            None => Vec::new(),
            Some(Keyterm::One(term)) => vec![term],
            Some(Keyterm::Many(terms)) => terms,
        })
    }
}
