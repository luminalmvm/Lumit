//! The supervisor: spawning a broker, watching it, and outliving it
//! (docs/impl/lfx.md §3.3, §3.5).
//!
//! # In plain terms
//!
//! This is the half of out-of-process hosting that stays in Lumit. It starts a
//! second program, hands it one bundle and a pipe, and from then on talks to
//! the plugin only through that pipe. It never calls the plugin. That is the
//! whole promise of docs/12 §1: a plugin cannot take Lumit down, because a
//! plugin is not in Lumit.
//!
//! Five things it does that are worth reading before changing anything here.
//!
//! **The manifest is read before any of the plugin's code.**
//! [`Broker::manifest`] asks the second process for the bundle's own
//! `Contents/lfx.toml` and gets back a listing; no module is opened to answer
//! it. That is what lets the Addons page name, label and re-enable a plugin
//! whose code has never run, and it is why [`Broker::describe`] reads the
//! listing first rather than trusting its caller to have asked. **The manifest
//! is the cheap listing, never the authority**: once the module really is open,
//! [`crate::manifest::agrees`] compares the two and a disagreement is
//! [`LfxRejection::ManifestMismatch`] - the code wins.
//!
//! **The module is opened lazily, on the first describe**, which carries the
//! disable list. `lumit-aplug`'s ordering rather than `lumit-ofx`'s, where the
//! bundle is opened straight after the handshake and a switched-off plugin's
//! load action fires anyway. A plugin the user has switched off never has its
//! `init` called at all (§5.4, the first of the three places a disable reaches).
//!
//! **Handles are minted here and only quoted back.** An instance's id is a
//! [`Handle`] with magic and a kind in it, and the host holds the record - which
//! plugin, which values - for every live one. That is what makes a restart a
//! *replay*: the same ids come back, naming the same plugins **by the
//! descriptor's own reverse-DNS id, never by a position in the last
//! `Described`**, because the disable list decides that list's membership and a
//! list that has lost a row renumbers the rest of it.
//!
//! **Three consecutive failures disable the plugin for the session.** A missed
//! deadline and a dead process are the same kind of event: a strike. One or two
//! cost that frame and buy a restart; the third stops trying, and every frame
//! from then on comes back with a sentence so the layer renders identity and
//! wears a badge. A successful action puts the count back to nought -
//! *consecutive* is the word docs/12 §2.3 uses and it is the word this obeys.
//!
//! **And the ring is the ledger's.** Unlike the two older hosts this one pays
//! the governor for the shared memory it maps, holds the reservation inside the
//! [`Ring`] that spent it, and - when a wider ring is needed - **drops the ring
//! it is replacing before it asks for the new one**, so the ledger is never
//! asked for both at once ([`Broker::fit`], docs/impl/lfx.md §3.4).
//!
//! # Thread role
//!
//! One [`Broker`] per bundle, owned by whoever opened it. Every method here is
//! the caller's thread; the only other thread is [`read_loop`], which holds no
//! lock and takes none, so the host waits on a deadline rather than on a plugin
//! (docs/14 §1). Nothing here touches the GPU and nothing here is called with a
//! lock held.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use half::f16;
use lumit_budget::Ledger;
use thiserror::Error;

use crate::ipc::handles::{Handle, KIND_INSTANCE};
use crate::ipc::pipe::{self, Listener, PipeError, SendHalf};
use crate::ipc::proto::{
    BrokerMessage, DescribedPlugin, HostMessage, InstanceId, NoteKind, ParamValue, PixelDepth,
    PluginIdentity, ProcessRequest, RectI, Slot, PROTOCOL_VERSION,
};
use crate::ipc::ring::{Ring, RingError, RingPlan, RingSlots};
use crate::quirks::{describe_deadline, Quirks};
use crate::{schema, LfxRejection};

/// How many consecutive failures a plugin gets before it is put away for the
/// session, how long the host waits for a freshly spawned broker to connect and
/// say hello, and what a switched-off plugin files. All three are `lumit-ipc`'s:
/// they are answers every plugin host must give the same way, not this one's to
/// choose (docs/impl/lfx.md §3.1).
pub use lumit_ipc::{DISABLED_REASON, HANDSHAKE_TIMEOUT, STRIKES_BEFORE_DISABLED};

/// The environment variable that overrides where the broker executable is, and
/// that executable's file name. Both are this host's own rather than the
/// transport's, for the reason [`crate::ipc::identity`] gives.
pub use crate::ipc::identity::{broker_exe_name, BROKER_EXE_ENV};

/// How many instances of one bundle may be live at once.
///
/// Carried over from the older host unchanged, and it bounds two things rather
/// than one: the memory a runaway caller can ask a broker for, and the length
/// of the replay a restart has to perform.
pub const MAX_LIVE_INSTANCES: usize = 1_024;

/// How many lines one bundle may say before the host stops keeping them.
///
/// A plugin in a loop is a plugin that would otherwise fill the process it is
/// talking to. The newest are kept: what a user wants after a failure is what
/// was said just before it.
pub const MAX_NOTES: usize = 64;

/// The plugins the user has switched off, shared with whoever edits the list.
///
/// Read **before describe**, so a switched-off plugin is never described and
/// never created, and none of its own code runs. The module it shares with the
/// plugins that are on is still opened for them - it is discovery that declines
/// to ask for a describe at all when a bundle has nothing left to describe,
/// which is what keeps a one-plugin bundle's `init` from running after the tick
/// (§5.4 place 1). The owner of the list is whoever reads
/// `lumit_project::PluginPrefs`; this crate only reads what it is handed, which
/// is what keeps the plugin host free of a dependency on the project format.
///
/// *ponytail:* the second and third places a disable reaches - the per-render
/// gate that stops a plugin switched off mid-session **now**, and the catalogue
/// filter that takes its row out of Effects & presets - are discovery's and the
/// bridge's (§5.4), and land with discovery and the bridge surface. What is here is the first, the
/// one that belongs to the conversation.
pub type DisableList = Arc<Mutex<BTreeSet<String>>>;

/// An empty list - nothing switched off.
#[must_use]
pub fn nothing_disabled() -> DisableList {
    Arc::new(Mutex::new(BTreeSet::new()))
}

/// Where the broker executable is: beside Lumit's own, which is where every
/// packaging step puts it. `lumit-ipc` does the looking; the two strings that
/// say which program is being looked for are this host's.
#[must_use]
pub fn broker_exe() -> PathBuf {
    lumit_ipc::broker_exe(broker_exe_name(), BROKER_EXE_ENV)
}

/// What can go wrong before there is a frame to blame.
#[derive(Debug, Error)]
pub enum BrokerError {
    /// The executable would not start.
    #[error("the plugin broker would not start: {0}")]
    Spawn(std::io::Error),
    /// The pipe.
    #[error(transparent)]
    Pipe(#[from] PipeError),
    /// The ring.
    #[error(transparent)]
    Ring(#[from] RingError),
    /// A regrow left the broker with no ring at all.
    #[error("the frame ring was never opened")]
    NoRing,
    /// The broker never connected, or never said hello.
    #[error("the plugin broker did not answer when it started")]
    NoHandshake,
    /// The broker speaks another version of the protocol. Refused here, with a
    /// sentence, rather than deserialised into whatever it happens to mean -
    /// and refused **after** the proof, so an impostor is not told which build
    /// it faces.
    #[error("the plugin broker speaks protocol {theirs}, this host speaks {ours}")]
    ProtocolMismatch {
        /// What the broker said.
        theirs: u32,
        /// What this host speaks.
        ours: u32,
    },
    /// The plugin has used up its three strikes.
    #[error("the plugin is disabled for this session")]
    Disabled,
    /// The **user** switched this plugin off, and the gate read the running
    /// list on the way in (docs/impl/lfx.md §5.4).
    ///
    /// Told apart from [`BrokerError::Disabled`] deliberately: that one is the
    /// host putting a bundle away after three strikes, and this one is a person
    /// ticking a box. The `Display` is `lumit-ipc`'s shared
    /// [`DISABLED_REASON`] rather than a sentence of
    /// this host's, because the seam that badges the layer decides "switched
    /// off" rather than "failed" by string equality against that one constant
    /// over a table every hosted effect files into - so a second host filing a
    /// string of its own badges its layers with the wrong sentence (§4.3).
    #[error("{}", lumit_ipc::DISABLED_REASON)]
    SwitchedOff,
    /// A message arrived that made no sense where it arrived.
    ///
    /// **The payload is the message's own name**, from
    /// [`BrokerMessage::name`], and never a sentence about it: a caller - and
    /// `lfx-validator`, which asks for a refusal by name - can then tell which
    /// message arrived rather than reading prose that happens to be in the same
    /// field. One field, one meaning (docs/14 §4).
    #[error("the plugin broker answered {0} out of turn")]
    Unexpected(&'static str),
    /// The broker refused, in its own words.
    #[error("{0}")]
    Refused(String),
    /// Something a stranger sent was past one of the header's own ceilings, or
    /// otherwise not a thing this host will keep.
    #[error(transparent)]
    Rejected(#[from] LfxRejection),
    /// The peer on the pipe could not prove it is the broker this host started,
    /// or the credential could not be minted or handed over at all.
    #[error(transparent)]
    Peer(lumit_peer::PeerError),
    /// More instances than a handle can name.
    #[error("this session has made every instance a handle can name")]
    NoMoreHandles,
    /// More live instances of one bundle than [`MAX_LIVE_INSTANCES`].
    #[error("this bundle already holds {limit} live instances")]
    TooManyInstances {
        /// [`MAX_LIVE_INSTANCES`].
        limit: usize,
    },
    /// The bundle holds no plugin of that id, or the one it holds was refused.
    #[error("this bundle offers no effect called {id:?}")]
    NoSuchPlugin {
        /// What was asked for.
        id: String,
    },
    /// A handle this host never minted, or one whose instance is gone.
    ///
    /// Answered here rather than sent, because the host holds the record for
    /// every live instance and a message about one it has never heard of is a
    /// message with nothing to say. **Every entry point that takes a handle
    /// asks this first** - a press that raced a layer deletion would otherwise
    /// reach the broker, come back a `Failed`, and cost the bundle a strike for
    /// a button the user was entitled to press.
    #[error("no such plugin instance")]
    NoSuchInstance {
        /// The handle that named nothing.
        instance: InstanceId,
    },
    /// The deadline passed with nothing said.
    ///
    /// The sentence is the one a failed frame badges the layer with, which is
    /// why it is here rather than composed at the call site: `Display` is the
    /// badge, and a caller that wants to tell this from a refusal matches the
    /// variant instead of reading the prose (docs/14 §4).
    #[error("the plugin missed its deadline")]
    Timeout,
    /// The broker process went away mid-conversation.
    #[error("the plugin stopped")]
    Gone,
    /// More pictures in one shipment than the ring has slots.
    ///
    /// Counted in **slots** rather than bytes, and declared rather than
    /// discovered: the ring is sized from the window the plugin said it reads,
    /// so a shipment this big means the plugin is asking for more than it
    /// declared (docs/impl/lfx.md §3.4).
    #[error("this frame wants {wanted} pictures and the ring holds {slots}")]
    RingTooSmall {
        /// How many pictures the shipment needs: the input, the output and
        /// every neighbour.
        wanted: usize,
        /// How many the ring has.
        slots: usize,
    },
    /// The broker answered with a frame in a slot that was not the one asked
    /// for.
    ///
    /// The host chooses the output slot and sends it; a broker naming another
    /// one is a broker whose answer would serve the *input* as the render, with
    /// no badge and nothing to see. The ring refuses a header that disagrees
    /// with itself one level down; this is the same question one level up.
    #[error("the plugin broker answered slot {answered} where slot {asked} was asked for")]
    WrongSlot {
        /// The slot the host chose and sent.
        asked: Slot,
        /// The slot the broker named.
        answered: Slot,
    },
    /// The broker answered with a frame that is not the size that was asked
    /// for.
    ///
    /// A hash says only that the bytes are the ones the writer meant, and the
    /// writer is a process holding a stranger's compiled code - so a frame
    /// whose header claims a 1×1 rectangle where a whole picture was asked for
    /// is refused rather than handed on as a `Picture` four elements long.
    #[error(
        "the plugin broker answered a {wide}×{tall} frame of {given} samples where \
         {wanted_wide}×{wanted_tall} was asked for"
    )]
    WrongFrame {
        /// The width the frame's own header claimed.
        wide: u32,
        /// The height it claimed.
        tall: u32,
        /// How many samples came back.
        given: usize,
        /// The width that was asked for.
        wanted_wide: u32,
        /// The height that was asked for.
        wanted_tall: u32,
    },
}

/// One failure of one action: the three things that count as a strike.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Fault {
    /// The deadline passed.
    Timeout,
    /// The process went away.
    Gone,
    /// The plugin answered with a failure, or the broker could not do it.
    Refused(String),
    /// The broker answered something the message does not admit.
    ///
    /// Its own kind rather than a [`Fault::Refused`] with a sentence in it,
    /// because the consequence differs: a refusal is a broker that said no and
    /// is otherwise well, and this is a broker out of step on the pipe, whose
    /// next answer belongs to the question before last.
    OutOfTurn {
        /// What arrived, by [`BrokerMessage::name`].
        answered: &'static str,
    },
}

/// How one bundle should be hosted.
pub struct BrokerConfig {
    /// The `Name.lfx.bundle` directory, whose `Contents/lfx.toml` is the
    /// listing. It crosses in [`HostMessage::Manifest`] and nothing on this
    /// side of the pipe reads it.
    pub bundle: PathBuf,
    /// The payload the broker opens, once a describe asks for it. Which file
    /// that is for this machine's architecture is discovery's answer (§5.1);
    /// what is here is the path it worked out.
    pub module: PathBuf,
    /// The deadlines and workarounds for this bundle.
    pub quirks: Quirks,
    /// Where the broker executable is, if not beside Lumit's own.
    pub exe: Option<PathBuf>,
    /// Extra environment for the child. Lumit sets none of its own; the tests
    /// use it to tell a plugin to misbehave on purpose, which is the only way
    /// to reach a plugin that is not in the test's own process.
    pub env: Vec<(String, String)>,
    /// The switched-off list, read before describe.
    pub disabled: DisableList,
    /// What the ring is first sized for. It is raised, never lowered, by the
    /// widest window the bundle's plugins declare and by the first frame bigger
    /// than a slot.
    pub plan: RingPlan,
    /// Where the ring's backing file goes, if not this machine's own temporary
    /// directory.
    ///
    /// Lumit sets none of its own. The caller that does is the test asking what
    /// a broker does when a ring cannot be made a **second** time: a directory
    /// taken away once the broker is up is the only way to reach that failure
    /// from outside, since the first ring is made before there is a broker to
    /// hold it and every later one is made from a name nobody else knows.
    pub ring_dir: Option<PathBuf>,
}

impl BrokerConfig {
    /// The common case: a bundle, its payload, one frame size and the shipped
    /// defaults.
    #[must_use]
    pub fn new(bundle: impl Into<PathBuf>, module: impl Into<PathBuf>, plan: RingPlan) -> Self {
        Self {
            bundle: bundle.into(),
            module: module.into(),
            quirks: Quirks::default(),
            exe: None,
            env: Vec::new(),
            disabled: nothing_disabled(),
            plan,
            ring_dir: None,
        }
    }
}

/// One picture, at whichever depth the project is in.
///
/// Owned, because it crosses a function boundary on its way into a ring slot
/// and the caller has it already. The depth is the picture's own rather than a
/// field beside it, so a caller cannot hand over halves and call them floats.
#[derive(Clone, Debug, PartialEq)]
pub enum Picture {
    /// Half floats, which is what Lumit's working texture holds.
    F16(Vec<f16>),
    /// Single floats, which is what an fp32 project sends and what the CPU
    /// read-back produces.
    F32(Vec<f32>),
}

impl Picture {
    /// Which depth this picture is.
    #[must_use]
    pub const fn depth(&self) -> PixelDepth {
        match self {
            Picture::F16(_) => PixelDepth::F16,
            Picture::F32(_) => PixelDepth::F32,
        }
    }

    /// How many samples it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Picture::F16(halves) => halves.len(),
            Picture::F32(whole) => whole.len(),
        }
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One frame to render, as the caller asks for it.
pub struct ProcessJob<'a> {
    /// The comp frame being rendered.
    pub time: f64,
    /// Where the input has pixels at all, which is the buffer exactly.
    pub bounds: RectI,
    /// The region of output wanted out of it.
    pub roi: RectI,
    /// The picture to read.
    pub input: &'a Picture,
    /// The frames either side of it the declared window admits, each with its
    /// own comp time. Shipped **with** the request rather than fetched after
    /// it, because version 1 offers no extension for a plugin to ask through.
    pub neighbours: &'a [(f64, Picture)],
}

/// What came back.
pub struct Rendered {
    /// The picture, at the depth it was asked for.
    pub pixels: Picture,
    /// What the instance said it reads, as offsets relative to the frame that
    /// was rendered. The host reads them into the neighbour window the next
    /// frame key is taken over.
    pub frames_needed: Vec<i32>,
}

/// What the host remembers about one instance, which is everything needed to
/// make it again.
#[derive(Clone)]
struct InstanceRecord {
    /// **The descriptor's own id**, never a position in the last `Described`.
    plugin: String,
    /// Every value, in declaration order.
    values: Vec<ParamValue>,
}

/// The live connection to one broker process.
struct Link {
    child: Child,
    sender: SendHalf,
    incoming: Receiver<Incoming>,
}

/// What the reading thread hands back.
enum Incoming {
    /// The broker connected; here is the half to write to.
    Connected(SendHalf),
    /// A message.
    Message(Box<BrokerMessage>),
    /// The pipe closed - which, for a child process, means it died.
    Gone,
}

/// One LFX bundle, hosted in a process of its own.
pub struct Broker {
    config: BrokerConfig,
    ledger: Arc<Ledger>,
    plan: RingPlan,
    ring: Option<Ring>,
    /// The same count as `ring`'s, published so that the pool above can read it
    /// without this broker's lock - which is held across a render (§4.4).
    granted: RingSlots,
    link: Option<Link>,
    manifest: Vec<PluginIdentity>,
    described: Vec<DescribedPlugin>,
    refused: Vec<(String, LfxRejection)>,
    report: Vec<LfxRejection>,
    instances: BTreeMap<InstanceId, InstanceRecord>,
    notes: Vec<(NoteKind, String)>,
    next_index: u32,
    next_slot: Slot,
    /// The endpoint the current broker connected on, kept for the test that
    /// asks whether two of them could ever be the same.
    endpoint: String,
    strikes: u32,
    disabled: bool,
    restarts: usize,
}

impl Broker {
    /// Start a broker for one bundle.
    ///
    /// The ledger is an `Arc` rather than a reference because **an `&Ledger`
    /// cannot reserve**: a `Reservation` owns an `Arc<Ledger>` so that its
    /// `Drop` can give the bytes back, which is the very property the ring
    /// relies on (docs/impl/lfx.md §3.4).
    ///
    /// # Errors
    ///
    /// [`BrokerError`] - the executable, the pipe, the ring, or a broker that
    /// speaks another protocol.
    pub fn spawn(config: BrokerConfig, ledger: &Arc<Ledger>) -> Result<Self, BrokerError> {
        let identifier = next_identifier()?;
        let plan = config.plan;
        let ring = Ring::create(
            &ring_path(config.ring_dir.as_deref(), &identifier),
            plan,
            ledger,
        )?;

        let mut broker = Self {
            config,
            ledger: Arc::clone(ledger),
            plan,
            granted: RingSlots::of(ring.slots()),
            ring: Some(ring),
            link: None,
            manifest: Vec::new(),
            described: Vec::new(),
            refused: Vec::new(),
            report: Vec::new(),
            instances: BTreeMap::new(),
            notes: Vec::new(),
            next_index: 0,
            next_slot: 0,
            endpoint: String::new(),
            strikes: 0,
            disabled: false,
            restarts: 0,
        };
        broker.start(&identifier)?;
        Ok(broker)
    }

    /// Bring a broker process up and hand it the ring.
    ///
    /// The order **is** the security property, and it is `lumit_ipc::rules`'
    /// own: listen before the spawn, so a child cannot connect to a name nobody
    /// is listening on; the broker speaks first and has loaded nothing; the
    /// host answers with its proof, and only then may the broker open a
    /// stranger's code; the host checks the **proof before the version**, so an
    /// impostor is not told which build it faces.
    fn start(&mut self, identifier: &str) -> Result<(), BrokerError> {
        let name = pipe::pipe_name(identifier);
        let listener = pipe::listen(&name)?;
        self.endpoint.clone_from(&name);

        // One secret per broker, per start. A restart after a crash mints a new
        // one, so nothing learned about a dead broker is worth anything against
        // its replacement.
        let secret = lumit_peer::Secret::generate().map_err(BrokerError::Peer)?;

        let exe = self.config.exe.clone().unwrap_or_else(broker_exe);
        let mut command = Command::new(exe);
        command
            .arg(&self.config.module)
            .arg(&name)
            // The secret goes down standard input, never on the command line
            // beside the pipe name: `/proc/<pid>/cmdline` is readable by every
            // process on the machine on Linux, and a command line is in every
            // `ps` listing on all of them.
            .stdin(Stdio::piped())
            // The child's own output is its own: a plugin that prints must not
            // be able to reach the protocol, which is why the protocol is not
            // on standard output in the first place.
            .stdout(Stdio::null());
        lumit_ipc::no_console(&mut command);
        for (key, value) in &self.config.env {
            command.env(key, value);
        }
        let mut child = command.spawn().map_err(BrokerError::Spawn)?;

        // Hand the credential over and close the pipe. Closing matters: the
        // child reads exactly one line and would otherwise wait for an end that
        // never comes if this process died between the two.
        match child.stdin.take() {
            Some(stdin) => secret.hand_over(stdin).map_err(BrokerError::Peer)?,
            None => {
                let _ = child.kill();
                return Err(BrokerError::Peer(lumit_peer::PeerError::Handover(
                    "the broker was spawned without a standard input".into(),
                )));
            }
        }

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || read_loop(listener, &tx));

        let sender = match rx.recv_timeout(HANDSHAKE_TIMEOUT) {
            Ok(Incoming::Connected(sender)) => sender,
            // **The child is ended here rather than dropped.** Every later
            // failure in this function calls `kill`, which reaches the child
            // through `self.link` - and `self.link` is not set until the line
            // below, so on this one arm there is nothing for `Drop for Broker`
            // to kill and `Child::drop` neither kills nor reaps on Unix. A
            // bundle whose broker hangs before connecting would otherwise leave
            // one orphaned process and one thread blocked in `accept` per
            // spawn, and a restart retries.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BrokerError::NoHandshake);
            }
        };
        self.link = Some(Link {
            child,
            sender,
            incoming: rx,
        });

        // Whoever connected says a nonce first. That much anybody can do; it
        // proves nothing and reveals nothing.
        let theirs = match self.wait_for(HANDSHAKE_TIMEOUT) {
            Ok(BrokerMessage::Ready { nonce }) => nonce,
            Ok(other) => {
                self.kill();
                return Err(BrokerError::Unexpected(other.name()));
            }
            Err(_) => {
                self.kill();
                return Err(BrokerError::NoHandshake);
            }
        };

        let ours = lumit_peer::Nonce::generate().map_err(BrokerError::Peer)?;
        self.send(&HostMessage::Challenge {
            nonce: ours,
            proof: lumit_peer::Proof::host(&secret, theirs),
        })?;

        match self.wait_for(HANDSHAKE_TIMEOUT) {
            Ok(BrokerMessage::Hello { version, proof }) => {
                // Who, before what: a peer that cannot prove who it is has no
                // version worth hearing, and answering a mismatch first would
                // tell an impostor which build it is up against.
                if !proof.matches(&lumit_peer::Proof::broker(&secret, ours)) {
                    self.kill();
                    return Err(BrokerError::Peer(lumit_peer::PeerError::NotAuthenticated));
                }
                if version != PROTOCOL_VERSION {
                    self.kill();
                    return Err(BrokerError::ProtocolMismatch {
                        theirs: version,
                        ours: PROTOCOL_VERSION,
                    });
                }
            }
            // The one answer `HostMessage::Challenge` admits is `Hello`, and
            // this is where that list is kept: the match is exhaustive, so a
            // second admissible answer would have to be written here as well as
            // named there.
            Ok(other) => {
                self.kill();
                return Err(BrokerError::Unexpected(other.name()));
            }
            Err(_) => {
                self.kill();
                return Err(BrokerError::NoHandshake);
            }
        }

        let spec = self
            .ring
            .as_ref()
            .ok_or(BrokerError::NoRing)?
            .spec()
            .clone();
        let open = HostMessage::Open { ring: spec };
        self.send(&open)?;
        if let Err(why) = self.ring_is_shared(&open) {
            self.kill();
            return Err(why);
        }
        Ok(())
    }

    // ------------------------------------------------- what the bundle is --

    /// Read the bundle's own listing, without opening its module.
    ///
    /// **This is the answer to failure 6.** A plugin switched off before a scan
    /// has a label, a vendor, a version and a family here, and the module has
    /// not been touched to get them. Read once per broker; a second call
    /// answers what the first one got.
    ///
    /// # Errors
    ///
    /// [`BrokerError`] - the broker could not read it, or answered something
    /// else.
    pub fn manifest(&mut self) -> Result<&[PluginIdentity], BrokerError> {
        if !self.manifest.is_empty() {
            return Ok(&self.manifest);
        }
        let path = self.config.bundle.to_string_lossy().into_owned();
        let control = self.config.quirks.control_timeout;
        match self.action(&HostMessage::Manifest { path }, control) {
            Ok(BrokerMessage::Manifested { entries }) => {
                self.accepted();
                self.manifest = entries;
                Ok(&self.manifest)
            }
            Ok(other) => Err(BrokerError::Unexpected(other.name())),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// Open the module and ask every plugin in it - bar the switched-off ones -
    /// what it is.
    ///
    /// The listing is read first, always, because the re-check below has
    /// nothing to compare against otherwise and because the order is the
    /// property: the cheap answer before the expensive one, and the expensive
    /// one is the first time a stranger's code runs at all.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn describe(&mut self) -> Result<&[DescribedPlugin], BrokerError> {
        self.manifest()?;
        let disabled = self.disabled_now();
        let deadline = describe_deadline(&self.config.quirks);
        let (plugins, refused) = match self.action(&HostMessage::Describe { disabled }, deadline) {
            Ok(BrokerMessage::Described {
                plugins,
                refused,
                report,
            }) => {
                self.accepted();
                self.report = report;
                (plugins, refused)
            }
            Ok(other) => return Err(BrokerError::Unexpected(other.name())),
            Err(fault) => return Err(self.fault_error(&fault)),
        };
        self.keep(plugins, refused);
        self.fit_to_the_declared_windows()?;
        Ok(&self.described)
    }

    /// What the module holds, as last described and as this host will host it.
    #[must_use]
    pub fn described(&self) -> &[DescribedPlugin] {
        &self.described
    }

    /// One plugin the describe turned away, and why - the raw material of §5.3's
    /// `REFUSED` table, which discovery builds.
    #[must_use]
    pub fn refused(&self) -> &[(String, LfxRejection)] {
        &self.refused
    }

    /// Lines about the bundle rather than about one plugin in it.
    #[must_use]
    pub fn report(&self) -> &[LfxRejection] {
        &self.report
    }

    /// What the bundle has said through the log, newest last, capped at
    /// [`MAX_NOTES`].
    #[must_use]
    pub fn notes(&self) -> &[(NoteKind, String)] {
        &self.notes
    }

    /// Keep the plugins that survive both checks, and file the rest.
    ///
    /// Two refusals, in this order. The **manifest re-check** is §4.3's, and
    /// the required-extension list is the field it exists for: negotiation runs
    /// from the manifest, because keeping the module shut is what §3.3 buys, so
    /// a bundle that declared `required = []` and in fact asks for an extension
    /// would otherwise pass negotiation, reach `create`, get a null and fail
    /// somewhere later.
    ///
    /// The **declared window** is §2.4's, and it is here rather than in
    /// [`crate::ipc::proto::DeclaredTraits::temporal_window`] because there is
    /// nowhere to say anything there: that method returns a pair, and the ring
    /// has to be sized from something. **The refusal goes in front of the
    /// narrowing.** A window that does not contain the frame being rendered, or
    /// reaches past the header's own ceiling, is refused by name here, so
    /// nothing downstream is handed a window narrower than the one the plugin
    /// says it reads - which would produce tile seams, and a seam is a
    /// correctness bug where a wasted read is only slow.
    ///
    /// The broker's own refusals come in first and are kept as they arrived. A
    /// plugin that could not be asked what it is at all - one that declined to
    /// describe itself, one needing an extension this host has not got, one
    /// whose sink met a fault that ends the effect - was turned away in the
    /// second process, and its sentence has no other road to §5.3's `REFUSED`
    /// table than this one.
    fn keep(&mut self, plugins: Vec<DescribedPlugin>, refused: Vec<(String, LfxRejection)>) {
        self.described.clear();
        self.refused = refused;
        for plugin in plugins {
            match admit(&self.manifest, &plugin) {
                Ok(()) => self.described.push(plugin),
                Err(why) => self.refused.push((plugin.identity.id, why)),
            }
        }
    }

    // ------------------------------------------------------- the instances --

    /// Make an instance of one of the described plugins, with these values in
    /// its controls.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn create_instance(
        &mut self,
        plugin: &str,
        values: Vec<ParamValue>,
    ) -> Result<InstanceId, BrokerError> {
        if self.instances.len() >= MAX_LIVE_INSTANCES {
            return Err(BrokerError::TooManyInstances {
                limit: MAX_LIVE_INSTANCES,
            });
        }
        if !self
            .described
            .iter()
            .any(|described| described.identity.id == plugin)
        {
            return Err(BrokerError::NoSuchPlugin {
                id: plugin.to_owned(),
            });
        }
        let handle =
            Handle::encode(KIND_INSTANCE, self.next_index).ok_or(BrokerError::NoMoreHandles)?;
        self.next_index = self.next_index.saturating_add(1);
        let instance = handle.bits();
        let record = InstanceRecord {
            plugin: plugin.to_owned(),
            values: values.clone(),
        };
        let message = HostMessage::CreateInstance {
            instance,
            plugin: plugin.to_owned(),
            values,
        };
        let control = self.config.quirks.control_timeout;
        match self.action(&message, control) {
            Ok(BrokerMessage::Created) => {
                self.accepted();
                self.instances.insert(instance, record);
                Ok(instance)
            }
            Ok(other) => Err(BrokerError::Unexpected(other.name())),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// Replace an instance's values.
    ///
    /// The host owns them and the plugin holds none of its own, which is what
    /// makes a restart a replay rather than a recovery - so the record is
    /// updated here whatever the broker says, and a broker that died on this
    /// message is replaced by one that is told the new values.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn set_values(
        &mut self,
        instance: InstanceId,
        values: Vec<ParamValue>,
    ) -> Result<(), BrokerError> {
        if let Some(record) = self.instances.get_mut(&instance) {
            record.values.clone_from(&values);
        } else {
            return Err(BrokerError::NoSuchInstance { instance });
        }
        let control = self.config.quirks.control_timeout;
        match self.action(&HostMessage::Values { instance, values }, control) {
            Ok(BrokerMessage::Done) => {
                self.accepted();
                Ok(())
            }
            Ok(other) => Err(BrokerError::Unexpected(other.name())),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// One of an instance's `ACTION` rows was pressed.
    ///
    /// **The handle is asked about here rather than sent**, as it is for every
    /// other entry point that takes one. A press that raced a layer deletion -
    /// press, destroy, press, press - would otherwise reach a broker that no
    /// longer holds the instance, come back a `Failed` each time, and three
    /// consecutive strikes would put away a bundle that has done nothing wrong.
    /// That is the very outcome answering a press with a `Done` rather than a
    /// `Failed` exists to prevent, arriving through the handle instead of
    /// through the press.
    ///
    /// # Errors
    ///
    /// [`BrokerError::NoSuchInstance`] for a handle this host does not hold,
    /// answered without touching the pipe; otherwise [`BrokerError`].
    pub fn press(&mut self, instance: InstanceId, param: &str) -> Result<(), BrokerError> {
        if !self.instances.contains_key(&instance) {
            return Err(BrokerError::NoSuchInstance { instance });
        }
        let control = self.config.quirks.control_timeout;
        let message = HostMessage::Action {
            instance,
            param: param.to_owned(),
        };
        match self.action(&message, control) {
            Ok(BrokerMessage::Done) => {
                self.accepted();
                Ok(())
            }
            Ok(other) => Err(BrokerError::Unexpected(other.name())),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// Send one message carrying a handle this host never minted, for the one
    /// test that asks what the **broker** does with one.
    ///
    /// §3.5 has two halves and they are answered in two places. The host's half
    /// is [`Self::press`] and the three beside it: a handle this host does not
    /// hold is answered here, never sent. The broker's half is that a forged
    /// handle is answered with a `Failed` at **every** entry point rather than
    /// with "unsupported", which would tell a plugin the feature is missing
    /// when the truth is its handle is rubbish - and once the host's half is
    /// kept, nothing in the shipping path can reach the broker's to prove it.
    /// This is the route that can, and it exists for that test and no other
    /// caller.
    ///
    /// # Errors
    ///
    /// [`BrokerError`] - which for a handle the broker has not got is the
    /// [`BrokerError::Refused`] carrying the broker's own sentence. It counts
    /// as a strike, exactly as any other `Failed` does.
    pub fn ask_with_a_forged_handle_for_test(
        &mut self,
        message: &HostMessage,
    ) -> Result<(), BrokerError> {
        let control = self.config.quirks.control_timeout;
        match self.action(message, control) {
            Ok(BrokerMessage::Done) => {
                self.accepted();
                Ok(())
            }
            Ok(other) => Err(BrokerError::Unexpected(other.name())),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// Render one frame.
    ///
    /// A frame that does not come back - the plugin crashed, hung, refused, or
    /// has been put away - is an `Err`, and the caller renders the input **byte
    /// for byte** and badges the layer with the error's own sentence. It is
    /// never a stopped render.
    ///
    /// **Typed rather than a sentence**, which docs/14 §4 makes a MUST and the
    /// older host's `Broker::render` already obeys: the caller has to tell
    /// "disabled for this session" from "the plugin refused this frame" to pick
    /// a badge, and matching on prose is the coupling `badge_of`'s own string
    /// comparison is recorded as a trap for. `Display` is still the sentence,
    /// so a caller that only wants to badge calls `to_string()`.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn process(
        &mut self,
        instance: InstanceId,
        job: &ProcessJob<'_>,
    ) -> Result<Rendered, BrokerError> {
        if self.disabled {
            return Err(BrokerError::Disabled);
        }
        if !self.instances.contains_key(&instance) {
            return Err(BrokerError::NoSuchInstance { instance });
        }

        let depth = job.input.depth();
        let width = job.bounds.width();
        let height = job.bounds.height();
        // A frame bigger than a slot, or of the deeper depth, needs a wider
        // ring before anything is written into this one.
        let wanted = self
            .plan
            .reading(self.widest_declared_window())
            .max_of(RingPlan::frame(width, height, depth));
        self.fit(wanted)?;

        let slots = self.ring.as_ref().map_or(0, Ring::slots) as usize;
        let shipment = job.neighbours.len().saturating_add(2);
        if shipment > slots {
            return Err(BrokerError::RingTooSmall {
                wanted: shipment,
                slots,
            });
        }

        let input_slot = self.put(job.input, job.bounds)?;
        let mut neighbours = Vec::with_capacity(job.neighbours.len());
        for (time, picture) in job.neighbours {
            let slot = self.put(picture, job.bounds)?;
            neighbours.push(crate::ipc::proto::FrameRef { time: *time, slot });
        }
        let output = self.take_slot();

        let request = ProcessRequest {
            time: job.time,
            depth,
            roi: job.roi,
            dod: job.bounds,
            input: crate::ipc::proto::FrameRef {
                time: job.time,
                slot: input_slot,
            },
            neighbours,
            output,
        };
        let deadline = self.config.quirks.process_timeout;
        match self.action(&HostMessage::Process { instance, request }, deadline) {
            Ok(BrokerMessage::Processed {
                slot,
                frames_needed,
            }) => {
                // **The slot the host chose, not the one the broker named.** A
                // broker holding a misbehaving plugin that answered with the
                // *input* slot would otherwise have the input served as the
                // render, with no badge and nothing to see.
                //
                // **And both refusals strike.** An answer of the right kind
                // carrying the wrong content is a process out of step, not a
                // plugin that said no: the reply admits the question and
                // answers something else about it, which is the same disorder
                // an answer out of turn is and wants the same replacement. It
                // is also what keeps "three *consecutive* strikes" reachable
                // for a broker that misanswers every frame - this is the one
                // path that has read the answer, so this is the path that has
                // to say whether it was a success.
                if slot != output {
                    return Err(self.struck(BrokerError::WrongSlot {
                        asked: output,
                        answered: slot,
                    }));
                }
                let pixels = match self.take_picture(output, depth, job.bounds) {
                    Ok(pixels) => pixels,
                    Err(why) => return Err(self.struck(why)),
                };
                self.accepted();
                Ok(Rendered {
                    pixels,
                    frames_needed,
                })
            }
            Ok(other) => Err(BrokerError::Unexpected(other.name())),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// Destroy an instance and forget it.
    pub fn destroy(&mut self, instance: InstanceId) {
        if self.instances.remove(&instance).is_none() {
            return;
        }
        let control = self.config.quirks.control_timeout;
        if let Ok(BrokerMessage::Done) = self.action(&HostMessage::Destroy { instance }, control) {
            self.accepted();
        }
    }

    /// Whether the bundle has used up its three strikes.
    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// How many consecutive failures stand against it right now.
    #[must_use]
    pub const fn strikes(&self) -> u32 {
        self.strikes
    }

    /// How many times a broker has been started again after one died.
    #[must_use]
    pub const fn restarts(&self) -> usize {
        self.restarts
    }

    /// How many live instances this bundle holds.
    #[must_use]
    pub fn live_instances(&self) -> usize {
        self.instances.len()
    }

    /// The endpoint the broker connected on, for the test that asks whether
    /// two of them could ever be the same and whether either is a process id.
    ///
    /// The ring's name and the endpoint's both come out of `next_identifier`,
    /// so a test that measured only the ring would go on passing if
    /// `pipe::pipe_name` alone were changed - and it is the *endpoint* that
    /// another program on the machine could connect to first.
    #[must_use]
    pub fn endpoint_name_for_test(&self) -> String {
        self.endpoint.clone()
    }

    /// Where the ring's file is, for the tests that ask whether it is still
    /// reachable by name and whether two brokers share one.
    #[must_use]
    pub fn ring_path_for_test(&self) -> String {
        self.ring
            .as_ref()
            .map(|ring| ring.spec().path.clone())
            .unwrap_or_default()
    }

    /// How many slots the ring holds right now.
    #[must_use]
    pub fn ring_slots(&self) -> u32 {
        self.ring.as_ref().map_or(0, Ring::slots)
    }

    /// The same count as a handle that goes on being right.
    ///
    /// [`Self::ring_slots`] is a number read under this broker's lock, and the
    /// ring is **not** a constant of the session: [`Self::fit`] drops it and
    /// makes another the first time a frame arrives that a slot will not hold,
    /// and a bigger frame buys fewer slots. The pool above reads this between
    /// its own frames and may not take this lock to do it - it is held across a
    /// render - so the count is published rather than copied
    /// (docs/impl/lfx.md §4.4).
    #[must_use]
    pub fn granted_slots(&self) -> RingSlots {
        self.granted.clone()
    }

    /// The governor's ledger this bundle was started against.
    ///
    /// Handed on rather than passed a second time, so the pool a definition
    /// grows under reads its pressure off the **same** ledger the ring was
    /// charged to (docs/impl/lfx.md §4.4). Two ledgers here would mean a pool
    /// that grew while the tier the ring is spending was full.
    #[must_use]
    pub const fn ledger(&self) -> &Arc<Ledger> {
        &self.ledger
    }

    // ------------------------------------------------------------ the ring --

    /// The widest window any described plugin declared.
    ///
    /// Read from the plugins that **survived** the describe, so it is a window
    /// somebody has checked: an unhonourable one was refused by [`Self::keep`]
    /// before it reached here.
    fn widest_declared_window(&self) -> (i32, i32) {
        let mut window = (0, 0);
        for plugin in &self.described {
            let (lo, hi) = plugin.traits.temporal_window();
            window = (window.0.min(lo), window.1.max(hi));
        }
        window
    }

    /// Make the ring wide enough for the windows the bundle declared, and say
    /// so when it could not be.
    ///
    /// **This is where a budget-shaped no is told from a ceiling-shaped one.**
    /// `slots_for` returns a number and cannot say which kind of no it gave -
    /// the ring's own `RING_MAX_SLOTS` answers before the ledger is asked
    /// anything, and the ledger answers after - so the two are told apart here,
    /// where there is a report to print them in (docs/impl/lfx.md §3.4).
    ///
    /// **Neither line refuses the effect, and both describe a ceiling that
    /// refuses frames.** The bundle is hosted, every plugin in it keeps the
    /// window it declared, and every frame whose shipment fits the ring
    /// renders; a job carrying more pictures than the ring has slots is
    /// refused whole by [`Self::process`] with [`BrokerError::RingTooSmall`],
    /// because nothing stages a prefetch across more journeys - version 1 has
    /// no frames-request seam to stage one through. So the report line and the
    /// refusal say the same thing to two different readers.
    fn fit_to_the_declared_windows(&mut self) -> Result<(), BrokerError> {
        let (lo, hi) = self.widest_declared_window();
        self.fit(self.plan.reading((lo, hi)))?;
        let lines = self.ring_lines();
        self.report.extend(lines);
        Ok(())
    }

    /// The lines the ring this broker holds right now has earned, read off the
    /// windows the bundle declared and the ring it ended up with.
    ///
    /// **A restart files them again**, which is why this is a method rather
    /// than three numbers worked out where the ring was made. A replacement
    /// broker's describe brings its own report and replaces this one's, and the
    /// replacement ring is made from the same plan on the same pressed machine -
    /// so a page that had been told why a prefetch is refused would stop
    /// being told it after the first crash, while the refusal went on.
    fn ring_lines(&self) -> Vec<LfxRejection> {
        let (lo, hi) = self.widest_declared_window();
        // Every frame the window says it reads, plus the one being written -
        // before any ceiling has had a say, which is what makes the comparison
        // mean something.
        let asked = u32::try_from(i64::from(hi) - i64::from(lo) + 2).unwrap_or(u32::MAX);
        let staged = self.plan.reading((lo, hi)).slots_wanted();
        ring_report(asked, staged, self.ring_slots())
    }

    /// Make room: a wider ring when one is needed, and the broker handed the
    /// new one the way it was handed the first.
    ///
    /// Only ever called between renders, when neither side is reading a slot.
    ///
    /// **The old ring is dropped before the new one is asked for.** The
    /// reservation lives inside the `Ring` that spent it - lumit-budget's own
    /// rule, keep it beside the thing it paid for - so creating the replacement
    /// first would put both rings' bytes to the ledger at once and have it
    /// refuse a regrow it could easily afford.
    ///
    /// **And the question is asked against the plan the ring was made from,
    /// never against the slots it got.** `Ring::create` halves its way down
    /// until the ledger says yes and takes `RING_MIN_SLOTS` over its head when
    /// it never does, so a ring the governor narrowed holds fewer slots than
    /// its own plan asked for - and comparing the wish against the answer finds
    /// that ring too small for the very plan it *is*. On a machine under
    /// pressure every frame would then drop the mapping, make a file, ask the
    /// ledger again for what it has already refused, spend a control round trip
    /// on the broker mapping the replacement, and arrive back at the same three
    /// slots. A ledger's no is an answer, not a question to put again at each
    /// frame; a wider plan is what asks it again.
    fn fit(&mut self, plan: RingPlan) -> Result<(), BrokerError> {
        if self.ring.is_none() {
            // No ring at all means the last attempt to make one failed. Taking
            // it to the watchdog rather than answering `NoRing` for ever is the
            // whole of the difference: a restart makes a fresh one at the plan
            // the last working ring was made from, so a transient costs a frame -
            // and a machine that cannot give this session a ring at all
            // reaches three strikes and the bundle is put away, badging once
            // rather than at every frame until the session ends.
            return Err(self.struck(BrokerError::NoRing));
        }
        let made_for = self.plan;
        if plan.slot_bytes() <= made_for.slot_bytes()
            && plan.slots_wanted() <= made_for.slots_wanted()
        {
            return Ok(());
        }
        let identifier = next_identifier()?;
        drop(self.ring.take());
        // Published before the replacement is asked for, not after: a ring the
        // ledger will not grant leaves this broker with none, and a pool that
        // read the old count would keep a ceiling the bytes behind it are gone.
        self.granted.set(0);
        // **The plan is committed only once a ring has been made from it.**
        // Everything below reads `self.plan` as the plan the ring it holds was
        // made from - the question above is asked against it - so recording a
        // plan no ring exists for would leave that reading describing nothing.
        // A ring that could not be made at all, a full disk or no file handles
        // left, is a fault to recover from rather than a state to sit in: the
        // strike replaces the broker, which builds a fresh ring at the plan
        // that last worked.
        let path = ring_path(self.config.ring_dir.as_deref(), &identifier);
        let made = Ring::create(&path, plan, &self.ledger);
        match made {
            Ok(ring) => {
                self.plan = plan;
                self.ring = Some(ring);
            }
            Err(why) => return Err(self.struck(why.into())),
        }
        self.granted.set(self.ring_slots());
        self.next_slot = 0;
        let spec = self
            .ring
            .as_ref()
            .ok_or(BrokerError::NoRing)?
            .spec()
            .clone();
        let open = HostMessage::Open { ring: spec };
        self.send(&open)?;
        // The replacement ring loses its name once the broker has mapped it,
        // exactly as the first one did - and an answer the `Open` does not
        // admit is two ends out of step here as anywhere else, so it strikes
        // and the replacement broker is handed a ring of its own.
        if let Err(why) = self.ring_is_shared(&open) {
            return Err(self.struck(why));
        }
        Ok(())
    }

    /// Put one picture in the next slot.
    fn put(&mut self, picture: &Picture, bounds: RectI) -> Result<Slot, RingError> {
        let slot = self.take_slot();
        let ring = self.ring.as_mut().ok_or(RingError::Empty)?;
        match picture {
            Picture::F16(halves) => ring.write_f16(slot, halves, bounds, true)?,
            Picture::F32(whole) => ring.write_f32(slot, whole, bounds, true)?,
        };
        Ok(slot)
    }

    /// Read one picture back out of a slot, at the depth and the size it was
    /// asked for.
    ///
    /// The depth is the request's rather than the slot header's on purpose: the
    /// ring refuses a slot written at one depth and read at the other, and that
    /// refusal is what the host never converting a depth to accommodate a
    /// plugin looks like from this side.
    ///
    /// **And the header is checked against the request rather than only against
    /// itself.** The ring's own reader holds a header to its own payload - a
    /// 4K rectangle over sixty-four honestly hashed bytes is refused rather than
    /// handed back as a frame whose bounds walk the caller off the end of it -
    /// and that check cannot see the other half: a well-formed 1×1 frame where
    /// a whole picture was asked for is an honest slot and the wrong answer. So
    /// the frame that comes back is the one the job asked for or it is a
    /// refusal.
    fn take_picture(
        &self,
        slot: Slot,
        depth: PixelDepth,
        bounds: RectI,
    ) -> Result<Picture, BrokerError> {
        let ring = self.ring.as_ref().ok_or(BrokerError::NoRing)?;
        let (header, pixels) = match depth {
            PixelDepth::F16 => ring
                .read_f16(slot)
                .map(|(header, halves)| (header, Picture::F16(halves)))?,
            PixelDepth::F32 => ring
                .read_f32(slot)
                .map(|(header, whole)| (header, Picture::F32(whole)))?,
        };
        let given = u64::try_from(pixels.len()).unwrap_or(u64::MAX);
        if header.bounds != bounds || given != bounds.samples() {
            return Err(BrokerError::WrongFrame {
                wide: header.bounds.width(),
                tall: header.bounds.height(),
                given: pixels.len(),
                wanted_wide: bounds.width(),
                wanted_tall: bounds.height(),
            });
        }
        Ok(pixels)
    }

    /// The next slot, round-robin. A slot is not reused until every other slot
    /// has been, which is what keeps the one being written away from the one
    /// being read.
    fn take_slot(&mut self) -> Slot {
        let slots = self.ring.as_ref().map_or(1, |ring| ring.slots().max(1));
        let slot = self.next_slot;
        self.next_slot = (self.next_slot + 1) % slots;
        slot
    }

    // ------------------------------------------------------------ the wire --

    /// The switched-off list, as it stands right now.
    fn disabled_now(&self) -> BTreeSet<String> {
        self.config
            .disabled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether one plugin is switched off right now.
    #[must_use]
    pub fn is_switched_off(&self, plugin_id: &str) -> bool {
        self.config
            .disabled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(plugin_id)
    }

    /// A failure the caller of a control action sees: the plugin being put away
    /// outranks whatever the last fault was.
    fn fault_error(&self, fault: &Fault) -> BrokerError {
        if self.disabled {
            BrokerError::Disabled
        } else {
            match fault {
                Fault::Timeout => BrokerError::Timeout,
                Fault::Gone => BrokerError::Gone,
                Fault::Refused(why) => BrokerError::Refused(why.clone()),
                Fault::OutOfTurn { answered } => BrokerError::Unexpected(answered),
            }
        }
    }

    /// One action, with its deadline and its consequences: a failure is a
    /// strike, and a strike is either a restart or the end of the plugin.
    ///
    /// **A message with no reply is never waited on**, and that is read off the
    /// message rather than remembered: waiting for an answer to a
    /// [`HostMessage::Frames`] or a [`HostMessage::Shutdown`] would be a
    /// guaranteed deadline, which is a guaranteed strike, which is a plugin
    /// disabled for doing exactly what the protocol says.
    ///
    /// **And the answer is held to the question**, off the same message,
    /// through [`HostMessage::answers`]. A reply the message does not admit is
    /// not a success: the two ends are out of step, the answer this question
    /// was really owed is still on the pipe, and the next message will collect
    /// it. Counting it as one - which is what a supervisor that looked only for
    /// a [`BrokerMessage::Failed`] does - puts the strike count back to nought,
    /// so a broker answering nonsense for ever would never reach three
    /// *consecutive* strikes, never be replaced and never be put away. So it
    /// strikes, and it strikes as a suspect process rather than as a refusal.
    ///
    /// Callers still match the one answer they wanted, because the compiler
    /// asks them to; after this check their fallback arm is a thing that cannot
    /// happen rather than a second copy of this rule.
    ///
    /// **What this does not do is call it a success.** A reply of the right
    /// *kind* can still be the wrong answer - a `Processed` naming a slot the
    /// host did not choose, or carrying a 1×1 rectangle where a whole picture
    /// was asked for - and only the caller that reads the content knows.
    /// Putting the count back to nought here would do it before that reading,
    /// so a broker that misanswered every frame would be counted a success at
    /// every frame and never reach three *consecutive* strikes either. The
    /// reset is [`Self::accepted`], and it is the caller's.
    fn action(
        &mut self,
        message: &HostMessage,
        deadline: Duration,
    ) -> Result<BrokerMessage, Fault> {
        if !message.expects_reply() {
            return Err(Fault::Refused(format!(
                "{} is sent rather than asked",
                message.name()
            )));
        }
        if self.disabled {
            return Err(Fault::Refused(
                "the plugin is disabled for this session".to_owned(),
            ));
        }
        match self.exchange(message, deadline) {
            Ok(BrokerMessage::Failed { action, message }) => {
                // A plugin that answers "no" is not a plugin that has gone
                // wrong: the frame is lost, the process is fine, and the next
                // one may well work. It still counts as a strike, because three
                // refusals in a row is a plugin that cannot do its job.
                self.strike(false);
                Err(Fault::Refused(format!("{action:?}: {message}")))
            }
            Ok(reply) if !message.answers().contains(&reply.name()) => {
                let answered = reply.name();
                self.strike(true);
                Err(Fault::OutOfTurn { answered })
            }
            Ok(reply) => Ok(reply),
            Err(fault) => {
                self.strike(true);
                Err(fault)
            }
        }
    }

    /// The answer was the one the question was owed, and the caller has kept
    /// it: the strike count goes back to nought.
    ///
    /// **A success is an answer that was accepted, not an answer that
    /// arrived.** [`Self::action`] holds a reply to the kinds its message
    /// admits, which is all a message can say about itself; whether a
    /// `Processed` names the slot the host chose and carries the picture the
    /// job asked for is read one level up, and until it has been read there is
    /// nothing to reset for. *consecutive* is the word docs/12 §2.3 uses, and
    /// this is the only place that puts the count back.
    const fn accepted(&mut self) {
        self.strikes = 0;
    }

    /// Count a fault the caller is about to report, and hand back the sentence
    /// it reports with.
    ///
    /// The bundle being put away outranks whatever the last failure was,
    /// exactly as [`Self::fault_error`] has it for a fault that came off the
    /// pipe: a caller that asked for the third time is told the plugin is
    /// disabled rather than told once more what the third answer was wrong
    /// about.
    fn struck(&mut self, why: BrokerError) -> BrokerError {
        self.strike(true);
        if self.disabled {
            BrokerError::Disabled
        } else {
            why
        }
    }

    /// Send, then read until something that is an answer arrives - absorbing
    /// the two messages that are the broker talking mid-action rather than
    /// answering.
    fn exchange(
        &mut self,
        message: &HostMessage,
        deadline: Duration,
    ) -> Result<BrokerMessage, Fault> {
        let expiry = Instant::now() + deadline;
        self.send(message).map_err(|_| Fault::Gone)?;
        loop {
            let left = expiry.saturating_duration_since(Instant::now());
            match self.wait_for(left) {
                Ok(BrokerMessage::NeedFrames { .. }) => {
                    // Version 1 offers no extension table at all, so nothing a
                    // plugin can call asks for a neighbour: every frame the
                    // declared window admits was shipped with the request. A
                    // request all the same is answered with an empty shipment
                    // rather than ignored, because the rule is that every
                    // question gets exactly one answer.
                    self.send(&HostMessage::Frames { frames: Vec::new() })
                        .map_err(|_| Fault::Gone)?;
                }
                Ok(BrokerMessage::Note { kind, text }) => self.note(kind, text),
                Ok(reply) => return Ok(reply),
                Err(fault) => return Err(fault),
            }
        }
    }

    /// Keep one line the plugin said, newest last.
    fn note(&mut self, kind: NoteKind, text: String) {
        if self.notes.len() >= MAX_NOTES {
            self.notes.remove(0);
        }
        self.notes.push((kind, text));
    }

    /// Write one message to the broker.
    fn send(&mut self, message: &HostMessage) -> Result<(), BrokerError> {
        let link = self.link.as_mut().ok_or(BrokerError::NoHandshake)?;
        pipe::send(&mut link.sender, message)?;
        Ok(())
    }

    /// Wait for the broker to say it has the ring mapped, then take the ring's
    /// name out of the directory.
    ///
    /// Notes are absorbed on the way past, the way [`Self::exchange`] absorbs
    /// them: a plugin that logged something while its module was opening would
    /// otherwise put a `Note` where the acknowledgement was expected and leave
    /// the acknowledgement sitting in the queue for whatever asked next.
    ///
    /// Losing the acknowledgement is not fatal: a deadline or a dead pipe here
    /// costs the tidying, not the ring, and the first render says what is
    /// wrong.
    ///
    /// **An answer the `Open` does not admit is fatal**, and it is held to the
    /// message that was sent rather than to a list kept here - the same rule
    /// [`Self::action`] applies to every other question, read off the same
    /// [`HostMessage::answers`]. This is the one exchange outside `action`, so
    /// taking whatever arrived would have a broker that answered an `Open` with
    /// somebody else's reply counted as one that has the ring mapped: the name
    /// would be unlinked on a mapping that never happened, the host would send
    /// frames into a ring nobody is reading, and the answer really owed to the
    /// `Open` would still be on the pipe for the next question to collect.
    ///
    /// # Errors
    ///
    /// [`BrokerError::Unexpected`], naming what arrived.
    fn ring_is_shared(&mut self, open: &HostMessage) -> Result<(), BrokerError> {
        let expiry = Instant::now() + HANDSHAKE_TIMEOUT;
        loop {
            let left = expiry.saturating_duration_since(Instant::now());
            match self.wait_for(left) {
                Ok(BrokerMessage::Note { kind, text }) => self.note(kind, text),
                Ok(BrokerMessage::RingOpened) => {
                    if let Some(ring) = self.ring.as_mut() {
                        ring.unlink_now_it_is_shared();
                    }
                    return Ok(());
                }
                Ok(reply) => {
                    if !open.answers().contains(&reply.name()) {
                        return Err(BrokerError::Unexpected(reply.name()));
                    }
                    // `RingRefused`, the other answer the message admits: the
                    // name stays in the directory for a retry, and the first
                    // render will say what is wrong. What matters is that this
                    // returns now rather than after the whole timeout.
                    return Ok(());
                }
                Err(_) => return Ok(()),
            }
        }
    }

    /// Wait for one message, or for the deadline, or for the process to die.
    ///
    /// **This is the gate on the untrusted direction.** Everything in a
    /// [`BrokerMessage`] was written in a process that is holding a stranger's
    /// compiled code, and the only thing bounding it before here is the
    /// transport's 8 MiB cap on one message. `BrokerMessage::checked` reads the
    /// header's own ceilings, and what does not pass them is a refusal with the
    /// ceiling named rather than a listing somebody then prints.
    fn wait_for(&mut self, left: Duration) -> Result<BrokerMessage, Fault> {
        let Some(link) = self.link.as_ref() else {
            return Err(Fault::Gone);
        };
        let arrived = match link.incoming.recv_timeout(left) {
            Ok(Incoming::Message(message)) => *message,
            Ok(Incoming::Connected(_)) => return Err(Fault::Gone),
            Ok(Incoming::Gone) | Err(RecvTimeoutError::Disconnected) => return Err(Fault::Gone),
            Err(RecvTimeoutError::Timeout) => return Err(Fault::Timeout),
        };
        match arrived.checked() {
            Ok(message) => Ok(message),
            Err(rejection) => {
                let sentence = rejection.to_string();
                self.report.push(rejection);
                Err(Fault::Refused(sentence))
            }
        }
    }

    // ------------------------------------------------------- the watchdog --

    /// Count a failure, and either start again or stop trying.
    fn strike(&mut self, process_is_suspect: bool) {
        self.strikes = self.strikes.saturating_add(1);
        if self.strikes >= STRIKES_BEFORE_DISABLED {
            self.disabled = true;
            self.kill();
            return;
        }
        if process_is_suspect {
            let _ = self.restart();
        }
    }

    /// Start a broker again and put it back where the last one was.
    ///
    /// **A restart is a replay, not a recovery.** The broker keeps nothing
    /// worth keeping - the host owns every parameter value and the plugin holds
    /// no opaque state at all (D8) - so a new broker is told to read the
    /// listing again, describe the module again, and make each instance again
    /// with the values it should have. The frame the old one died in the middle
    /// of does not come back; it comes back *failed*, and the layer renders
    /// identity with a badge.
    ///
    /// Each instance is re-created **by its plugin's own id**. The re-describe
    /// carries whatever disable list the session now has, and a list that has
    /// lost a row renumbers the rest of it: an index of one would name a
    /// different effect while the layer went on holding the first one's values
    /// and the first one's frame key.
    fn restart(&mut self) -> Result<(), BrokerError> {
        self.kill();
        self.restarts = self.restarts.saturating_add(1);
        // A fresh ring, not the old one: the old ring's *name* is gone, because
        // it is unlinked as soon as the broker that died had it mapped, and a
        // replacement has no name to open. Which is the answer this function
        // gives to everything else too.
        let identifier = next_identifier()?;
        drop(self.ring.take());
        self.granted.set(0);
        let path = ring_path(self.config.ring_dir.as_deref(), &identifier);
        self.ring = Some(Ring::create(&path, self.plan, &self.ledger)?);
        self.granted.set(self.ring_slots());
        self.next_slot = 0;
        self.start(&identifier)?;

        let control = self.config.quirks.control_timeout;
        let path = self.config.bundle.to_string_lossy().into_owned();
        if let Ok(BrokerMessage::Manifested { entries }) =
            self.exchange(&HostMessage::Manifest { path }, control)
        {
            self.manifest = entries;
        }
        let disabled = self.disabled_now();
        if let Ok(BrokerMessage::Described {
            plugins,
            refused,
            report,
        }) = self.exchange(
            &HostMessage::Describe { disabled },
            describe_deadline(&self.config.quirks),
        ) {
            self.report = report;
            self.keep(plugins, refused);
            // The replacement broker's report is its own and has just replaced
            // this one's, so the lines the ring earned are filed again against
            // the ring this restart made. It is made from the same plan on the
            // same machine: if the ledger narrowed the last one it narrows this
            // one, and a page that stopped saying so after a crash would leave
            // every later refused shipment unexplained.
            let lines = self.ring_lines();
            self.report.extend(lines);
        }
        let records: Vec<(InstanceId, InstanceRecord)> = self
            .instances
            .iter()
            .map(|(id, record)| (*id, record.clone()))
            .collect();
        for (instance, record) in records {
            let _ = self.exchange(
                &HostMessage::CreateInstance {
                    instance,
                    plugin: record.plugin,
                    values: record.values,
                },
                control,
            );
        }
        Ok(())
    }

    /// End the broker process, however it feels about that.
    fn kill(&mut self) {
        if let Some(mut link) = self.link.take() {
            let _ = pipe::send(&mut link.sender, &HostMessage::Shutdown);
            let _ = link.child.kill();
            let _ = link.child.wait();
        }
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The lines a ring earns, when it is narrower than the declaration asked for.
///
/// Three numbers, in the order the answers are given. `asked` is every frame
/// the widest declared window reads plus the one being written, before anything
/// has had a say. `staged` is what `slots_for` would grant it, which has been
/// through the ring's own `RING_MAX_SLOTS`. `granted` is what the ledger paid
/// for, which has been through the governor.
///
/// **From `slots_for` the two answers read alike**, because it returns a
/// number; the ceiling answers before the ledger is asked anything at all, and
/// telling one from the other is what a report line is for (docs/impl/lfx.md
/// §3.4). Both are lines rather than refusals of the **bundle** - an effect
/// taken away over a scheduling detail is gone - and both describe a ceiling
/// that refuses **frames**: a shipment wider than the ring has slots comes back
/// [`BrokerError::RingTooSmall`] for as long as the narrowing lasts.
fn ring_report(asked: u32, staged: u32, granted: u32) -> Vec<LfxRejection> {
    let mut lines = Vec::new();
    if asked > staged {
        lines.push(LfxRejection::WindowHeldToTheRing {
            wanted: asked,
            slots: staged,
        });
    }
    if granted < staged {
        lines.push(LfxRejection::RingNarrowedByTheLedger {
            wanted: staged,
            granted,
        });
    }
    lines
}

/// Whether one described plugin may be hosted, or why not.
///
/// A free function rather than a method because it is a decision about two
/// records and nothing else, and because the two refusals it makes are the ones
/// this package exists to put in the right order.
///
/// **The listing first.** A plugin the module holds and the listing never
/// mentioned is the same kind of disagreement as a vendor spelled two ways, and
/// gets the same refusal with an empty manifest answer: the listing is what the
/// Addons page would have named it from, and naming an effect the listing does
/// not admit to is the hole the re-check exists to close.
///
/// **Then the window, and this is the ordering §2.4 asks for.**
/// [`crate::ipc::proto::DeclaredTraits::temporal_window`] *narrows* a window to
/// something the ring can be sized from, and narrowing is not refusing: a
/// window that does not contain the frame being rendered would come out of it
/// as `(0, 0)` with nobody told, and an effect handed a narrower window than
/// the one it says it reads produces tile seams. So the refusal by name is
/// taken here, before anything narrows anything.
fn admit(listing: &[PluginIdentity], plugin: &DescribedPlugin) -> Result<(), LfxRejection> {
    let code = &plugin.identity;
    let Some(listed) = listing.iter().find(|entry| entry.id == code.id) else {
        return Err(LfxRejection::ManifestMismatch {
            id: code.id.clone(),
            field: "id",
            manifest: String::new(),
            code: code.id.clone(),
        });
    };
    crate::manifest::agrees(listed, code)?;
    schema::traits_of(Some(&plugin.traits.into()))?;
    Ok(())
}

/// A name no other broker on this machine uses, and none of them can work out
/// in advance.
///
/// Unique is all a name needs to be to keep two brokers apart; unguessable is
/// what it also needs to be when something else could connect to the endpoint
/// instead of the broker, and the programs best placed to guess are the other
/// brokers, each running a third party's compiled code. There is no fallback to
/// a counter: a guessable name is the thing being avoided.
fn next_identifier() -> Result<String, BrokerError> {
    Ok(lumit_peer::Token::generate()
        .map_err(BrokerError::Peer)?
        .as_name())
}

/// Where one ring's backing file goes: the directory the config named, or this
/// machine's own temporary one.
fn ring_path(dir: Option<&Path>, identifier: &str) -> PathBuf {
    let mut path = dir.map_or_else(std::env::temp_dir, Path::to_path_buf);
    path.push(format!("lumit-lfx-{identifier}.ring"));
    path
}

/// The reading thread: accept the one connection, hand the writing half back,
/// then read until the pipe closes.
///
/// It holds no lock and takes none, which is what lets the host wait on a
/// deadline rather than on the plugin (docs/14 §1).
fn read_loop(listener: Listener, tx: &mpsc::Sender<Incoming>) {
    let Ok(stream) = pipe::accept(&listener) else {
        let _ = tx.send(Incoming::Gone);
        return;
    };
    let (mut receiver, sender) = pipe::split(stream);
    if tx.send(Incoming::Connected(sender)).is_err() {
        return;
    }
    loop {
        match pipe::recv::<_, BrokerMessage>(&mut receiver) {
            Ok(message) => {
                if tx.send(Incoming::Message(Box::new(message))).is_err() {
                    return;
                }
            }
            Err(_) => {
                let _ = tx.send(Incoming::Gone);
                return;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// This host's three identity strings are the ones `lumit-ipc` reserved for
    /// it before this crate existed. A rename in one place and not the other
    /// puts two brokers' endpoints in one namespace, or lets one host's
    /// environment variable redirect the other's child.
    #[test]
    fn this_hosts_three_strings_are_the_ones_reserved_for_it() {
        use crate::ipc::identity::HOST_PREFIX;

        assert_eq!(HOST_PREFIX, lumit_ipc::hosts::LFX.prefix);
        assert_eq!(BROKER_EXE_ENV, lumit_ipc::hosts::LFX.broker_exe_env);
        assert_eq!(
            broker_exe_name(),
            if cfg!(windows) {
                "lumit-lfx-broker.exe"
            } else {
                "lumit-lfx-broker"
            },
            "this one string decides which second program the host starts"
        );
        assert!(
            broker_exe_name().starts_with(lumit_ipc::hosts::LFX.broker_exe_stem),
            "and the stem it starts with is the one reserved for this host"
        );
        assert_eq!(
            BROKER_EXE_ENV, "LUMIT_LFX_BROKER",
            "a packaging step depends on this"
        );
    }

    /// The endpoint carries this host's prefix and 128 bits of randomness, and
    /// no other host's prefix.
    #[test]
    fn the_endpoint_name_is_this_hosts_own() {
        let name = pipe::pipe_name("deadbeef");
        let expected = if cfg!(windows) {
            "lumit-lfx-deadbeef.pipe".to_owned()
        } else {
            std::env::temp_dir()
                .join("lumit-lfx-deadbeef.sock")
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(name, expected);
        for other in [lumit_ipc::hosts::OFX, lumit_ipc::hosts::APLUG] {
            assert!(
                !name.contains(other.prefix),
                "the LFX endpoint carries {}'s prefix",
                other.host
            );
        }
    }

    /// The two messages the protocol says go unanswered are never waited on,
    /// and the guard is read off the message rather than remembered. Waiting on
    /// one would be a guaranteed deadline, which is a guaranteed strike, which
    /// is a plugin disabled for obeying the protocol.
    #[test]
    fn a_message_with_no_reply_is_never_waited_on() {
        for message in [
            HostMessage::Frames { frames: Vec::new() },
            HostMessage::Shutdown,
        ] {
            assert!(
                !message.expects_reply(),
                "{} is what this test is about",
                message.name()
            );
        }
    }

    /// One listed plugin, as the manifest declares it.
    fn listed(id: &str) -> PluginIdentity {
        PluginIdentity {
            id: id.into(),
            name: "Blur".into(),
            vendor: "Example".into(),
            major: 1,
            minor: 0,
            patch: 0,
            categories: vec![lumit_lfx_abi::LFX_CATEGORY_BLUR_SHARPEN],
            abi_version: lumit_lfx_abi::LFX_ABI_VERSION,
            required_extensions: Vec::new(),
        }
    }

    /// The same plugin, as its own code answers.
    fn answered(id: &str, traits: crate::ipc::proto::DeclaredTraits) -> DescribedPlugin {
        DescribedPlugin {
            identity: listed(id),
            traits,
            ..DescribedPlugin::default()
        }
    }

    /// The window is **refused before it is narrowed**, which is the whole of
    /// what §2.4 left for this package. A declaration that does not contain the
    /// frame being rendered comes out of `temporal_window` as nothing at all,
    /// and an effect handed a narrower window than the one it says it reads
    /// produces tile seams - a correctness bug, where a wasted read is only
    /// slow. So the test asserts both halves: that the narrowing would have
    /// been silent, and that nothing reaches it.
    #[test]
    fn a_declared_window_the_host_cannot_honour_is_refused_before_it_is_narrowed() {
        let unusable = crate::ipc::proto::DeclaredTraits {
            temporal_lo: 3,
            temporal_hi: 5,
            ..crate::ipc::proto::DeclaredTraits::default()
        };
        assert_eq!(
            unusable.temporal_window(),
            (0, 5),
            "the narrowing turns a window of [3, 5] into one of [0, 5] and says nothing, \
             which is why it may not run first"
        );
        let plugin = answered("com.example.blur", unusable);
        let listing = [listed("com.example.blur")];
        assert_eq!(
            admit(&listing, &plugin),
            Err(LfxRejection::TemporalWindowUnusable { lo: 3, hi: 5 })
        );

        // A window past the header's own ceiling is the other road to the same
        // refusal, and it is refused for the same reason.
        let too_far = crate::ipc::proto::DeclaredTraits {
            temporal_lo: -lumit_lfx_abi::LFX_MAX_TEMPORAL_WINDOW - 1,
            temporal_hi: 0,
            ..crate::ipc::proto::DeclaredTraits::default()
        };
        assert!(admit(&listing, &answered("com.example.blur", too_far)).is_err());

        // And one the host can honour passes, so the refusal is about the
        // window rather than about there being one.
        let honourable = crate::ipc::proto::DeclaredTraits {
            temporal_lo: -1,
            temporal_hi: 1,
            ..crate::ipc::proto::DeclaredTraits::default()
        };
        assert_eq!(
            admit(&listing, &answered("com.example.blur", honourable)),
            Ok(())
        );
    }

    /// An effect the module holds and the listing never mentioned is a
    /// disagreement like any other: the listing is what the Addons page would
    /// have named it from, so an effect it does not admit to is refused rather
    /// than catalogued from one half of a record.
    #[test]
    fn a_plugin_the_listing_never_mentioned_is_a_manifest_mismatch() {
        let listing = [listed("com.example.blur")];
        let stranger = answered(
            "com.example.stranger",
            crate::ipc::proto::DeclaredTraits::default(),
        );
        match admit(&listing, &stranger) {
            Err(LfxRejection::ManifestMismatch { id, field, .. }) => {
                assert_eq!(id, "com.example.stranger");
                assert_eq!(field, "id");
            }
            other => panic!("an unlisted effect must be refused: {other:?}"),
        }
    }

    /// A budget-shaped no and a ceiling-shaped one are two different sentences,
    /// and this is the seam that tells them apart. From `slots_for` they read
    /// alike, because it returns a number; the ring's own ceiling answers before
    /// the ledger is asked anything, so which of the two narrowed a declaration
    /// is a fact only a caller holding all three numbers can state.
    #[test]
    fn a_narrowed_ring_says_which_kind_of_no_it_got() {
        // A window the ring could stage whole, and a ledger that paid for it.
        assert!(ring_report(12, 12, 12).is_empty(), "nothing to report");

        // Wider than `RING_MAX_SLOTS`: the ceiling answered, and the ledger
        // paid for every slot the ceiling left.
        assert_eq!(
            ring_report(130, 64, 64),
            vec![LfxRejection::WindowHeldToTheRing {
                wanted: 130,
                slots: 64
            }]
        );

        // Inside the ceiling and past the ledger: the other sentence, and only
        // the other sentence.
        assert_eq!(
            ring_report(32, 32, 3),
            vec![LfxRejection::RingNarrowedByTheLedger {
                wanted: 32,
                granted: 3
            }]
        );

        // Both, in the order the answers were given.
        let both = ring_report(130, 64, 3);
        assert_eq!(both.len(), 2);
        assert!(matches!(
            both.first(),
            Some(LfxRejection::WindowHeldToTheRing { .. })
        ));
        assert!(matches!(
            both.get(1),
            Some(LfxRejection::RingNarrowedByTheLedger { .. })
        ));

        // And neither takes the effect away.
        for line in ring_report(130, 64, 3) {
            assert!(!line.refuses_the_effect(), "{line} ended the effect");
        }
    }

    /// A ring plan is raised by the declared window and by the frame, never
    /// lowered - so a regrow for a bigger frame does not quietly give back the
    /// slots a temporal plugin was promised.
    #[test]
    fn a_ring_plan_is_raised_by_the_window_and_by_the_frame() {
        let small = RingPlan::frame(64, 64, PixelDepth::F16).reading((-5, 5));
        let big = RingPlan::frame(256, 256, PixelDepth::F32);
        let joined = small.max_of(big);
        assert_eq!(joined.width, 256);
        assert_eq!(joined.height, 256);
        assert_eq!(joined.depth, PixelDepth::F32);
        assert_eq!(joined.window, (-5, 5));
        assert!(
            joined.slots_wanted() >= 12,
            "the window still buys its slots"
        );
    }
}
