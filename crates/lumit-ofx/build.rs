//! Compiles the one piece of C in the workspace: the four parameter-suite
//! entry points that are C-variadic in the OFX header
//! (`src/suites/variadic.c`, and the note at the top of it says why).

fn main() {
    println!("cargo:rerun-if-changed=src/suites/variadic.c");
    cc::Build::new()
        .file("src/suites/variadic.c")
        .warnings(true)
        .compile("lumit_ofx_variadic");
}
