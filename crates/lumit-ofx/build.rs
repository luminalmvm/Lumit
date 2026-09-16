//! Compiles the four parameter-suite entry points that are C-variadic in the
//! OFX header (`src/suites/variadic.c`, and the note at the top of it says
//! why). It is one of the workspace's two pieces of C; the other is
//! `lumit-lfx-abi`'s layout suite, which asserts the LFX header's own
//! `sizeof`/`offsetof` against its Rust mirror.

fn main() {
    println!("cargo:rerun-if-changed=src/suites/variadic.c");
    cc::Build::new()
        .file("src/suites/variadic.c")
        .warnings(true)
        .compile("lumit_ofx_variadic");
}
