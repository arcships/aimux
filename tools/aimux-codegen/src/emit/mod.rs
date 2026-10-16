//! Language emitters consume only the normalized wire model.
use crate::model::Model;
use std::path::PathBuf;

pub trait Emitter {
    fn files(&self, model: &Model) -> Vec<(PathBuf, String)>;
}
