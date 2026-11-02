//! The Gaggle protocol, `proto/gaggle.proto`, as tonic and prost generate
//! it. The generated code is checked in, so building Goose never needs
//! `protoc` or the generator. After changing the proto file, regenerate it
//! with `cargo run --manifest-path proto/gen/Cargo.toml`; CI runs the same
//! command with `--check` and fails on any difference.

#[allow(clippy::all, clippy::pedantic, missing_docs, unreachable_pub)]
mod generated {
    include!("goose.gaggle.v1.rs");
}

pub(crate) use generated::*;

/// The protocol version a [`Hello`] carries.
pub(crate) const PROTOCOL_VERSION: u32 = 1;
