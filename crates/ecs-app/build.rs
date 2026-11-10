//! Windows (MSVC): raylib defines `CloseWindow` and `ShowCursor`, which user32
//! (linked through the `windows` crate) also exports. Let the linker keep the
//! first definition instead of failing with LNK2005; raylib's own calls resolve
//! to its own functions.

fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows && msvc {
        println!("cargo:rustc-link-arg-bins=/FORCE:MULTIPLE");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
