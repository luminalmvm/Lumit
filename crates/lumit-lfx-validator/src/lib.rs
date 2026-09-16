//! `lfx-validator` - what a vendor runs before they ship (docs/impl/lfx.md §9).
//!
//! # In plain terms
//!
//! Somebody has written an LFX plugin. This is the program that tells them
//! whether it is one. It opens their bundle the way Lumit opens it - a broker
//! per bundle, the listing read with the module shut, every frame across the
//! ring - asks it ten different questions, and prints a table with one row per
//! question and a non-zero exit if any of them came back wrong.
//!
//! Ten suites, each a named refusal in a sentence:
//!
//! | suite | what it proves |
//! |---|---|
//! | layout | every struct's size prefix is one this header can read |
//! | describe | no unstated unit, no duplicate id, a picture family, a version that is not nought |
//! | lifecycle | every step of the pinned [`suites::ACTIONS`] list driven in turn, and nothing answered after destroy |
//! | depth | the frame at fp16 **and** fp32, compared within tolerance |
//! | determinism | two runs bit-identical, and a fresh instance the same again |
//! | ROI | one bright pixel past the declared padding moves nothing; one tile against four |
//! | temporal | the declared window against what the instance asks for |
//! | threading | a stress scheduler, frames out of order, each the picture its own values make |
//! | fuzz | parameter edges, one ULP outside, NaN and both infinities, from one seed |
//! | baseline | a stored digest per depth: a moved pixel with no version bump is a refusal |
//!
//! Beside the ten there is one more heading, [`Suite::Bundle`], for the answers
//! that are the bundle's rather than any plugin's: the scan's own report lines,
//! the one ceiling a bundle can be past, and a plugin whose broker never came
//! up at all.
//!
//! **It never opens a module.** docs/12:354-356 forbids an in-process path in
//! version 1 - "one fewer code path, and the crash-isolation promise stays
//! unconditional" - and the one tool whose job is to find a crash must survive
//! the crash it finds. Everything here goes through
//! [`lumit_lfx::Broker`], which is the same supervisor the editor uses.
//!
//! **One broker per plugin.** The editor spawns one per bundle; here that would
//! mean a fuzz pass that struck one plugin out three times took the other
//! eleven down with it, and a table of "the bundle was put away" says nothing
//! about eleven plugins. The listing and the describe are the bundle's own
//! answers and are read once.
//!
//! # What it is not
//!
//! It is not a signature check and it is not an installer. A bundle this
//! program passes is a well-formed bundle, not a trusted one - §6.2's trust on
//! first use is a different question asked in a different place, and this
//! program is happy to drive a plugin nobody has ever heard of, which is
//! exactly what a vendor needs of it before they have anybody to hear of them.

pub mod baseline;
pub mod finding;
pub mod fuzz;
pub mod suites;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lumit_lfx::{bundle, BrokerError};

pub use baseline::{Baseline, BaselineError, Digests, Stored};
pub use finding::Finding;
pub use suites::{a_gradient, values_at_defaults, ACTIONS, FRAME};

/// The environment variable a fuzz seed is read from, so a failing run is
/// reproducible from the line the table printed.
pub const FUZZ_SEED_ENV: &str = "LUMIT_LFX_FUZZ_SEED";

/// The seed a run uses when nobody named one. Fixed rather than drawn from the
/// clock: a tool whose default run is different every time is a tool whose
/// green run means nothing (§9's determinism suite says the same thing about
/// the plugin).
pub const DEFAULT_SEED: u64 = 0x_1f10_5eed;

/// What a bundle may not answer at all.
#[derive(Debug, thiserror::Error)]
pub enum ValidatorError {
    /// Nothing in the bundle this machine's architecture can open.
    #[error(
        "{bundle} holds no payload this machine can open - expected one under Contents/{arch}"
    )]
    NoPayloadForThisMachine {
        /// The bundle that was asked.
        bundle: String,
        /// The first architecture directory this build looks in.
        arch: String,
    },
    /// The broker would not start, or the bundle would not describe.
    #[error("the bundle could not be opened: {0}")]
    Broker(#[from] BrokerError),
    /// The baseline file could not be read or written.
    #[error(transparent)]
    Baseline(#[from] BaselineError),
}

/// How one run is set up.
#[derive(Clone, Debug)]
pub struct Options {
    /// The `Name.lfx.bundle` directory.
    pub bundle: PathBuf,
    /// The payload inside it this machine opens, worked out by
    /// [`lumit_lfx::bundle::payload`].
    pub payload: PathBuf,
    /// Where the broker executable is, if not beside this program.
    pub broker_exe: Option<PathBuf>,
    /// The fuzz seed, printed with the table so a run can be repeated.
    pub seed: u64,
    /// The stored digests, if the run was given any.
    pub baseline: Option<PathBuf>,
    /// Whether to write what this run measured instead of comparing against it.
    /// Wants a [`Self::baseline`] to write to: the program refuses the switch
    /// without one, and a library caller that sets it alone gets a row per
    /// plugin saying nothing was written.
    pub write_baseline: bool,
    /// Plugin ids this run expects to be refused. A refusal of one of these is
    /// a report line; a **pass** of one of them is a failure, which is what
    /// keeps the flag an assertion rather than a way of turning the tool off.
    pub allow_refused: Vec<String>,
    /// How long a control message may take.
    pub control_timeout: Duration,
    /// How long one frame may take.
    pub process_timeout: Duration,
}

impl Options {
    /// The options for one bundle, with the shipped deadlines.
    ///
    /// # Errors
    ///
    /// [`ValidatorError::NoPayloadForThisMachine`] where the bundle holds
    /// nothing for this build's architecture - which is not a fault in the
    /// bundle, since §5.1's list is ordered per target and a bundle built for
    /// another CPU is still a bundle.
    pub fn over(bundle: &Path) -> Result<Self, ValidatorError> {
        let payload =
            bundle::payload(bundle).ok_or_else(|| ValidatorError::NoPayloadForThisMachine {
                bundle: bundle.display().to_string(),
                arch: bundle::arch_dirs()
                    .first()
                    .copied()
                    .unwrap_or("this machine")
                    .to_owned(),
            })?;
        Ok(Self {
            bundle: bundle.to_path_buf(),
            payload,
            broker_exe: None,
            seed: seed_from_environment(),
            baseline: None,
            write_baseline: false,
            allow_refused: Vec::new(),
            control_timeout: Duration::from_secs(10),
            process_timeout: Duration::from_secs(10),
        })
    }
}

/// The seed [`FUZZ_SEED_ENV`] names, or [`DEFAULT_SEED`].
#[must_use]
pub fn seed_from_environment() -> u64 {
    std::env::var(FUZZ_SEED_ENV)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_SEED)
}

/// One of the ten questions, or the heading the bundle's own answers sit under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Suite {
    /// Not one of the ten: what the bundle answered rather than what any plugin
    /// did - the scan's own report lines, the one ceiling that is the bundle's,
    /// and a plugin whose broker never started, which is a fault in the bundle
    /// rather than an answer to a question about pixels.
    Bundle,
    /// Size prefixes and the ABI version.
    Layout,
    /// What the sink was pushed and what it declined.
    Describe,
    /// The pinned call order, forwards and backwards.
    Lifecycle,
    /// Both mandatory depths.
    Depth,
    /// The same frame twice.
    Determinism,
    /// The declared reach, and one call against four.
    Roi,
    /// The declared window against what is asked for.
    Temporal,
    /// The stress scheduler.
    Threading,
    /// The parameter edges.
    Fuzz,
    /// The stored digest.
    Baseline,
}

impl Suite {
    /// Every heading, in the order the table prints them.
    pub const ALL: [Suite; 11] = [
        Suite::Bundle,
        Suite::Layout,
        Suite::Describe,
        Suite::Lifecycle,
        Suite::Depth,
        Suite::Determinism,
        Suite::Roi,
        Suite::Temporal,
        Suite::Threading,
        Suite::Fuzz,
        Suite::Baseline,
    ];

    /// What the table calls it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Suite::Bundle => "bundle",
            Suite::Layout => "layout",
            Suite::Describe => "describe",
            Suite::Lifecycle => "lifecycle",
            Suite::Depth => "depth",
            Suite::Determinism => "determinism",
            Suite::Roi => "ROI",
            Suite::Temporal => "temporal",
            Suite::Threading => "threading",
            Suite::Fuzz => "fuzz",
            Suite::Baseline => "baseline",
        }
    }
}

/// What one suite answered about one plugin.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// It proved what it set out to, and here is what that was.
    Passed(String),
    /// A line in the report beside a plugin that is still fine.
    Noted(Finding),
    /// The failure, which is what the exit code is made of.
    Refused(Finding),
    /// A refusal the run was told to expect, which is a line rather than a
    /// failure.
    Expected(Finding),
    /// Nothing was asked, and here is why.
    Skipped(String),
}

impl Outcome {
    /// Whether this outcome is why the program exits non-zero.
    #[must_use]
    pub const fn is_refusal(&self) -> bool {
        matches!(self, Outcome::Refused(_))
    }

    /// The cell the table prints.
    #[must_use]
    pub fn sentence(&self) -> String {
        match self {
            Outcome::Passed(what) => format!("passed - {what}"),
            Outcome::Noted(finding) => format!("noted - {finding}"),
            Outcome::Refused(finding) => format!("**REFUSED** - {finding}"),
            Outcome::Expected(finding) => format!("expected - {finding}"),
            Outcome::Skipped(why) => format!("skipped - {why}"),
        }
    }
}

/// One row of the table.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The plugin's own id, or [`Row::THE_BUNDLE`] for a line that belongs to
    /// the bundle rather than to any plugin in it.
    pub plugin: String,
    /// `major.minor.patch`, as the descriptor declared it.
    pub version: String,
    /// Which question was asked.
    pub suite: Suite,
    /// What came back.
    pub outcome: Outcome,
}

impl Row {
    /// What the plugin column says for a line the bundle itself filed.
    pub const THE_BUNDLE: &'static str = "(the bundle)";
}

/// Everything one run found.
#[derive(Clone, Debug)]
pub struct Report {
    /// The bundle's own directory name.
    pub bundle: String,
    /// How many plugins the listing claimed, read with the module shut.
    pub listed: usize,
    /// How many of them the code described.
    pub described: usize,
    /// The seed this run's fuzz suite walked, so it can be walked again.
    pub seed: u64,
    /// One row per question asked.
    pub rows: Vec<Row>,
}

impl Report {
    /// How many rows are refusals.
    #[must_use]
    pub fn refusals(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.outcome.is_refusal())
            .count()
    }

    /// Whether the bundle passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.refusals() == 0
    }

    /// The table, as the markdown that goes in a pull request - the OFX
    /// conformance bench's shape, with the context column become a suite.
    #[must_use]
    pub fn table(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "| bundle | plugin | version | suite | result |");
        let _ = writeln!(out, "|---|---|---|---|---|");
        for row in &self.rows {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} |",
                cell(&self.bundle),
                cell(&row.plugin),
                cell(&row.version),
                row.suite.name(),
                cell(&row.outcome.sentence()),
            );
        }
        let count =
            |wanted: fn(&Outcome) -> bool| self.rows.iter().filter(|r| wanted(&r.outcome)).count();
        let _ = writeln!(
            out,
            "\n{} passed, {} noted, {} expected, {} skipped, {} refused; \
             {} of {} listed effect(s) described. Fuzz seed {} ({}={}).",
            count(|o| matches!(o, Outcome::Passed(_))),
            count(|o| matches!(o, Outcome::Noted(_))),
            count(|o| matches!(o, Outcome::Expected(_))),
            count(|o| matches!(o, Outcome::Skipped(_))),
            self.refusals(),
            self.described,
            self.listed,
            self.seed,
            FUZZ_SEED_ENV,
            self.seed,
        );
        out
    }
}

/// One markdown cell: a pipe in a stranger's sentence would otherwise end the
/// column it is in.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// Drive one bundle through every suite.
///
/// # Errors
///
/// [`ValidatorError`], which is the bundle answering nothing at all: a broker
/// that would not start, a describe that never came back, or a baseline file
/// that is damaged. Everything a *plugin* does wrong is a row in the report
/// rather than an error here - a tool that stopped at the first fault would
/// make a vendor run it once per fault.
pub fn validate(options: &Options) -> Result<Report, ValidatorError> {
    let survey = suites::survey(options)?;
    let mut rows = Vec::new();

    for line in &survey.report {
        rows.push(Row {
            plugin: Row::THE_BUNDLE.to_owned(),
            version: String::new(),
            suite: Suite::Bundle,
            outcome: Outcome::Noted(Finding::Declined(line.clone())),
        });
    }
    // The one ceiling that is the bundle's rather than a plugin's. The host
    // refuses a bundle past it outright rather than truncating; here it is a
    // row, because the point of this program is that a vendor reads the
    // sentence.
    if let Some(over) =
        suites::bundle_past_its_ceiling(survey.described.len() + survey.refused.len())
    {
        rows.push(Row {
            plugin: Row::THE_BUNDLE.to_owned(),
            version: String::new(),
            suite: Suite::Bundle,
            outcome: Outcome::Refused(Finding::Declined(over)),
        });
    }

    for (id, why) in &survey.refused {
        rows.push(Row {
            plugin: id.clone(),
            version: String::new(),
            suite: Suite::Describe,
            outcome: Outcome::Refused(Finding::RefusedAtDescribe(why.clone())),
        });
        rows.push(Row {
            plugin: id.clone(),
            version: String::new(),
            suite: Suite::Lifecycle,
            outcome: Outcome::Skipped(
                "the effect was refused at describe, so nothing was driven".to_owned(),
            ),
        });
    }

    let mut baseline = match options.baseline.as_deref() {
        Some(path) => Some(Baseline::read(path)?),
        None => None,
    };

    for plugin in &survey.described {
        let id = plugin.identity.id.clone();
        let version = format!(
            "{}.{}.{}",
            plugin.identity.major, plugin.identity.minor, plugin.identity.patch
        );
        let mut push = |suite: Suite, outcomes: Vec<Outcome>| {
            for outcome in outcomes {
                rows.push(Row {
                    plugin: id.clone(),
                    version: version.clone(),
                    suite,
                    outcome,
                });
            }
        };
        push(Suite::Layout, suites::layout(plugin));
        push(Suite::Describe, suites::describe(plugin));

        let mut driver = match suites::Driver::open(options, &plugin.identity.id) {
            Ok(driver) => driver,
            Err(why) => {
                push(
                    Suite::Bundle,
                    vec![Outcome::Refused(Finding::BrokerWouldNotStart {
                        why: why.to_string(),
                    })],
                );
                continue;
            }
        };
        push(
            Suite::Lifecycle,
            suites::lifecycle(&mut driver, survey.listed),
        );
        push(Suite::Depth, suites::depth(&mut driver));
        push(Suite::Determinism, suites::determinism(&mut driver));
        push(Suite::Roi, suites::roi(&mut driver));
        push(Suite::Temporal, suites::temporal(&mut driver));
        push(
            Suite::Threading,
            suites::threading(&mut driver, options.seed),
        );
        push(Suite::Fuzz, suites::fuzz(&mut driver, options.seed));
        push(
            Suite::Baseline,
            against_the_baseline(&mut driver, baseline.as_mut(), options, &version),
        );
    }

    if options.write_baseline {
        if let (Some(path), Some(baseline)) = (options.baseline.as_deref(), baseline.as_ref()) {
            baseline.write(path)?;
        }
    }

    let mut report = Report {
        bundle: options.bundle.file_name().map_or_else(
            || options.bundle.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ),
        listed: survey.listed,
        described: survey.described.len(),
        seed: options.seed,
        rows,
    };
    expect_the_refusals(&mut report, &options.allow_refused);
    Ok(report)
}

/// The frames at the declared defaults against what was stored, or the entry
/// this run writes.
///
/// One row per depth that moved, because the two depths are two pictures and a
/// vendor who has changed one of them wants to be told which.
fn against_the_baseline(
    driver: &mut suites::Driver,
    baseline: Option<&mut Baseline>,
    options: &Options,
    version: &str,
) -> Vec<Outcome> {
    let Some(baseline) = baseline else {
        // The command line refuses `--write-baseline` with nowhere to write, so
        // this is a library caller; either way the sentence says which of the
        // two things did not happen.
        return vec![Outcome::Skipped(if options.write_baseline {
            "no baseline file was named, so nothing was written".to_owned()
        } else {
            "no baseline was named, so nothing was compared".to_owned()
        })];
    };
    let id = driver.plugin().identity.id.clone();
    let Some(now) = suites::baseline_of(driver) else {
        return vec![Outcome::Skipped(
            "the frames at the declared defaults did not render".to_owned(),
        )];
    };
    if options.write_baseline {
        let written = Finding::BaselineWritten {
            fp32: now.fp32.clone(),
            fp16: now.fp16.clone(),
            version: version.to_owned(),
        };
        baseline.plugins.insert(
            id,
            Stored {
                version: version.to_owned(),
                digests: now,
            },
        );
        return vec![Outcome::Noted(written)];
    }
    let Some(stored) = baseline.plugins.get(&id) else {
        return vec![Outcome::Refused(Finding::NoStoredBaseline)];
    };
    let mut out = Vec::new();
    for (depth, was, is) in [
        ("fp32", &stored.digests.fp32, &now.fp32),
        ("fp16", &stored.digests.fp16, &now.fp16),
    ] {
        if was == is {
            continue;
        }
        out.push(if stored.version == version {
            Outcome::Refused(Finding::PixelsMovedWithNoVersionBump {
                depth,
                stored: was.clone(),
                now: is.clone(),
                version: version.to_owned(),
            })
        } else {
            Outcome::Noted(Finding::PixelsMovedWithTheVersion {
                depth,
                stored: was.clone(),
                now: is.clone(),
                was: stored.version.clone(),
                version: version.to_owned(),
            })
        });
    }
    if out.is_empty() {
        out.push(Outcome::Passed(format!(
            "the frames are {} at fp32 and {} at fp16, as they were at version {}",
            now.fp32, now.fp16, stored.version
        )));
    }
    out
}

/// Turn the refusals the run was told to expect into lines, and the absence of
/// one into a refusal of its own.
///
/// **Only the describe refusal the flag is about.** A vendor names a plugin
/// because the host is expected to refuse it at describe; a plugin that later
/// starts describing and then fails a suite that drives frames - a moved pixel,
/// a reach it never declared, a frame that is not the frame it was a moment ago -
/// is refused, flag or no flag. Downgrading those too would make the switch
/// CI runs the program with a way of turning the program off.
fn expect_the_refusals(report: &mut Report, expected: &[String]) {
    if expected.is_empty() {
        return;
    }
    let mut met: Vec<String> = Vec::new();
    for row in &mut report.rows {
        if !expected.contains(&row.plugin) || row.suite != Suite::Describe {
            continue;
        }
        if let Outcome::Refused(finding @ Finding::RefusedAtDescribe(_)) = row.outcome.clone() {
            met.push(row.plugin.clone());
            row.outcome = Outcome::Expected(finding);
        }
    }
    for plugin in expected {
        if !met.contains(plugin) {
            report.rows.push(Row {
                plugin: plugin.clone(),
                version: String::new(),
                suite: Suite::Describe,
                outcome: Outcome::Refused(Finding::ExpectedARefusal),
            });
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{cell, Finding, Options, Outcome, Report, Row, Suite};
    use lumit_lfx::LfxRejection;

    /// The row the flag is about: the host refusing a plugin at describe.
    fn refused_at_describe(plugin: &str) -> Row {
        Row {
            plugin: plugin.to_owned(),
            version: String::new(),
            suite: Suite::Describe,
            outcome: Outcome::Refused(Finding::RefusedAtDescribe(LfxRejection::DescribeRefused {
                id: plugin.to_owned(),
            })),
        }
    }

    /// A report with one row of each kind.
    fn a_report() -> Report {
        Report {
            bundle: "Test.lfx.bundle".to_owned(),
            listed: 3,
            described: 2,
            seed: 7,
            rows: vec![
                Row {
                    plugin: "org.example.one".to_owned(),
                    version: "1.2.3".to_owned(),
                    suite: Suite::Depth,
                    outcome: Outcome::Passed("both depths agree".to_owned()),
                },
                Row {
                    plugin: "org.example.two".to_owned(),
                    version: "0.0.0".to_owned(),
                    suite: Suite::Describe,
                    outcome: Outcome::Refused(Finding::VersionIsNought),
                },
            ],
        }
    }

    /// The table is the OFX bench's five columns, one row per question, and a
    /// summary line that says how to run it again.
    #[test]
    fn the_table_is_the_conformance_benchs_shape() {
        let table = a_report().table();
        let mut lines = table.lines();
        assert_eq!(
            lines.next(),
            Some("| bundle | plugin | version | suite | result |")
        );
        assert_eq!(lines.next(), Some("|---|---|---|---|---|"));
        assert!(
            table.contains("| Test.lfx.bundle | org.example.one | 1.2.3 | depth | passed"),
            "{table}"
        );
        assert!(table.contains("**REFUSED**"), "{table}");
        assert!(
            table.contains("LUMIT_LFX_FUZZ_SEED=7"),
            "the summary says how to repeat the run: {table}"
        );
        assert!(
            table.contains("2 of 3 listed effect(s) described"),
            "{table}"
        );
    }

    /// One refusal anywhere is a run that failed, and the exit code is made of
    /// that count and nothing else.
    #[test]
    fn one_refusal_is_a_run_that_failed() {
        let report = a_report();
        assert_eq!(report.refusals(), 1);
        assert!(!report.passed());

        let mut clean = a_report();
        clean.rows.truncate(1);
        assert_eq!(clean.refusals(), 0);
        assert!(clean.passed());
    }

    /// A refusal the run expected is a line; a plugin the run expected to be
    /// refused and which was not is a refusal of its own; and a plugin the run
    /// expected to be refused **at describe** which fails a suite that drives
    /// frames is still refused. All three are what keep the flag an assertion
    /// rather than a way of turning the tool off.
    #[test]
    fn an_expected_refusal_is_a_line_and_a_missing_one_is_a_refusal() {
        let mut report = a_report();
        report.rows.truncate(1);
        report.rows.push(refused_at_describe("org.example.two"));
        super::expect_the_refusals(&mut report, &["org.example.two".to_owned()]);
        assert!(report.passed(), "{}", report.table());
        assert!(report
            .rows
            .iter()
            .any(|row| matches!(row.outcome, Outcome::Expected(_))));

        // The same plugin, allowed its describe refusal, failing the baseline.
        // The flag says nothing about that one and must not quieten it.
        let mut also_wrong = a_report();
        also_wrong.rows.truncate(1);
        also_wrong.rows.push(refused_at_describe("org.example.two"));
        also_wrong.rows.push(Row {
            plugin: "org.example.two".to_owned(),
            version: "1.0.0".to_owned(),
            suite: Suite::Baseline,
            outcome: Outcome::Refused(Finding::PixelsMovedWithNoVersionBump {
                depth: "fp16",
                stored: "abc".to_owned(),
                now: "def".to_owned(),
                version: "1.0.0".to_owned(),
            }),
        });
        super::expect_the_refusals(&mut also_wrong, &["org.example.two".to_owned()]);
        assert_eq!(
            also_wrong.refusals(),
            1,
            "a refusal that is not the describe refusal stays a refusal:\n{}",
            also_wrong.table()
        );
        assert!(!also_wrong.passed());

        let mut missing = a_report();
        super::expect_the_refusals(&mut missing, &["org.example.one".to_owned()]);
        assert_eq!(missing.refusals(), 2, "{}", missing.table());
        assert!(missing
            .rows
            .iter()
            .any(|row| row.outcome == Outcome::Refused(Finding::ExpectedARefusal)));
    }

    /// A pipe in a stranger's sentence does not end the column it is in.
    #[test]
    fn a_strangers_sentence_cannot_break_the_table() {
        assert_eq!(cell("a | b"), "a \\| b");
        assert_eq!(cell("a\nb"), "a b");
    }

    /// A folder that is not a bundle is refused before a broker is started,
    /// with the architecture this build looked for in the sentence.
    #[test]
    fn a_bundle_with_no_payload_for_this_machine_is_refused_by_name() {
        let Ok(root) = tempfile::tempdir() else {
            return;
        };
        let empty = root.path().join("Nothing.lfx.bundle");
        assert!(std::fs::create_dir_all(&empty).is_ok());
        let refused = Options::over(&empty);
        assert!(
            matches!(
                refused,
                Err(super::ValidatorError::NoPayloadForThisMachine { .. })
            ),
            "an empty bundle should be refused by name"
        );
    }
}
