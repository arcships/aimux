//! Language emitters consume only the normalized wire model.
pub mod dart;
pub mod go;
pub mod node;
pub mod swift;

use crate::model::Model;
use std::path::PathBuf;

pub trait Emitter: Sync {
    /// Files and directories this emitter owns; `--check` treats anything else
    /// in them as stale.
    fn owned(&self) -> &'static [&'static str];
    fn files(&self, model: &Model) -> Vec<(PathBuf, String)>;
}

/// Every language's DTO emitter; each writes only its own binding's files.
pub static EMITTERS: &[&dyn Emitter] = &[&dart::Dart, &go::Go, &node::Node, &swift::Swift];

pub(crate) fn owned_paths() -> impl Iterator<Item = &'static str> {
    EMITTERS
        .iter()
        .flat_map(|emitter| emitter.owned().iter().copied())
}
