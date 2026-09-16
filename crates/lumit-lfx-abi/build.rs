//! Compiles the C half of the layout suite.
//!
//! `tests/layout.c` includes the canonical header and asserts every struct's
//! size and every field's offset with `sizeof`/`offsetof`, by the same numbers
//! `tests/layout.rs` asserts for the Rust mirror. The assertions are static, so
//! a header that moved a field fails this build rather than a test run; the
//! functions the file defines are called from `tests/layout.rs`, so a
//! translation unit that quietly stopped being compiled fails a test run as
//! well, and they answer the header's own constants, strings and field types
//! for the Rust half to compare.
//!
//! Two consequences of compiling it here rather than behind a feature, both
//! deliberate. It is built for **every** target and every consumer, not only
//! for `cargo test`, so a moved field is a build failure wherever this crate is
//! used; and `tests/layout.c`'s first assertion is that a pointer is eight
//! bytes, so the library itself does not build on a 32-bit target, which is the
//! same ship list docs/05-ARCHITECTURE.md already keeps. The cost is a small
//! static archive and one unreferenced symbol in anything that links the ABI
//! crate. If that ever matters, the file goes behind an `abi-layout-tests`
//! feature the dev-dependency edge turns on.

fn main() {
    println!("cargo:rerun-if-changed=tests/layout.c");
    println!("cargo:rerun-if-changed=include/lfx.h");
    cc::Build::new()
        .file("tests/layout.c")
        .include("include")
        .warnings(true)
        .compile("lumit_lfx_layout");
}
