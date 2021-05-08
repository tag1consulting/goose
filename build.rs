use rustc_version::{version, Version};
use std::io::{self, Write};
use std::process::exit;

fn main() {
    // Goose can only be compiled with rustc version 1.49.0 or greater.
    if version().expect("failed to determine rustc version")
        < Version::parse("1.49.0").expect("failed to parse minimum required version")
    {
        writeln!(&mut io::stderr(), "goose dependency `flume` depends on `spinning_top` crate which requires rustc >= 1.49.0.").expect("failed to write to stderr");
        writeln!(
            &mut io::stderr(),
            "detected rustc version: {}",
            version().expect("failed to determine rustc version")
        )
        .expect("failed to write to stderr");
        writeln!(&mut io::stderr(), "note: see issue #55002 <https://github.com/rust-lang/rust/issues/55002> for more information").expect("failed to write to stderr");
        // Exit to avoid a more confusing error message and simplify debugging if
        // trying to build Goose with an unsupported version of rustc.
        exit(1);
    }
}
