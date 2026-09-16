//! What one suite found, by name (docs/impl/lfx.md §9).
//!
//! # In plain terms
//!
//! Every row of the validator's table says one thing, and this is the closed
//! list of things it may say. A vendor reads a sentence; a vendor's CI - and
//! this workspace's own suite - asks for one **by name**, which is why these
//! are variants rather than the strings a report of this shape usually is
//! (docs/14-ENGINEERING-RULES.md §4).
//!
//! Three of them carry an [`LfxRejection`] rather than restating it. The host's
//! own refusals already cross the pipe as themselves (§3.2), so a plugin
//! refused at describe reaches the table with the broker's sentence intact and
//! the validator adds only which suite was asking.

use lumit_lfx::LfxRejection;
use thiserror::Error;

/// One named thing a suite found.
///
/// Whether a finding ends the run is [`crate::Outcome`]'s answer rather than
/// this enum's: the same `Declined` line is a report line beside a plugin that
/// still loads, and there is no finding here that is a failure in one suite and
/// a note in another.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum Finding {
    /// The plugin never reached the catalogue, and the host said why.
    #[error("refused at describe - {0}")]
    RefusedAtDescribe(LfxRejection),
    /// One declaration the host could not keep, beside a plugin that loaded.
    #[error("a line in the scan report - {0}")]
    Declined(LfxRejection),
    /// A size prefix shorter than this header's, which is the one fault
    /// §2.1's growth mechanism exists to catch.
    #[error("a size prefix this header cannot read - {0}")]
    SizePrefixUnreadable(LfxRejection),
    /// Built against another version of the ABI.
    #[error("built against LFX_ABI_VERSION {declared}; this header is {header}")]
    AbiVersionNotThisHeader {
        /// What the plugin's own descriptor said.
        declared: u32,
        /// What `lumit-lfx-abi` publishes.
        header: u32,
    },
    /// Every number nought, which is a release that can never re-key a frame.
    #[error("the version is 0.0.0, so no release of this plugin can ever re-key a cached frame")]
    VersionIsNought,
    /// No picture family at all, so the row has no heading to sit under.
    #[error("no picture family declared, so the row has no heading")]
    NoCategoryDeclared,
    /// A control the host kept and the validator will not.
    #[error("the control `{id}` states no unit")]
    UnitUnset {
        /// The declaration's own id.
        id: String,
    },
    /// Two declarations on one id, which is one of them driving the other.
    #[error("two controls share the id `{id}`")]
    DuplicateParamId {
        /// The id they share.
        id: String,
    },
    /// An action answered for an instance that does not exist, which is the
    /// lifecycle order not being held.
    #[error("`{action}` was answered for an instance that had been destroyed")]
    ActionAnsweredOutOfOrder {
        /// Which entry point, spelled as [`crate::suites::ACTIONS`] spells it.
        action: &'static str,
    },
    /// An action on a **live** instance answered nothing, which is the pinned
    /// order not being drivable at all.
    #[error("`{action}` on a live instance answered nothing - {why}")]
    ActionWouldNotAnswer {
        /// Which entry point, spelled as [`crate::suites::ACTIONS`] spells it.
        action: &'static str,
        /// The host's own sentence.
        why: String,
    },
    /// This plugin's own broker never came up, so none of the ten questions was
    /// ever put to it.
    #[error("no broker of its own would start - {why}")]
    BrokerWouldNotStart {
        /// The host's own sentence.
        why: String,
    },
    /// The broker is up and the plugin would not make an instance.
    #[error("no instance was created - {why}")]
    InstanceWouldNotCreate {
        /// The host's own sentence.
        why: String,
    },
    /// One of the two mandatory depths did not render at all.
    #[error("no frame at {depth} - {why}")]
    DepthRefused {
        /// `fp16` or `fp32`.
        depth: &'static str,
        /// The host's own sentence.
        why: String,
    },
    /// A frame came back at a depth nobody asked for. §2.5's rule is that the
    /// host never converts between the two, so a depth that crossed the ring as
    /// the other one is a picture the caller has nowhere to put.
    #[error("the frame asked for at {asked} came back at {answered}")]
    DepthCrossedAsTheOther {
        /// The depth the job named.
        asked: &'static str,
        /// The depth the ring carried.
        answered: &'static str,
    },
    /// Both depths rendered and they are not the same picture.
    #[error("fp16 and fp32 disagree by {worst} at sample {at}, past a tolerance of {tolerance}")]
    DepthsDisagree {
        /// The widest difference found.
        worst: f32,
        /// Where it was found.
        at: usize,
        /// What was allowed at that magnitude.
        tolerance: f32,
    },
    /// The same frame rendered twice is not the same frame.
    #[error("two renders of one frame differ at sample {at}")]
    NotDeterministic {
        /// The first sample that differs.
        at: usize,
    },
    /// A pixel outside the declared reach moved the output, which is the tile
    /// seam §4.6 calls a correctness bug.
    #[error(
        "a pixel {distance} px outside the region moved the output at sample {at}, \
         and the declared padding is {padding} px"
    )]
    RoiNotHonoured {
        /// What the trait block declared.
        padding: f32,
        /// How far outside the **region** the changed pixel was put, which is
        /// the declared padding and one pixel more - the distance that makes
        /// the move a reach the plugin never declared rather than one it did.
        distance: u32,
        /// The first sample inside the region that moved.
        at: usize,
    },
    /// The frame rendered in four regions is not the frame rendered in one.
    #[error("four tiles do not make the frame one call makes, at sample {at}")]
    TilesDisagree {
        /// The first sample that differs.
        at: usize,
    },
    /// The instance asked for a neighbour it never declared it would read.
    #[error(
        "the frame at offset {offset} was asked for, outside the declared window [{lo}, {hi}]"
    )]
    AskedOutsideTheDeclaredWindow {
        /// The offset answered.
        offset: i32,
        /// The declared first frame.
        lo: i32,
        /// The declared last frame.
        hi: i32,
    },
    /// A frame came back carrying another frame's numbers, which is the fault
    /// §4.4's lease exists to make impossible.
    #[error("the frame dispatched at step {step} came back painted with another frame's values")]
    PaintedWithAnothersValues {
        /// Which dispatch, in the stress pass's own order.
        step: usize,
    },
    /// Nothing was ever in flight beside anything else, so the stress pass
    /// proved an absence by never creating the presence (§11 item 12).
    #[error(
        "at most {most} frame(s) were ever in flight at once, so no overlap happened \
         and no absence is proved"
    )]
    NothingOverlapped {
        /// The most frames this run ever had in flight at one moment.
        most: usize,
    },
    /// A value the plugin's own declaration admits produced a picture that is
    /// not numbers.
    #[error("the control `{id}` at {value} is inside its own declared bounds and the output at sample {at} is not a number")]
    NotFiniteInsideItsOwnBounds {
        /// The declaration's id.
        id: String,
        /// The value it was driven at.
        value: String,
        /// The first sample that is not finite.
        at: usize,
    },
    /// The host could not get a frame at all out of an edge value.
    #[error("no answer for the control `{id}` at {value} - {why}")]
    NoAnswer {
        /// The declaration's id.
        id: String,
        /// The value it was driven at.
        value: String,
        /// The host's own sentence.
        why: String,
    },
    /// The maths moved and the version did not, which is the one obligation
    /// nothing can enforce and `--baseline` is the only pressure on.
    #[error(
        "the {depth} pixels moved and version {version} did not: {stored} is stored, \
         this run is {now}"
    )]
    PixelsMovedWithNoVersionBump {
        /// Which of the two mandatory depths moved.
        depth: &'static str,
        /// The digest the baseline holds.
        stored: String,
        /// The digest this run produced.
        now: String,
        /// The version both were taken at.
        version: String,
    },
    /// The maths moved and so did the version, which is the whole point of
    /// keeping a baseline and is not a fault.
    #[error(
        "the {depth} frame moved from {stored} to {now} and the version moved \
         from {was} to {version}"
    )]
    PixelsMovedWithTheVersion {
        /// Which of the two mandatory depths moved.
        depth: &'static str,
        /// The digest the baseline holds.
        stored: String,
        /// The digest this run produced.
        now: String,
        /// The version the baseline was taken at.
        was: String,
        /// The version this run is.
        version: String,
    },
    /// Nothing stored to compare against yet.
    #[error("no stored baseline for this plugin - run again with --write-baseline")]
    NoStoredBaseline,
    /// What a `--write-baseline` run wrote down, which is a line rather than a
    /// fault: the run that creates a record has nothing to compare it against
    /// and must not tell a vendor to run the command they have just run.
    #[error("recorded at version {version}: {fp32} at fp32 and {fp16} at fp16")]
    BaselineWritten {
        /// The fp32 frame's digest.
        fp32: String,
        /// The fp16 frame's digest.
        fp16: String,
        /// The version it was recorded at.
        version: String,
    },
    /// The run was told this plugin would be refused and it was not, which is
    /// what keeps `--allow-refused` an assertion rather than a way of turning
    /// the tool off.
    #[error("this plugin was expected to be refused and nothing refused it")]
    ExpectedARefusal,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::Finding;
    use lumit_lfx::{subject, Ceiling, LfxRejection};

    /// Every name the closed list holds, written out once.
    ///
    /// This is what makes the sweep below a sweep rather than a walk over
    /// whatever somebody remembered to add. Three things are held to each
    /// other: this list, [`named`]'s `match` - which has no `_` arm, so a
    /// variant added later does not compile until it is answered for - and one
    /// sample of each variant. A new finding cannot reach a vendor's table
    /// without a sentence this test has read.
    const KINDS: [&str; 28] = [
        "RefusedAtDescribe",
        "Declined",
        "SizePrefixUnreadable",
        "AbiVersionNotThisHeader",
        "VersionIsNought",
        "NoCategoryDeclared",
        "UnitUnset",
        "DuplicateParamId",
        "ActionAnsweredOutOfOrder",
        "ActionWouldNotAnswer",
        "BrokerWouldNotStart",
        "InstanceWouldNotCreate",
        "DepthRefused",
        "DepthCrossedAsTheOther",
        "DepthsDisagree",
        "NotDeterministic",
        "RoiNotHonoured",
        "TilesDisagree",
        "AskedOutsideTheDeclaredWindow",
        "PaintedWithAnothersValues",
        "NothingOverlapped",
        "NotFiniteInsideItsOwnBounds",
        "NoAnswer",
        "PixelsMovedWithNoVersionBump",
        "PixelsMovedWithTheVersion",
        "NoStoredBaseline",
        "BaselineWritten",
        "ExpectedARefusal",
    ];

    /// What one finding is called, through a `match` with no `_` arm.
    fn named(finding: &Finding) -> &'static str {
        match finding {
            Finding::RefusedAtDescribe(..) => "RefusedAtDescribe",
            Finding::Declined(..) => "Declined",
            Finding::SizePrefixUnreadable(..) => "SizePrefixUnreadable",
            Finding::AbiVersionNotThisHeader { .. } => "AbiVersionNotThisHeader",
            Finding::VersionIsNought => "VersionIsNought",
            Finding::NoCategoryDeclared => "NoCategoryDeclared",
            Finding::UnitUnset { .. } => "UnitUnset",
            Finding::DuplicateParamId { .. } => "DuplicateParamId",
            Finding::ActionAnsweredOutOfOrder { .. } => "ActionAnsweredOutOfOrder",
            Finding::ActionWouldNotAnswer { .. } => "ActionWouldNotAnswer",
            Finding::BrokerWouldNotStart { .. } => "BrokerWouldNotStart",
            Finding::InstanceWouldNotCreate { .. } => "InstanceWouldNotCreate",
            Finding::DepthRefused { .. } => "DepthRefused",
            Finding::DepthCrossedAsTheOther { .. } => "DepthCrossedAsTheOther",
            Finding::DepthsDisagree { .. } => "DepthsDisagree",
            Finding::NotDeterministic { .. } => "NotDeterministic",
            Finding::RoiNotHonoured { .. } => "RoiNotHonoured",
            Finding::TilesDisagree { .. } => "TilesDisagree",
            Finding::AskedOutsideTheDeclaredWindow { .. } => "AskedOutsideTheDeclaredWindow",
            Finding::PaintedWithAnothersValues { .. } => "PaintedWithAnothersValues",
            Finding::NothingOverlapped { .. } => "NothingOverlapped",
            Finding::NotFiniteInsideItsOwnBounds { .. } => "NotFiniteInsideItsOwnBounds",
            Finding::NoAnswer { .. } => "NoAnswer",
            Finding::PixelsMovedWithNoVersionBump { .. } => "PixelsMovedWithNoVersionBump",
            Finding::PixelsMovedWithTheVersion { .. } => "PixelsMovedWithTheVersion",
            Finding::NoStoredBaseline => "NoStoredBaseline",
            Finding::BaselineWritten { .. } => "BaselineWritten",
            Finding::ExpectedARefusal => "ExpectedARefusal",
        }
    }

    /// One of each, in the order the enum declares them.
    fn one_of_each() -> Vec<Finding> {
        vec![
            Finding::RefusedAtDescribe(LfxRejection::DescribeRefused {
                id: "org.example.one".to_owned(),
            }),
            Finding::Declined(LfxRejection::PastCeiling {
                subject: subject::CONTROL_LABEL,
                ceiling: Ceiling::StringBytes,
                given: 9_000,
            }),
            Finding::SizePrefixUnreadable(LfxRejection::UnreadableDeclaration {
                kind: lumit_lfx_abi::LFX_PARAM_FLOAT,
                bytes: 4,
            }),
            Finding::AbiVersionNotThisHeader {
                declared: 2,
                header: 1,
            },
            Finding::VersionIsNought,
            Finding::NoCategoryDeclared,
            Finding::UnitUnset {
                id: "gain".to_owned(),
            },
            Finding::DuplicateParamId {
                id: "gain".to_owned(),
            },
            Finding::ActionAnsweredOutOfOrder { action: "process" },
            Finding::ActionWouldNotAnswer {
                action: "set-values",
                why: "the plugin went away".to_owned(),
            },
            Finding::BrokerWouldNotStart {
                why: "the broker executable was not found".to_owned(),
            },
            Finding::InstanceWouldNotCreate {
                why: "the plugin refused".to_owned(),
            },
            Finding::DepthRefused {
                depth: "fp16",
                why: "the plugin refused".to_owned(),
            },
            Finding::DepthCrossedAsTheOther {
                asked: "fp16",
                answered: "fp32",
            },
            Finding::DepthsDisagree {
                worst: 0.5,
                at: 12,
                tolerance: 0.01,
            },
            Finding::NotDeterministic { at: 4 },
            Finding::RoiNotHonoured {
                padding: 8.0,
                distance: 9,
                at: 0,
            },
            Finding::TilesDisagree { at: 7 },
            Finding::AskedOutsideTheDeclaredWindow {
                offset: 4,
                lo: -1,
                hi: 1,
            },
            Finding::PaintedWithAnothersValues { step: 3 },
            Finding::NothingOverlapped { most: 1 },
            Finding::NotFiniteInsideItsOwnBounds {
                id: "gain".to_owned(),
                value: "1".to_owned(),
                at: 0,
            },
            Finding::NoAnswer {
                id: "gain".to_owned(),
                value: "inf".to_owned(),
                why: "the plugin went away".to_owned(),
            },
            Finding::PixelsMovedWithNoVersionBump {
                depth: "fp32",
                stored: "abc".to_owned(),
                now: "def".to_owned(),
                version: "1.0.0".to_owned(),
            },
            Finding::PixelsMovedWithTheVersion {
                depth: "fp16",
                stored: "abc".to_owned(),
                now: "def".to_owned(),
                was: "1.0.0".to_owned(),
                version: "1.0.1".to_owned(),
            },
            Finding::NoStoredBaseline,
            Finding::BaselineWritten {
                fp32: "abc".to_owned(),
                fp16: "def".to_owned(),
                version: "1.0.0".to_owned(),
            },
            Finding::ExpectedARefusal,
        ]
    }

    /// Every finding prints one sentence with no debug formatting in it, which
    /// is what puts it in a markdown cell - and **every** means every: the
    /// samples are held to [`KINDS`] both ways round, so a variant added with a
    /// newline or a pipe in its `#[error]` string fails here rather than
    /// shipping and breaking the column this test exists to protect.
    #[test]
    fn every_finding_is_one_sentence() {
        let each = one_of_each();
        for finding in &each {
            let sentence = finding.to_string();
            assert!(!sentence.is_empty(), "{finding:?} prints nothing");
            assert!(
                !sentence.contains('\n') && !sentence.contains('|'),
                "{finding:?} cannot go in a markdown cell: {sentence}"
            );
        }

        let mut swept: Vec<&str> = each.iter().map(named).collect();
        swept.sort_unstable();
        let mut every = KINDS.to_vec();
        every.sort_unstable();
        assert_eq!(
            swept, every,
            "the sweep is the closed list: every finding has one sample and \
             every sample has a name"
        );
    }
}
