//! `lumit-media-broker`, the program an Illustrator file is read in.
//!
//! # In plain terms
//!
//! Lumit starts one of these for each read of an `.ai` file and waits for its
//! answer. The file is a stranger's bytes, and a bad one can ask for more
//! memory or time than there is. Here that ends this program and nothing
//! else: Lumit holds it to a memory cap and a deadline, and reports an error
//! when it fails.
//!
//! It reads its arguments, writes the answer to standard output and exits.
//! What it is asked and what it answers are in `lumit_media::ai`.

use std::ffi::OsString;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match lumit_media::ai::serve(&args, &mut std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("{why}");
            ExitCode::FAILURE
        }
    }
}
