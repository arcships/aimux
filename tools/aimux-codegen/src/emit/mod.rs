//! Language emitters consume only the normalized wire model.

pub mod dart;
pub mod go;
pub mod java;
pub mod node;
pub mod python;
pub mod swift;

use crate::model::{Model, Result};
use std::path::PathBuf;

pub trait Emitter: Sync {
    /// Files and directories this emitter owns; `--check` treats anything else
    /// in them as stale.
    fn owned(&self) -> &'static [&'static str];
    /// Render files from the normalized wire model.
    ///
    /// # Errors
    /// Returns the containing type and JSON pointer for invalid model structure.
    fn files(&self, model: &Model) -> Result<Vec<(PathBuf, String)>>;
}

/// Every language's DTO emitter; each writes only its own binding's files.
pub static EMITTERS: &[&dyn Emitter] = &[
    &dart::Dart,
    &go::Go,
    &java::Java,
    &node::Node,
    &python::Python,
    &swift::Swift,
];

pub(crate) fn owned_paths() -> impl Iterator<Item = &'static str> {
    EMITTERS
        .iter()
        .flat_map(|emitter| emitter.owned().iter().copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Field, Kind, NamedType, Ty};

    #[test]
    fn missing_emitter_types_are_errors() {
        assert_eq!(
            python::Python.files(&Model { types: vec![] }).unwrap_err(),
            "ErrorEnvelope #: missing type"
        );
        let model = Model {
            types: vec![NamedType {
                name: "Example".into(),
                doc: None,
                kind: Kind::Struct {
                    closed: false,
                    fields: vec![Field {
                        json_name: "value".into(),
                        ty: Ty::Ref("Missing".into()),
                        required: false,
                        doc: None,
                    }],
                },
            }],
        };
        for emitter in [&go::Go as &dyn Emitter, &dart::Dart] {
            let error = emitter.files(&model).unwrap_err();
            assert!(error.contains("Example #/properties/value"), "{error}");
            assert!(error.contains("Missing #/$ref: unresolved type"), "{error}");
        }
    }
}
