//! `lfx-validator` - the program (docs/impl/lfx.md §9).
//!
//! # In plain terms
//!
//! A shell around [`lumit_lfx_validator::validate`]: it reads a bundle path and
//! a few switches, prints the table, and exits non-zero if anything in it is a
//! refusal. Everything it knows is in the library beside it, so a vendor's CI
//! and this workspace's own suite are asking the same code the same questions.
//!
//! ```text
//! lfx-validator [options] <Name.lfx.bundle>
//!
//!   --broker <path>        the lumit-lfx-broker executable, if not beside this one
//!   --seed <n>             the fuzz seed (default: LUMIT_LFX_FUZZ_SEED, then a fixed one)
//!   --baseline <path>      compare the frames against this stored file
//!   --write-baseline       write what this run measured instead of comparing (wants --baseline)
//!   --allow-refused <id>   this plugin is expected to be refused; passing is the failure
//!   --timeout <seconds>    how long one frame or one control message may take
//! ```
//!
//! Exit code: 0 when nothing was refused, 1 when something was, 2 when the
//! bundle could not be opened at all or the line could not be read. The three
//! are different on purpose - a bundle that will not start is a different
//! message to a vendor than a bundle that started and is wrong. `--help` is a
//! line that asked for the help and got it: it prints on standard output and
//! exits 0, because a vendor's CI that runs it as a smoke check is not a run
//! that failed.
//!
//! **This file** is in `EXEMPT_FILES` in
//! `lumit-bridge/tests/no_panicking_prints.rs` - the file and not the crate,
//! since the library beside it is a library like any other and §11 item 13 is
//! about exactly that. Like the brokers and the bench, the program's printed
//! output **is** its product, and a print whose write fails here costs a run
//! that was already over.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use lumit_lfx_validator::{seed_from_environment, validate, Options, ValidatorError};

/// What the command line said.
struct Args {
    /// The bundle to drive.
    bundle: PathBuf,
    /// Where the broker executable is, if not beside this one.
    broker: Option<PathBuf>,
    /// The fuzz seed.
    seed: u64,
    /// The stored digests.
    baseline: Option<PathBuf>,
    /// Whether to write them rather than compare against them.
    write_baseline: bool,
    /// Plugin ids this run expects to be refused.
    allow_refused: Vec<String>,
    /// How long one action may take.
    timeout: Duration,
}

/// The usage text, which is also the whole of the help.
const USAGE: &str = "\
lfx-validator - drive an LFX bundle through the broker and say what is wrong with it

    lfx-validator [options] <Name.lfx.bundle>

    --broker <path>        the lumit-lfx-broker executable, if not beside this one
    --seed <n>             the fuzz seed (default: LUMIT_LFX_FUZZ_SEED, then a fixed one)
    --baseline <path>      compare the frames against this stored file
    --write-baseline       write what this run measured instead of comparing (wants --baseline)
    --allow-refused <id>   this plugin is expected to be refused; passing is the failure
    --timeout <seconds>    how long one frame or one control message may take
    -h, --help             this text
";

/// What one line asked for.
enum Parsed {
    /// Drive a bundle.
    Run(Args),
    /// Print the usage and stop, which is not a failure.
    Help,
}

/// Read the command line, or say what was wrong with it.
fn parse(mut argv: impl Iterator<Item = String>) -> Result<Parsed, String> {
    let mut bundle: Option<PathBuf> = None;
    let mut args = Args {
        bundle: PathBuf::new(),
        broker: None,
        seed: seed_from_environment(),
        baseline: None,
        write_baseline: false,
        allow_refused: Vec::new(),
        timeout: Duration::from_secs(10),
    };
    while let Some(word) = argv.next() {
        let mut value = |what: &str| argv.next().ok_or(format!("{what} wants a value"));
        match word.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "--broker" => args.broker = Some(PathBuf::from(value("--broker")?)),
            "--baseline" => args.baseline = Some(PathBuf::from(value("--baseline")?)),
            "--write-baseline" => args.write_baseline = true,
            "--allow-refused" => args.allow_refused.push(value("--allow-refused")?),
            "--seed" => {
                let text = value("--seed")?;
                args.seed = text
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| format!("--seed wants a whole number, not {text:?}"))?;
            }
            "--timeout" => {
                let text = value("--timeout")?;
                let seconds = text
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| format!("--timeout wants whole seconds, not {text:?}"))?;
                args.timeout = Duration::from_secs(seconds.max(1));
            }
            other if other.starts_with('-') => return Err(format!("{other} is not an option")),
            other if bundle.is_none() => bundle = Some(PathBuf::from(other)),
            other => {
                return Err(format!(
                    "only one bundle at a time, and {other} is a second"
                ))
            }
        }
    }
    args.bundle = bundle.ok_or_else(|| "no bundle was named".to_owned())?;
    // A run that asked to write a record and named nowhere to write it would
    // otherwise measure everything, store nothing and exit 0 - a green run and
    // no file, which a vendor finds out about at their next release.
    if args.write_baseline && args.baseline.is_none() {
        return Err("--write-baseline wants --baseline <path> to write to".to_owned());
    }
    Ok(Parsed::Run(args))
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(Parsed::Run(args)) => args,
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(why) => {
            eprintln!("{why}");
            return ExitCode::from(2);
        }
    };

    let mut options = match Options::over(&args.bundle) {
        Ok(options) => options,
        Err(why) => {
            eprintln!("lfx-validator: {why}");
            return ExitCode::from(2);
        }
    };
    options.broker_exe = args.broker;
    options.seed = args.seed;
    options.baseline = args.baseline;
    options.write_baseline = args.write_baseline;
    options.allow_refused = args.allow_refused;
    options.control_timeout = args.timeout;
    options.process_timeout = args.timeout;

    match validate(&options) {
        Ok(report) => {
            print!("{}", report.table());
            if report.passed() {
                ExitCode::SUCCESS
            } else {
                eprintln!(
                    "lfx-validator: {} refusal(s) in {}",
                    report.refusals(),
                    report.bundle
                );
                ExitCode::FAILURE
            }
        }
        Err(
            why @ ValidatorError::Broker(_) | why @ ValidatorError::NoPayloadForThisMachine { .. },
        ) => {
            eprintln!("lfx-validator: {why}");
            ExitCode::from(2)
        }
        Err(why) => {
            eprintln!("lfx-validator: {why}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{parse, Parsed};

    /// A line of arguments, as the shell would hand it over.
    fn words(line: &[&str]) -> std::vec::IntoIter<String> {
        line.iter()
            .map(|word| (*word).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// Every switch reaches the options it names, and the bundle is the one
    /// word that is not a switch.
    #[test]
    fn the_command_line_reaches_the_options() {
        let parsed = parse(words(&[
            "--seed",
            "42",
            "--baseline",
            "base.json",
            "--write-baseline",
            "--allow-refused",
            "org.example.broken",
            "--timeout",
            "3",
            "Test.lfx.bundle",
        ]))
        .ok();
        let Some(Parsed::Run(args)) = parsed else {
            panic!("the line should parse as a run");
        };
        assert_eq!(args.seed, 42);
        assert_eq!(
            args.baseline.as_deref(),
            Some(std::path::Path::new("base.json"))
        );
        assert!(args.write_baseline);
        assert_eq!(args.allow_refused, vec!["org.example.broken".to_owned()]);
        assert_eq!(args.timeout.as_secs(), 3);
        assert_eq!(args.bundle, std::path::Path::new("Test.lfx.bundle"));
    }

    /// Everything that is not a run is a sentence rather than a panic: no
    /// bundle, two bundles, an unknown switch, a switch with nothing after it,
    /// a seed that is not a number, and a run that asked to write a record
    /// without saying where.
    #[test]
    fn a_line_that_is_not_a_run_is_a_sentence() {
        for line in [
            vec![],
            vec!["one.lfx.bundle", "two.lfx.bundle"],
            vec!["--nonsense", "one.lfx.bundle"],
            vec!["--seed"],
            vec!["--seed", "later", "one.lfx.bundle"],
            vec!["--write-baseline", "one.lfx.bundle"],
        ] {
            let refused = parse(words(&line));
            assert!(refused.is_err(), "{line:?} should not be a run");
        }
    }

    /// Asking for the help is a line that got what it asked for, so it prints
    /// on standard output and exits 0 rather than going out as a parse failure
    /// with the code a bundle that would not open uses.
    #[test]
    fn asking_for_the_help_is_not_a_failed_run() {
        for line in [vec!["-h"], vec!["--help"], vec!["--help", "one.lfx.bundle"]] {
            assert!(
                matches!(parse(words(&line)), Ok(Parsed::Help)),
                "{line:?} asked for the help"
            );
        }
    }
}
