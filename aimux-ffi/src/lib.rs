//! C ABI adapter for the shared aimux-ops dispatch layer.
//!
//! C consumers use the versioned primitives in [`abi`]. Native Node and
//! Python bindings call aimux-ops directly. The generated C header lives at
//! `include/aimux.h`; operation-specific behavior belongs in aimux-ops.

// C callers must supply valid pointers under the ABI ownership contract.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

pub mod abi;

pub use abi::*;
