//! Compiles the one C file this crate has (K-756).
//!
//! # In plain terms
//!
//! Four of OFX's parameter entry points take "however many arguments the caller
//! felt like passing", and Rust cannot write a function that receives one.
//! `shim/ofx_varargs.c` holds the four that can; this turns it into an object
//! the crate links, using whichever compiler the platform provides — which is
//! the point, because the rule for where a variadic argument was put is the
//! compiler's knowledge and no two platforms share it.

fn main() {
    println!("cargo:rerun-if-changed=shim/ofx_varargs.c");
    cc::Build::new()
        .file("shim/ofx_varargs.c")
        .warnings(true)
        // The shim is four functions and no state, so anything the compiler has
        // to say about it is a defect rather than a matter of taste. Both flags
        // are gcc/clang spellings and are simply skipped on MSVC.
        .flag_if_supported("-Wall")
        .flag_if_supported("-Wextra")
        .compile("lumit_ofx_varargs");
}
