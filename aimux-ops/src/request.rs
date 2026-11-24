use std::collections::HashMap;

use serde::de::{DeserializeOwned, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Value, value::RawValue};

use crate::envelope::{BindingError, OpsError, ProtocolError, SourceError};
use crate::op::HandleField;

/// Borrowed request JSON and the adapter's argument name for syntax errors.
#[derive(Clone, Copy)]
pub struct WireParams<'a> {
    bytes: &'a [u8],
    argument: &'static str,
}

impl<'a> WireParams<'a> {
    #[must_use]
    pub fn new(bytes: &'a [u8], argument: &'static str) -> Self {
        Self { bytes, argument }
    }

    pub(crate) fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    pub(crate) fn invalid_json(self, error: &serde_json::Error) -> OpsError {
        BindingError::InvalidWireJson {
            argument: self.argument.into(),
            message: error.to_string(),
            cause: Some(SourceError::new(error)),
        }
        .into()
    }

    pub(crate) fn decode_error(self, error: &serde_json::Error) -> OpsError {
        if error.is_data() {
            invalid_params(error)
        } else {
            self.invalid_json(error)
        }
    }

    fn value(self) -> Result<Value, OpsError> {
        serde_json::from_slice(self.bytes).map_err(|error| self.invalid_json(&error))
    }
}

pub enum Params<'a> {
    Wire(WireParams<'a>),
    Value(Value),
}

impl From<Value> for Params<'_> {
    fn from(value: Value) -> Self {
        Self::Value(value)
    }
}

impl Params<'_> {
    pub(crate) fn validate_object(&self) -> Result<(), OpsError> {
        let object = match self {
            Self::Wire(wire) => {
                match wire
                    .bytes
                    .iter()
                    .find(|&&byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
                {
                    Some(b'{') => true,
                    _ => {
                        serde_json::from_slice::<IgnoredAny>(wire.bytes)
                            .map_err(|error| wire.invalid_json(&error))?;
                        false
                    }
                }
            }
            Self::Value(value) => value.is_object(),
        };
        if object {
            Ok(())
        } else {
            Err(ProtocolError::invalid_params("operation params must be an object").into())
        }
    }

    pub(crate) fn into_value(self) -> Result<Value, OpsError> {
        match self {
            Self::Wire(wire) => wire.value(),
            Self::Value(value) => Ok(value),
        }
    }
}

/// Deserialize an object using a derived DTO without accepting positional arrays.
pub(crate) fn deserialize_object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    struct Object<T>(std::marker::PhantomData<T>);

    impl<'de, T: serde::Deserialize<'de>> Visitor<'de> for Object<T> {
        type Value = T;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object")
        }

        fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<T, M::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Object(std::marker::PhantomData))
}

pub(crate) fn invalid_params(error: &serde_json::Error) -> OpsError {
    ProtocolError::InvalidParams {
        message: error.to_string(),
        cause: Some(SourceError::new(error)),
    }
    .into()
}

pub(crate) fn decode<'a, T: DeserializeOwned>(
    params: impl Into<Params<'a>>,
) -> Result<T, OpsError> {
    match params.into() {
        Params::Wire(wire) => {
            serde_json::from_slice(wire.bytes).map_err(|error| wire.decode_error(&error))
        }
        Params::Value(value) => {
            serde_json::from_value(value).map_err(|error| invalid_params(&error))
        }
    }
}

pub(crate) fn decode_handle(raw: &RawValue) -> Result<crate::HandleId, OpsError> {
    serde_json::from_str(raw.get()).map_err(|error| {
        let mut message = error.to_string();
        if error.line() != 0 {
            let suffix = format!(" at line {} column {}", error.line(), error.column());
            if message.ends_with(&suffix) {
                message.truncate(message.len() - suffix.len());
            }
        }
        ProtocolError::InvalidParams {
            cause: Some(message.as_str().into()),
            message,
        }
        .into()
    })
}

// Capture just the values at declared handle paths, in manifest order. Raw
// leaves borrow the input so a malformed handle cannot allocate a JSON tree.
pub(crate) fn probe<'de>(
    wire: WireParams<'de>,
    fields: &[HandleField],
) -> Result<Vec<Vec<&'de RawValue>>, OpsError> {
    let mut deserializer = serde_json::Deserializer::from_slice(wire.bytes);
    let mut found = vec![Vec::new(); fields.len()];
    let result = serde::Deserializer::deserialize_map(
        &mut deserializer,
        Root {
            fields,
            found: &mut found,
        },
    )
    .and_then(|()| deserializer.end());
    result.map_err(|error| wire.invalid_json(&error))?;
    Ok(found)
}

type Capture<'a, 'de> = (&'a [&'static str], &'a mut Vec<&'de RawValue>);

struct Root<'a, 'de> {
    fields: &'a [HandleField],
    found: &'a mut [Vec<&'de RawValue>],
}

impl<'de> Visitor<'de> for Root<'_, 'de> {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an operation params object")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        while let Some(Key(key)) = map.next_key()? {
            read_field(
                &mut map,
                &key,
                self.fields
                    .iter()
                    .zip(self.found.iter_mut())
                    .map(|((path, _), slot)| (*path, slot)),
            )?;
        }
        Ok(())
    }
}

fn read_field<'a, 'de, M: MapAccess<'de>>(
    map: &mut M,
    key: &str,
    paths: impl Iterator<Item = Capture<'a, 'de>>,
) -> Result<(), M::Error>
where
    'de: 'a,
{
    let mut matches = paths.filter_map(|(path, found)| {
        let (head, tail) = path.split_first()?;
        if *head != key {
            return None;
        }
        // A later occurrence of this key replaces the entire earlier subtree.
        found.clear();
        Some((tail, found))
    });
    let Some((path, found)) = matches.next() else {
        map.next_value::<IgnoredAny>()?;
        return Ok(());
    };
    if let Some(second) = matches.next() {
        let mut paths = vec![(path, found), second];
        paths.extend(matches);
        map.next_value_seed(Branch::Shared(paths))
    } else {
        map.next_value_seed(Branch::One(Path { path, found }))
    }
}

struct Path<'a, 'de> {
    path: &'a [&'static str],
    found: &'a mut Vec<&'de RawValue>,
}

enum Branch<'a, 'de> {
    One(Path<'a, 'de>),
    Shared(Vec<Capture<'a, 'de>>),
}

impl<'de> DeserializeSeed<'de> for Branch<'_, 'de> {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        if let Self::One(Path { path: [], found }) = self {
            let raw: &RawValue = serde::Deserialize::deserialize(deserializer)?;
            if raw.get() != "null" {
                found.push(raw);
            }
            Ok(())
        } else {
            deserializer.deserialize_any(self)
        }
    }
}

impl<'de> Visitor<'de> for Branch<'_, 'de> {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON value containing a potential handle")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        let Self::One(Path { path, found }) = self else {
            let Self::Shared(mut paths) = self else {
                unreachable!()
            };
            while let Some(Key(key)) = map.next_key()? {
                read_field(
                    &mut map,
                    &key,
                    paths.iter_mut().map(|(path, slot)| (*path, &mut **slot)),
                )?;
            }
            return Ok(());
        };
        let (segment, path) = path.split_first().expect("nonempty probe path");
        if *segment == "*" {
            // Wildcards also traverse JSON objects. Preserve insertion order
            // and the last value for repeated keys, like preserve_order Value.
            let mut indices = HashMap::new();
            let mut entries = Vec::<Vec<&RawValue>>::new();
            while let Some(key) = map.next_key::<Key>()? {
                let index = *indices.entry(key).or_insert_with(|| {
                    let index = entries.len();
                    entries.push(Vec::new());
                    index
                });
                let values = &mut entries[index];
                values.clear();
                map.next_value_seed(Branch::One(Path {
                    path,
                    found: values,
                }))?;
            }
            found.extend(entries.into_iter().flatten());
        } else {
            let start = found.len();
            while let Some(Key(key)) = map.next_key()? {
                if key == *segment {
                    found.truncate(start);
                    map.next_value_seed(Branch::One(Path {
                        path,
                        found: &mut *found,
                    }))?;
                } else {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        match self {
            Self::One(Path {
                path: ["*", path @ ..],
                found,
            }) => {
                while seq
                    .next_element_seed(Branch::One(Path {
                        path,
                        found: &mut *found,
                    }))?
                    .is_some()
                {}
            }
            _ => while seq.next_element::<IgnoredAny>()?.is_some() {},
        }
        Ok(())
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
}

#[derive(PartialEq, Eq, Hash)]
pub(crate) struct Key<'de>(pub(crate) std::borrow::Cow<'de, str>);

impl<'de> serde::Deserialize<'de> for Key<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringKey;
        impl<'de> Visitor<'de> for StringKey {
            type Value = Key<'de>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object key")
            }
            fn visit_borrowed_str<E: serde::de::Error>(self, key: &'de str) -> Result<Key<'de>, E> {
                Ok(Key(key.into()))
            }
            fn visit_str<E: serde::de::Error>(self, key: &str) -> Result<Key<'de>, E> {
                Ok(Key(key.to_owned().into()))
            }
        }
        deserializer.deserialize_str(StringKey)
    }
}
