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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
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
use crate::ipc::shm::{Ring, ShmError};
use crate::props::PropValue;
use crate::quirks::Quirks;
use crate::render::RenderRequest;

/// How many consecutive failures a plugin gets before it is put away for the
/// session (docs/12 §2.3).
pub const STRIKES_BEFORE_DISABLED: u32 = 3;

/// How long the host waits for a freshly spawned broker to connect and say
/// hello. Separate from the action deadlines: this one is about a program
/// starting, not about a plugin thinking.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

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
    /// The comp's frame size. The ring is sized from this **once**, when the
    /// broker is spawned, and never again.
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

/// A counter, so two brokers in one process never pick the same pipe name.
static PIPE_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Broker {
    /// Start a broker for one bundle, and describe what is in it.
    ///
    /// # Errors
    ///
    /// [`BrokerError`] — the executable, the pipe, the ring, or a broker that
    /// speaks another protocol.
    pub fn spawn(config: BrokerConfig) -> Result<Self, BrokerError> {
        let identifier = format!(
            "{}-{}",
            std::process::id(),
            PIPE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let mut ring_path = std::env::temp_dir();
        ring_path.push(format!("lumit-ofx-{identifier}.ring"));
        let ring = Ring::create(&ring_path, config.frame.0, config.frame.1)?;

        let mut broker = Self {
            config,
            ring,
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

        let exe = self.config.exe.clone().unwrap_or_else(broker_exe);
        let mut command = Command::new(exe);
        command
            .arg(&self.config.bundle)
            .arg(&name)
            // The child's own output is its own: a plugin that prints must not
            // be able to reach the protocol, which is why the protocol is not
            // on standard output in the first place (see `ipc::pipe`).
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        for (key, value) in &self.config.env {
            command.env(key, value);
        }
        let child = command.spawn().map_err(BrokerError::Spawn)?;

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

        match self.wait_for(HANDSHAKE_TIMEOUT) {
            Ok(BrokerMessage::Hello { version }) if version == PROTOCOL_VERSION => {}
            Ok(BrokerMessage::Hello { version }) => {
                self.kill();
                return Err(BrokerError::ProtocolMismatch {
                    theirs: version,
                    ours: PROTOCOL_VERSION,
                });
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
        Ok(())
    }

    /// Ask the bundle what is in it, and remember the answer.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn describe(&mut self) -> Result<&[PluginDescriptor], BrokerError> {
        let control = self.config.quirks.control_timeout;
        match self.action(&HostMessage::Describe, control, None) {
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

    /// One control changed, and the plugin is to be told.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn changed(
        &mut self,
        instance: InstanceId,
        name: &str,
        value: PropValue,
        reason: &str,
        time: f64,
    ) -> Result<(), BrokerError> {
        let record = self
            .instances
            .get_mut(&instance)
            .ok_or(BrokerError::NoSuchInstance)?;
        record.params.set(name, value.clone());
        let control = self.config.quirks.control_timeout;
        let _ = self.action(
            &HostMessage::InstanceChanged {
                instance,
                name: name.to_owned(),
                value,
                reason: reason.to_owned(),
                time,
            },
            control,
            None,
        );
        Ok(())
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

        // Every input, plus one for the answer. The slots are taken before the
        // message goes out, because the message names them.
        let mut inputs = Vec::with_capacity(request.inputs.len());
        for (clip, frame) in &request.inputs {
            let slot = self.take_slot();
            self.ring.write_frame(slot, frame, request.bounds, true)?;
            inputs.push(FrameRef {
                clip: clip.clone(),
                time: request.time,
                slot,
            });
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
            }) => {
                let (_, frame) = self.ring.read_frame(slot)?;
                Ok(BrokerRender {
                    frame,
                    errored: false,
                    error: None,
                    frames_needed,
                    identity_of,
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
        let identifier = format!(
            "{}-{}",
            std::process::id(),
            PIPE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        self.start(&identifier)?;

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

/// The frame a failed render answers with: its own input, and a sentence.
fn errored(frame: Frame16, why: &str) -> BrokerRender {
    BrokerRender {
        frame,
        errored: true,
        error: Some(why.to_owned()),
        frames_needed: BTreeMap::new(),
        identity_of: None,
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
