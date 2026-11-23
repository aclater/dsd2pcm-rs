//! Gives the shared library the SONAME `libdsd2pcm.so.1` (ABI version 1) on Linux, so that
//! programs linked against it refuse a later incompatible ABI.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libdsd2pcm.so.1");
    }
}
