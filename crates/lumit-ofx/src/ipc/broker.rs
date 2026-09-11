//! The supervisor: spawning a broker, watching it, and outliving it.
//!
//! # In plain terms
//!
//! This is the half of out-of-process hosting that stays in Lumit. It starts a
//! second program, hands it a bundle and a pipe, and from then on talks to the
//! plugin only through that pipe. It never calls the plugin. That is the whole
//! promise of docs/12 §1: a plugin cannot take Lumit down, because a plugin is
//! not in Lumit.
//!
//! Three things it does that are worth reading before changing anything here:
//!
//! **Every action carries a deadline.** The deadline comes from the quirks table
//! ([`crate::quirks`]) — a short one for control actions, a longer one for a
//! render — so a plugin that genuinely takes a minute a frame says so in data
//! rather than in code. Waiting is done on a channel with a timeout, never on a
//! lock, and no lock is held across the wait (docs/14 §1).
//!
//! **Three consecutive failures disable the plugin for the session.** A missed
//! deadline and a dead process are the same kind of event: a strike. One or two
//! strikes cost that frame and buy a restart; the third stops trying, and the
//! effect renders as an errored placeholder from then on. A successful action
//! puts the count back to nought — *consecutive* is the word docs/12 §2.3 uses
//! and it is the word this obeys.
//!
//! **A restart is a replay, not a recovery.** The broker keeps nothing worth
//! keeping: the host owns every parameter, so a new broker is told to describe
//! the bundle again and to make each instance again with the values it should
//! have. That is why parameter ownership is a non-negotiable in docs/12 §1 and
//! not merely a nice arrangement.
//!
//! The frame the plugin died in the middle of does not come back empty: it comes
//! back as its own input, with `errored` set, and the caller puts a calm badge
//! on the layer.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{Listener, SendHalf};
use thiserror::Error;

use crate::describe::{Context, PluginDescriptor};
use crate::image::{Frame16, RectI};
use crate::instance::ParamSnapshot;
use crate::ipc::pipe::{self, PipeError};
use crate::ipc::proto::{
    BrokerMessage, FrameRef, FrameWanted, HostMessage, InstanceId, Slot, PROTOCOL_VERSION,
};
use crate::ipc::shm::{slot_bytes_for, Ring, ShmError};
use crate::quirks::Quirks;
use crate::render::{RenderRequest, SOURCE_CLIP};

/// How many consecutive failures a plugin gets before it is put away for the
/// session (docs/12 §2.3).
pub const STRIKES_BEFORE_DISABLED: u32 = 3;

/// How long the host waits for a freshly spawned broker to connect and say
/// hello. Separate from the action deadlines: this one is about a program
/// starting, not about a plugin thinking.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a press may take. A plugin with its own editor stays inside the
/// press until the user closes the window, so this is hours, not seconds. It
/// exists so a plugin that never comes back is still a plugin that stopped.
pub const PRESS_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// How long a describe may take: the handshake's ceiling, or a quirks-table
/// control deadline set longer than it. The audio broker's twin, for the same
/// reason: the first describe opens the bundle from disk on a process
/// that has only just said hello, which is a program starting rather than a
/// plugin thinking, and nothing on a render waits on it.
pub(crate) fn describe_deadline(quirks: &crate::quirks::Quirks) -> Duration {
    HANDSHAKE_TIMEOUT.max(quirks.control_timeout)
}

/// How many instances of one bundle's plugins may be alive at once.
///
/// A comp with a thousand OFX effects from one vendor's bundle on it is not a
/// comp anybody has built; a runaway that keeps making them is. Each one costs
/// memory in the broker and a message on every restart, so the ceiling bounds
/// both.
pub const MAX_LIVE_INSTANCES: usize = 1_024;

/// How many of the plugin's messages are kept. A plugin in a loop can call the
/// message suite as fast as it likes; the host keeps the most recent few and
/// drops the rest, because an unbounded queue fed by somebody else's code is
/// not a queue, it is a memory leak with a plugin attached (docs/14 §3).
pub const MAX_NOTES: usize = 64;

/// The environment variable that overrides where the broker executable is,
/// for a test or for a developer running from a build tree.
pub const BROKER_EXE_ENV: &str = "LUMIT_OFX_BROKER";

/// The broker executable's file name.
#[must_use]
pub fn broker_exe_name() -> &'static str {
    if cfg!(windows) {
        "lumit-ofx-broker.exe"
    } else {
        "lumit-ofx-broker"
    }
}

/// Where the broker executable is: beside Lumit's own, which is where every
/// packaging step puts it.
#[must_use]
pub fn broker_exe() -> PathBuf {
    if let Some(override_path) = std::env::var_os(BROKER_EXE_ENV) {
        return PathBuf::from(override_path);
    }
    let name = broker_exe_name();
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Start the broker with no console window of its own.
///
/// A broker is a console program and Lumit is a windowed one, so on Windows
/// every spawn opens a console window in front of the editor — one per plugin
/// file, all at once, during the start-up scan. `CREATE_NO_WINDOW` gives the
/// child no console at all instead. Nothing is lost by it: the protocol was
/// never on the child's standard streams (see `ipc::pipe`), and its output is
/// already sent to nowhere.
fn no_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// `CREATE_NO_WINDOW`, from winbase.h.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// What can go wrong before there is a broker to blame.
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
    Ring(#[from] ShmError),
    /// The broker never connected, or never said hello.
    #[error("the plugin broker did not answer when it started")]
    NoHandshake,
    /// The broker speaks another version of the protocol. Refused here, with a
    /// sentence, rather than deserialised into whatever it happens to mean.
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
    /// A message arrived that made no sense where it arrived.
    #[error("the plugin broker answered {0} out of turn")]
    Unexpected(&'static str),
    /// No such instance.
    #[error("no such plugin instance")]
    NoSuchInstance,
    /// The peer on the pipe could not prove it is the broker this host started,
    /// or the credential could not be minted or handed over at all.
    #[error(transparent)]
    Peer(lumit_peer::PeerError),
    /// More instances of one bundle than [`MAX_LIVE_INSTANCES`].
    #[error("this plugin already has {limit} instances, which is as many as Lumit hosts at once")]
    TooManyInstances {
        /// The ceiling.
        limit: usize,
    },
}

/// Where the frames a plugin asks for come from: the evaluation graph, in
/// Lumit; a closure over a fixture, in a test. It answers `None` for a frame
/// there is no picture for, which is a legal answer — OFX plugins ask for
/// frames past the end of a clip all the time.
pub type FrameSource<'a> = dyn Fn(&str, f64) -> Option<Frame16> + 'a;

/// One failure of one action: the three things that count as a strike.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Fault {
    /// The deadline passed.
    Timeout,
    /// The process went away.
    Gone,
    /// The plugin answered with a failure, or the broker could not do it.
    Refused(String),
}

/// How a plugin should be started.
pub struct BrokerConfig {
    /// The `.ofx` binary inside the bundle.
    pub bundle: PathBuf,
    /// The deadlines and workarounds for this bundle.
    pub quirks: Quirks,
    /// The frame size the ring starts at. A bigger frame regrows the ring
    /// when it arrives ([`Broker::fit`]).
    pub frame: (usize, usize),
    /// Where the broker executable is, if not beside Lumit's own.
    pub exe: Option<PathBuf>,
    /// Extra environment for the child. Lumit sets none of its own; the tests
    /// use it to tell a plugin to misbehave on purpose.
    pub env: Vec<(String, String)>,
}

impl BrokerConfig {
    /// The common case: a bundle, the shipped defaults, and a frame size.
    #[must_use]
    pub fn new(bundle: impl Into<PathBuf>, frame: (usize, usize)) -> Self {
        Self {
            bundle: bundle.into(),
            quirks: Quirks::default(),
            frame,
            exe: None,
            env: Vec::new(),
        }
    }
}

/// What the host remembers about one instance, which is everything needed to
/// make it again.
#[derive(Clone)]
struct InstanceRecord {
    plugin: u32,
    context: Context,
    params: ParamSnapshot,
}

/// One render's answer.
pub struct BrokerRender {
    /// The picture. On a failure this is the effect's own input — identity —
    /// so the comp still composites.
    pub frame: Frame16,
    /// Whether this frame is the plugin's work or a placeholder for it. The
    /// caller badges the layer; nothing here is modal (docs/12 §2.3).
    pub errored: bool,
    /// What went wrong, in a sentence, when `errored` is set.
    pub error: Option<String>,
    /// `getFramesNeeded`'s answer, which is what the evaluation graph's
    /// temporal edges are made of (docs/05 §4.2).
    pub frames_needed: BTreeMap<String, (f64, f64)>,
    /// The clip the plugin said this frame simply is, if it said so.
    pub identity_of: Option<String>,
    /// The controls the plugin is hiding after this render, or `None` when
    /// no render happened to ask.
    pub secret: Option<BTreeSet<String>>,
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
    /// The pipe closed — which, for a child process, means it died.
    Gone,
}

/// One plugin bundle, hosted in a process of its own.
pub struct Broker {
    config: BrokerConfig,
    ring: Ring,
    /// The frame size the current ring was built for. `config.frame` is only
    /// the size it *started* at; `fit` grows it, and a restart has to rebuild
    /// the ring at the size the plugin is actually being asked for rather than
    /// at the size the scan opened with.
    ring_frame: (usize, usize),
    link: Option<Link>,
    descriptors: Vec<PluginDescriptor>,
    instances: BTreeMap<InstanceId, InstanceRecord>,
    next_instance: InstanceId,
    next_slot: Slot,
    strikes: u32,
    disabled: bool,
    shipments: usize,
    restarts: usize,
    notes: Vec<(String, String)>,
}

/// A name no other pipe or ring on this machine will have, and none of them can
/// work out in advance.
///
/// This used to be the host's process id and a counter. That is unique, which
/// is all a name needs to be to keep two brokers apart — and it is also
/// something any other program on the machine can compute, which is not all a
/// name needs to be when the endpoint it names is one somebody could connect to
/// instead of the broker. The programs best placed to do the computing are the
/// *other* brokers, each of which is running a third party's compiled code.
///
/// A failure here is a broker that does not start, deliberately: there is no
/// fallback to a counter, because a guessable name is the thing being fixed.
fn fresh_identifier() -> Result<String, BrokerError> {
    Ok(lumit_peer::Token::generate()
        .map_err(BrokerError::Peer)?
        .as_name())
}

/// Where the ring with this name lives.
fn ring_path(identifier: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("lumit-ofx-{identifier}.ring"));
    path
}

impl Broker {
    /// Start a broker for one bundle, and describe what is in it.
    ///
    /// # Errors
    ///
    /// [`BrokerError`] — the executable, the pipe, the ring, or a broker that
    /// speaks another protocol.
    pub fn spawn(config: BrokerConfig) -> Result<Self, BrokerError> {
        let identifier = fresh_identifier()?;
        let ring = Ring::create(&ring_path(&identifier), config.frame.0, config.frame.1)?;

        let ring_frame = config.frame;
        let mut broker = Self {
            config,
            ring,
            ring_frame,
            link: None,
            descriptors: Vec::new(),
            instances: BTreeMap::new(),
            next_instance: 1,
            next_slot: 0,
            strikes: 0,
            disabled: false,
            shipments: 0,
            restarts: 0,
            notes: Vec::new(),
        };
        broker.start(&identifier)?;
        Ok(broker)
    }

    /// Bring a broker process up, hand it the ring, and describe the bundle.
    fn start(&mut self, identifier: &str) -> Result<(), BrokerError> {
        let name = pipe::pipe_name(identifier);
        let listener = pipe::listen(&name)?;

        // One secret per broker, per start. A restart after a crash mints a
        // new one, so nothing learned about a dead broker is worth anything
        // against its replacement.
        let secret = lumit_peer::Secret::generate().map_err(BrokerError::Peer)?;

        let exe = self.config.exe.clone().unwrap_or_else(broker_exe);
        let mut command = Command::new(exe);
        command
            .arg(&self.config.bundle)
            .arg(&name)
            // The secret goes down standard input, never on the command line
            // beside the pipe name: `/proc/<pid>/cmdline` is readable by every
            // process on the machine on Linux, and a command line is in every
            // `ps` listing on all of them. Standard input is the child's own.
            .stdin(Stdio::piped())
            // The child's own output is its own: a plugin that prints must not
            // be able to reach the protocol, which is why the protocol is not
            // on standard output in the first place (see `ipc::pipe`).
            .stdout(Stdio::null());
        no_console(&mut command);
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
            _ => return Err(BrokerError::NoHandshake),
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
            Ok(_) => {
                self.kill();
                return Err(BrokerError::Unexpected("something other than a nonce"));
            }
            Err(_) => {
                self.kill();
                return Err(BrokerError::NoHandshake);
            }
        };

        // The host answers it — which is how a genuine broker knows it is
        // talking to Lumit — and sets its own for the broker to answer.
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
            Ok(_) => {
                self.kill();
                return Err(BrokerError::Unexpected("something other than hello"));
            }
            Err(_) => {
                self.kill();
                return Err(BrokerError::NoHandshake);
            }
        }

        let spec = self.ring.spec().clone();
        self.send(&HostMessage::Open { ring: spec })?;
        // And once the broker has it mapped, the ring's name comes out of the
        // directory: on Unix a mapping outlives the name, so from here the file
        // is reachable only by the two processes holding it and the kernel
        // reclaims it when the last of them goes — a crash included.
        self.ring_is_shared();
        Ok(())
    }

    /// Ask the bundle what is in it, and remember the answer.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn describe(&mut self) -> Result<&[PluginDescriptor], BrokerError> {
        let deadline = describe_deadline(&self.config.quirks);
        match self.action(&HostMessage::Describe, deadline, None) {
            Ok(BrokerMessage::Described { plugins }) => {
                self.descriptors = plugins;
                Ok(&self.descriptors)
            }
            Ok(_) => Err(BrokerError::Unexpected(
                "something other than a description",
            )),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// A failure the caller of a control action sees: the plugin being put away
    /// outranks whatever the last fault was.
    fn fault_error(&self, fault: &Fault) -> BrokerError {
        if self.disabled {
            BrokerError::Disabled
        } else {
            match fault {
                Fault::Timeout => BrokerError::Unexpected("nothing, before the deadline"),
                Fault::Gone => BrokerError::NoHandshake,
                Fault::Refused(_) => BrokerError::Unexpected("a refusal"),
            }
        }
    }

    /// What the bundle holds, as last described.
    #[must_use]
    pub fn descriptors(&self) -> &[PluginDescriptor] {
        &self.descriptors
    }

    /// The plugin's render deadline, as the quirks table set it.
    #[must_use]
    pub fn render_timeout(&self) -> Duration {
        self.config.quirks.render_timeout
    }

    /// Make an instance of one of the plugins.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn create_instance(
        &mut self,
        plugin: u32,
        context: Context,
        params: ParamSnapshot,
    ) -> Result<InstanceId, BrokerError> {
        // A ceiling on how many of one bundle's plugins may be alive at once.
        //
        // Every instance is memory and state inside the broker, and every one
        // of them is rebuilt from scratch on a restart (this crate's replay, in
        // `restart`) — so a project that had accumulated tens of thousands of
        // them would turn every plugin crash into a very long pause. The
        // ceiling is far above a real comp: a thousand instances of one
        // bundle's plugins is a timeline nobody has built.
        if self.instances.len() >= MAX_LIVE_INSTANCES {
            return Err(BrokerError::TooManyInstances {
                limit: MAX_LIVE_INSTANCES,
            });
        }
        let instance = self.next_instance;
        self.next_instance = self.next_instance.saturating_add(1);
        let record = InstanceRecord {
            plugin,
            context,
            params,
        };
        let message = HostMessage::CreateInstance {
            instance,
            plugin: record.plugin,
            context: record.context,
            params: record.params.clone(),
        };
        let control = self.config.quirks.control_timeout;
        match self.action(&message, control, None) {
            Ok(BrokerMessage::Created) => {
                self.instances.insert(instance, record);
                Ok(instance)
            }
            Ok(_) => Err(BrokerError::Unexpected("something other than an instance")),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// Replace an instance's values. The host owns them, so this is a note to
    /// the broker rather than a request.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn set_params(
        &mut self,
        instance: InstanceId,
        params: ParamSnapshot,
    ) -> Result<(), BrokerError> {
        let record = self
            .instances
            .get_mut(&instance)
            .ok_or(BrokerError::NoSuchInstance)?;
        record.params = params.clone();
        let control = self.config.quirks.control_timeout;
        let _ = self.action(
            &HostMessage::ParamSnapshot { instance, params },
            control,
            None,
        );
        Ok(())
    }

    /// Press one of an instance's buttons and wait for the plugin to come back
    /// from it, however long that takes.
    ///
    /// The frame goes across first, since a plugin's own window asks the
    /// Source clip for its preview. The wait is [`PRESS_TIMEOUT`] rather than
    /// the control deadline, since Magic Bullet Looks stays in its editor until
    /// the user closes it. What comes back is every value the plugin holds
    /// afterwards, which is how a look the user built reaches the document.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn press(
        &mut self,
        instance: InstanceId,
        name: &str,
        time: f64,
        source: &Frame16,
    ) -> Result<ParamSnapshot, BrokerError> {
        if !self.instances.contains_key(&instance) {
            return Err(BrokerError::NoSuchInstance);
        }
        let bounds = RectI::sized(
            i32::try_from(source.width()).unwrap_or(0),
            i32::try_from(source.height()).unwrap_or(0),
        );
        self.fit(source.width(), source.height())?;
        let slot = self.take_slot();
        self.ring.write_frame(slot, source, bounds, true)?;
        let message = HostMessage::Press {
            instance,
            name: name.to_owned(),
            time,
            source: FrameRef {
                clip: SOURCE_CLIP.to_owned(),
                time,
                slot,
            },
        };
        allow_foreground(self.link.as_ref().map(|link| link.child.id()));
        match self.action(&message, PRESS_TIMEOUT, None) {
            Ok(BrokerMessage::Pressed { params }) => {
                // The record is what a restart rebuilds the instance from, so
                // it carries the plugin's own writes from here on.
                if let Some(record) = self.instances.get_mut(&instance) {
                    record.params = params.clone();
                }
                Ok(params)
            }
            Ok(_) => Err(BrokerError::Unexpected("something other than a press")),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// Render one frame.
    ///
    /// `source` answers for frames the plugin asks for beyond the ones handed
    /// over — a retimer's `getFramesNeeded` — and every such frame goes across
    /// in **one** shipment.
    ///
    /// A dead or unresponsive plugin is not an error here: the frame comes back
    /// as its own input with `errored` set, because a comp that stops
    /// compositing is worse than a comp with a badge on one layer.
    ///
    /// # Errors
    ///
    /// [`BrokerError::Ring`] if the frames will not fit the ring, which is a
    /// host fault rather than a plugin one.
    pub fn render(
        &mut self,
        instance: InstanceId,
        request: &RenderRequest,
        source: &FrameSource<'_>,
    ) -> Result<BrokerRender, BrokerError> {
        let identity = request
            .inputs
            .values()
            .next()
            .cloned()
            .or_else(|| Frame16::black(request.bounds.width(), request.bounds.height()).ok())
            .ok_or(ShmError::Empty)?;

        if self.disabled {
            return Ok(errored(identity, "the plugin is disabled for this session"));
        }
        self.fit(request.bounds.width(), request.bounds.height())?;

        // Every input, plus one for the answer. The slots are taken before the
        // message goes out, because the message names them.
        let mut inputs = Vec::with_capacity(request.inputs.len() + request.neighbours.len());
        for (clip, frame) in &request.inputs {
            let slot = self.take_slot();
            self.ring.write_frame(slot, frame, request.bounds, true)?;
            inputs.push(FrameRef {
                clip: clip.clone(),
                time: request.time,
                slot,
            });
        }
        // The frames either side go in the same shipment, each under its own
        // time, when the ring has a slot for each: a shipment wider than the
        // ring would write over a frame the broker has not read yet, so it
        // ships the frame in hand alone and the plugin gets that frame for
        // every other time, the same answer a refused prefetch gives.
        let fits = request.inputs.len() + request.neighbours.len() <= self.ring.slots() as usize;
        if fits {
            for (offset, frame) in &request.neighbours {
                let slot = self.take_slot();
                self.ring.write_frame(slot, frame, request.bounds, true)?;
                inputs.push(FrameRef {
                    clip: SOURCE_CLIP.to_owned(),
                    time: request.time + f64::from(*offset),
                    slot,
                });
            }
        }
        let output = self.take_slot();

        let message = HostMessage::Render {
            instance,
            time: request.time,
            bounds: request.bounds,
            order: request.order,
            inputs,
            output,
        };
        let deadline = self.config.quirks.render_timeout;
        match self.action(&message, deadline, Some(source)) {
            Ok(BrokerMessage::Rendered {
                slot,
                frames_needed,
                identity_of,
                secret,
            }) => {
                let (_, frame) = self.ring.read_frame(slot)?;
                Ok(BrokerRender {
                    frame,
                    errored: false,
                    error: None,
                    frames_needed,
                    identity_of,
                    secret: Some(secret),
                })
            }
            Ok(_) => Ok(errored(identity, "the broker answered out of turn")),
            Err(Fault::Timeout) => Ok(errored(identity, "the plugin missed its deadline")),
            Err(Fault::Gone) => Ok(errored(identity, "the plugin stopped")),
            Err(Fault::Refused(why)) => Ok(errored(identity, &why)),
        }
    }

    /// Destroy an instance and forget it.
    ///
    /// # Errors
    ///
    /// [`BrokerError::NoSuchInstance`].
    pub fn destroy(&mut self, instance: InstanceId) -> Result<(), BrokerError> {
        if self.instances.remove(&instance).is_none() {
            return Err(BrokerError::NoSuchInstance);
        }
        let control = self.config.quirks.control_timeout;
        let _ = self.action(&HostMessage::Destroy { instance }, control, None);
        Ok(())
    }

    /// Whether the plugin has used up its three strikes.
    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// How many consecutive failures stand against the plugin right now.
    #[must_use]
    pub const fn strikes(&self) -> u32 {
        self.strikes
    }

    /// How many times a broker has been started again after one died.
    #[must_use]
    pub const fn restarts(&self) -> usize {
        self.restarts
    }

    /// How many prefetch shipments have gone out. One shipment per render is
    /// the whole point of batching (docs/impl/ofx-host.md §4).
    #[must_use]
    pub const fn shipments(&self) -> usize {
        self.shipments
    }

    /// What the plugin has said through the message suite, most recent last,
    /// capped at [`MAX_NOTES`].
    #[must_use]
    pub fn notes(&self) -> &[(String, String)] {
        &self.notes
    }

    /// The same list, taken — what the host does once it has shown them, so a
    /// message is drawn once rather than on every drain.
    #[must_use]
    pub fn take_notes(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.notes)
    }

    /// The path the current ring was made at, for the tests that assert the
    /// name is unguessable and that it is unlinked once both ends hold it.
    ///
    /// Not private, because those tests live in `lumit-ofx-broker` — the
    /// package that owns the binary, which is the only place a test can spawn a
    /// real second process. Not a general accessor either: it is named so that
    /// nothing in the application reaches for it by accident.
    #[must_use]
    pub fn ring_path_for_test(&self) -> String {
        self.ring.spec().path.clone()
    }

    // ------------------------------------------------------------ the wire --

    /// One action, with its deadline and its consequences: a failure is a
    /// strike, and a strike is either a restart or the end of the plugin.
    fn action(
        &mut self,
        message: &HostMessage,
        deadline: Duration,
        source: Option<&FrameSource<'_>>,
    ) -> Result<BrokerMessage, Fault> {
        if self.disabled {
            return Err(Fault::Refused(
                "the plugin is disabled for this session".to_owned(),
            ));
        }
        let outcome = self.exchange(message, deadline, source);
        match outcome {
            Ok(BrokerMessage::Failed { action, message }) => {
                // A plugin that answers "no" is not a plugin that has gone
                // wrong: the frame is lost, the process is fine, and the next
                // one may well work. It still counts as a strike, because three
                // refusals in a row is a plugin that cannot do its job.
                self.strike(false);
                Err(Fault::Refused(format!("{action}: {message}")))
            }
            Ok(reply) => {
                self.strikes = 0;
                Ok(reply)
            }
            Err(fault) => {
                self.strike(true);
                Err(fault)
            }
        }
    }

    /// Send, then read until something that is an answer arrives — shipping
    /// frames and collecting messages on the way, both of which are the broker
    /// talking mid-action rather than answering.
    fn exchange(
        &mut self,
        message: &HostMessage,
        deadline: Duration,
        source: Option<&FrameSource<'_>>,
    ) -> Result<BrokerMessage, Fault> {
        let expiry = Instant::now() + deadline;
        self.send(message).map_err(|_| Fault::Gone)?;
        loop {
            let left = expiry.saturating_duration_since(Instant::now());
            match self.wait_for(left) {
                Ok(BrokerMessage::NeedFrames { frames }) => {
                    self.ship(&frames, source)?;
                }
                Ok(BrokerMessage::Note { kind, text }) => {
                    if self.notes.len() >= MAX_NOTES {
                        self.notes.remove(0);
                    }
                    self.notes.push((kind, text));
                }
                Ok(reply) => return Ok(reply),
                Err(fault) => return Err(fault),
            }
        }
    }

    /// Answer a `NeedFrames` with exactly one `Frames`.
    fn ship(
        &mut self,
        wanted: &[FrameWanted],
        source: Option<&FrameSource<'_>>,
    ) -> Result<(), Fault> {
        let Some(source) = source else {
            return self
                .send(&HostMessage::Frames { frames: Vec::new() })
                .map_err(|_| Fault::Gone);
        };
        if wanted.len() > self.ring.slots() as usize {
            // The ring is sized once per bundle and is not grown mid-render.
            // A prefetch this big is refused, and the plugin gets the frames it
            // was handed, which is the OFX-legal answer to a frame it cannot
            // have.
            return self
                .send(&HostMessage::Frames { frames: Vec::new() })
                .map_err(|_| Fault::Gone);
        }

        let mut frames = Vec::with_capacity(wanted.len());
        for want in wanted {
            let Some(frame) = source(&want.clip, want.time) else {
                continue;
            };
            let bounds = RectI::sized(
                i32::try_from(frame.width()).unwrap_or(0),
                i32::try_from(frame.height()).unwrap_or(0),
            );
            let slot = self.take_slot();
            if self.ring.write_frame(slot, &frame, bounds, true).is_err() {
                continue;
            }
            frames.push(FrameRef {
                clip: want.clip.clone(),
                time: want.time,
                slot,
            });
        }
        self.shipments = self.shipments.saturating_add(1);
        self.send(&HostMessage::Frames { frames })
            .map_err(|_| Fault::Gone)
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
    /// them: the plugin's own code has run by this point (the bundle is loaded
    /// before the broker answers anything) and a plugin that used the message
    /// suite while loading would otherwise put a `Note` where the
    /// acknowledgement was expected — and leave the acknowledgement sitting in
    /// the queue for whatever asked next. Which is exactly what happened.
    ///
    /// Losing the acknowledgement is not fatal: it costs the tidying, not the
    /// ring. The file is then removed by `Drop` as it always was.
    fn ring_is_shared(&mut self) {
        let expiry = Instant::now() + HANDSHAKE_TIMEOUT;
        loop {
            let left = expiry.saturating_duration_since(Instant::now());
            match self.wait_for(left) {
                Ok(BrokerMessage::Note { kind, text }) => {
                    if self.notes.len() >= MAX_NOTES {
                        self.notes.remove(0);
                    }
                    self.notes.push((kind, text));
                }
                Ok(BrokerMessage::RingOpened) => {
                    self.ring.unlink_now_it_is_shared();
                    return;
                }
                _ => return,
            }
        }
    }

    /// Wait for one message, or for the deadline, or for the process to die.
    fn wait_for(&mut self, left: Duration) -> Result<BrokerMessage, Fault> {
        let Some(link) = self.link.as_ref() else {
            return Err(Fault::Gone);
        };
        match link.incoming.recv_timeout(left) {
            Ok(Incoming::Message(message)) => Ok(*message),
            Ok(Incoming::Connected(_)) => Err(Fault::Gone),
            Ok(Incoming::Gone) | Err(RecvTimeoutError::Disconnected) => Err(Fault::Gone),
            Err(RecvTimeoutError::Timeout) => Err(Fault::Timeout),
        }
    }

    /// Make room for a frame of this size. The ring is built for the scan's
    /// 1080p, and the first 4K layer, or a 3840 by 1620 clip in a 1080p comp,
    /// is bigger than a slot. A new ring at the bigger size replaces it, and
    /// the broker is handed the new one the way it was handed the first. Only
    /// ever called between renders, when neither side is reading a slot.
    fn fit(&mut self, width: usize, height: usize) -> Result<(), BrokerError> {
        if slot_bytes_for(width, height) <= self.ring.spec().slot_bytes {
            return Ok(());
        }
        self.ring = Ring::create(&ring_path(&fresh_identifier()?), width, height)?;
        self.ring_frame = (width, height);
        self.next_slot = 0;
        let spec = self.ring.spec().clone();
        self.send(&HostMessage::Open { ring: spec })?;
        // The replacement ring loses its name once the broker has mapped it,
        // exactly as the first one did.
        self.ring_is_shared();
        Ok(())
    }

    /// The next slot, round-robin. A slot is not reused until every other slot
    /// has been, which is what keeps the one being written away from the one
    /// being read.
    fn take_slot(&mut self) -> Slot {
        let slot = self.next_slot;
        self.next_slot = (self.next_slot + 1) % self.ring.slots().max(1);
        slot
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

    /// Start a broker again and put it back where the last one was: describe
    /// the bundle, make every instance, with the values the host holds.
    fn restart(&mut self) -> Result<(), BrokerError> {
        self.kill();
        self.restarts = self.restarts.saturating_add(1);
        // A fresh ring, not the old one.
        //
        // The old ring's *name* is gone: it is unlinked as soon as the broker
        // that died had it mapped, which is what stops a third program opening
        // it and what has the kernel reclaim it when that broker fell over. A
        // replacement broker therefore has no name to open, so it gets a new
        // ring — which is the same answer this function already gives to every
        // other question, since a restart is a replay and the broker keeps
        // nothing worth keeping.
        let (width, height) = self.ring_frame;
        self.ring = Ring::create(&ring_path(&fresh_identifier()?), width, height)?;
        self.next_slot = 0;
        self.start(&fresh_identifier()?)?;

        let control = self.config.quirks.control_timeout;
        if let Ok(BrokerMessage::Described { plugins }) =
            self.exchange(&HostMessage::Describe, control, None)
        {
            self.descriptors = plugins;
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
                    context: record.context,
                    params: record.params,
                },
                control,
                None,
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

/// Let the plugin's window come to the front. Windows only lets the process
/// that has the foreground pass that right on, and the plugin lives in the
/// broker, so without this its editor opens behind Lumit.
#[cfg(windows)]
fn allow_foreground(broker: Option<u32>) {
    #[link(name = "user32")]
    extern "system" {
        fn AllowSetForegroundWindow(process: u32) -> i32;
    }
    if let Some(process) = broker {
        // SAFETY: a plain Win32 call that takes a process id and nothing else.
        unsafe {
            AllowSetForegroundWindow(process);
        }
    }
}

#[cfg(not(windows))]
fn allow_foreground(_broker: Option<u32>) {}

/// The frame a failed render answers with: its own input, and a sentence.
fn errored(frame: Frame16, why: &str) -> BrokerRender {
    BrokerRender {
        frame,
        errored: true,
        error: Some(why.to_owned()),
        frames_needed: BTreeMap::new(),
        identity_of: None,
        secret: None,
    }
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
    let (mut receiver, sender) = stream.split();
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
mod describe_deadline_tests {
    use super::{describe_deadline, HANDSHAKE_TIMEOUT};
    use crate::quirks::Quirks;
    use std::time::Duration;

    /// The shipped two-second control deadline is not what describe waits
    /// under: the first describe opens the module, which is a program starting.
    #[test]
    fn describe_takes_the_handshake_ceiling_by_default() {
        let quirks = Quirks::default();
        assert!(quirks.control_timeout < HANDSHAKE_TIMEOUT);
        assert_eq!(describe_deadline(&quirks), HANDSHAKE_TIMEOUT);
    }

    /// A quirks-table entry that asks for longer than the handshake still gets
    /// it: the table is the mechanism for a plugin that is genuinely slow.
    #[test]
    fn a_longer_control_deadline_from_the_table_still_wins() {
        let quirks = Quirks {
            control_timeout: Duration::from_secs(30),
            ..Quirks::default()
        };
        assert_eq!(describe_deadline(&quirks), Duration::from_secs(30));
    }
}
