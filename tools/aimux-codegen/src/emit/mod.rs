//! Language emitters consume only the normalized wire model.
use crate::model::Model;
use std::path::PathBuf;

pub trait Emitter: Sync {
    fn files(&self, model: &Model) -> Vec<(PathBuf, String)>;
}

/// Every language's DTO emitter; each writes only its own binding's files.
pub static EMITTERS: &[&dyn Emitter] = &[];
