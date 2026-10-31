//! The Gaggle protocol, `proto/gaggle.proto`, as tonic and prost generate
//! it. The generated code is checked in, so building Goose never needs
//! `protoc`; `gaggle_proto_is_current` regenerates it and fails on any
//! difference, and `GOOSE_UPDATE_GAGGLE_PROTO=1` rewrites it.

#[allow(clippy::all, clippy::pedantic, missing_docs, unreachable_pub)]
mod generated {
    include!("goose.gaggle.v1.rs");
}

pub(crate) use generated::*;

/// The protocol version a [`Hello`] carries.
pub(crate) const PROTOCOL_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// The checked in code is what the proto file generates.
    #[test]
    fn gaggle_proto_is_current() {
        crate::gaggle::within(30, || {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"));
            let fds = protox::compile(["gaggle.proto"], [root.join("proto")]).unwrap();
            let out =
                std::env::temp_dir().join(format!("goose-gaggle-proto-{}", std::process::id()));
            std::fs::create_dir_all(&out).unwrap();
            tonic_prost_build::configure()
                .build_client(true)
                .build_server(true)
                .build_transport(false)
                .out_dir(&out)
                .compile_fds(fds)
                .unwrap();
            let generated = std::fs::read_to_string(out.join("goose.gaggle.v1.rs")).unwrap();
            let _ = std::fs::remove_dir_all(&out);
            let checked_in = root.join("src/gaggle/proto/goose.gaggle.v1.rs");
            if std::env::var_os("GOOSE_UPDATE_GAGGLE_PROTO").is_some() {
                std::fs::write(&checked_in, &generated).unwrap();
                return;
            }
            let current = std::fs::read_to_string(&checked_in).unwrap_or_default();
            assert!(
                current == generated,
                "src/gaggle/proto/goose.gaggle.v1.rs is not what proto/gaggle.proto generates: run GOOSE_UPDATE_GAGGLE_PROTO=1 cargo test --lib --features gaggle gaggle_proto_is_current"
            );
        });
    }
}
