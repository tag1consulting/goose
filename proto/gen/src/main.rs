//! Regenerates the Gaggle protocol code Goose checks in.
//!
//! `cargo run --manifest-path proto/gen/Cargo.toml` rewrites
//! `src/gaggle/proto/goose.gaggle.v1.rs` from `proto/gaggle.proto`; with
//! `--check` it changes nothing and exits non zero when the checked in file
//! is not what the proto file generates.

use std::path::Path;
use std::process::ExitCode;

const REGENERATE: &str = "cargo run --manifest-path proto/gen/Cargo.toml";

fn main() -> ExitCode {
    let check = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--check") => true,
        Some(other) => {
            eprintln!("unknown argument {other:?}: the only option is --check");
            return ExitCode::from(2);
        }
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let generated = match generate(&root) {
        Ok(generated) => generated,
        Err(e) => {
            eprintln!("could not generate the Gaggle protocol code: {e}");
            return ExitCode::FAILURE;
        }
    };
    let checked_in = root.join("src/gaggle/proto/goose.gaggle.v1.rs");
    if check {
        let current = std::fs::read_to_string(&checked_in).unwrap_or_default();
        if current == generated {
            println!("src/gaggle/proto/goose.gaggle.v1.rs is current");
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "src/gaggle/proto/goose.gaggle.v1.rs is not what proto/gaggle.proto generates; regenerate it with: {REGENERATE}"
        );
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&checked_in, &generated) {
        eprintln!("could not write {}: {e}", checked_in.display());
        return ExitCode::FAILURE;
    }
    println!("wrote src/gaggle/proto/goose.gaggle.v1.rs");
    ExitCode::SUCCESS
}

/// Compiles the proto file the way Goose checks it in: client and server, no
/// transport helpers, into a temporary folder, and returns the Rust source.
fn generate(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let fds = protox::compile(["gaggle.proto"], [root.join("proto")])?;
    let out = std::env::temp_dir().join(format!("goose-gaggle-proto-{}", std::process::id()));
    std::fs::create_dir_all(&out)?;
    let built = tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .build_transport(false)
        .out_dir(&out)
        .compile_fds(fds);
    let generated = built.map_err(Into::into).and_then(|()| {
        std::fs::read_to_string(out.join("goose.gaggle.v1.rs")).map_err(Into::into)
    });
    let _ = std::fs::remove_dir_all(&out);
    generated
}
