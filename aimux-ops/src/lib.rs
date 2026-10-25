//! Shared asynchronous dispatch, resource ownership and wire framing for bindings.

pub mod dispatch;
pub mod envelope;
pub mod frame;
pub mod handle;
pub mod limits;
pub mod manifest;
pub mod op;
pub mod operation;

pub mod ops {
    pub mod composite;
    pub mod decision;
    pub mod files;
    pub mod handles;
    pub mod misc;
    pub mod modalities;
    pub mod operation;
    pub mod provider;
    pub mod recording;
    pub mod registry;
    pub mod replay;
    pub mod runtime;
    pub mod session;
    pub mod text;
    pub mod trace;
}

pub use dispatch::{Ctx, Dispatcher, Handler, Target};
pub use envelope::{BindingError, ErrorEnvelope, OpsError, ProtocolError};
pub use frame::{FrameStream, Outcome, StreamFrame};
pub use handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
pub use op::{ABI_VERSION, OPS_VERSION, OpDescriptor, OpMode};

#[cfg(test)]
mod dto_vectors;
