//! The supervisor: spawning a broker, watching it, and outliving it.
//!
//! # In plain terms
//!
//! This is the half of out-of-process hosting that stays in Lumit. It starts a
//! second program, hands it a `.clap` file and a pipe, and from then on talks to
//! the plugin only through that pipe. It never calls the plugin. That is the
//! whole promise of docs/12 §1: a plugin cannot take Lumit down, because a
//! plugin is not in Lumit.
//!
//! Four things it does that are worth reading before changing anything here.
//!
//! **One broker per module.** A `.clap` file holding forty effects gets one
//! process, not forty. The cost of that is recorded and deliberate: the
//! three strikes below are struck against the *module*, so a plugin that dies
//! three times takes its file-mates with it. A vendor that ships one crashing
//! effect beside thirty good ones is a quirks-table entry away from its own
//! file, not a redesign.
//!
//! **A block's deadline is the caller's, not the table's.** The OFX host gives a
//! render ten seconds because a frame can honestly take ten seconds. A block of
//! sound cannot: it is eleven milliseconds long, and the only deadline that
//! means anything is *how much lookahead the chain worker has left*
//! (docs/impl/audio-plugins.md §3). The caller passes that margin in; the
//! quirks table only puts a floor under it. Control actions — describe, create,
//! save — keep the two seconds, because those happen when nothing is playing.
//!
//! **Three consecutive failures disable the plugin for the session.** A missed
//! deadline and a dead process are the same kind of event: a strike. One or two
//! strikes cost that block and buy a restart; the third stops trying, and every
//! block from then on comes back failed so the caller ships the sound dry. A
//! successful action puts the count back to nought — *consecutive* is the word
//! docs/12 §2.3 uses and it is the word this obeys.
//!
//! **A restart is a replay, not a recovery.** The broker keeps nothing worth
//! keeping: the host owns every parameter value and the last state blob, so a
//! new broker is told to describe the module again and to make each instance
//! again with what it should have. The block the old one died in the middle of
//! does not come back at all — it comes back *failed*, and the chain worker
//! ships that block dry with a ramp either side of the splice, which is the
//! whole of what a dying plugin costs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{Listener, SendHalf};
use thiserror::Error;

use crate::describe::{PluginDescriptor, Refusal};
use crate::ipc::handles::{Handle, KIND_INSTANCE};
use crate::ipc::pipe::{self, PipeError};
use crate::ipc::proto::{Bring, BrokerMessage, HostMessage, InstanceId, Slot, PROTOCOL_VERSION};
use crate::ipc::ring::{Ring, RingError};
use crate::process::ParamEvent;
use crate::quirks::Quirks;

/// How many consecutive failures a plugin gets before it is put away for the
/// session (docs/12 §2.3).
pub const STRIKES_BEFORE_DISABLED: u32 = 3;

/// How long the host waits for a freshly spawned broker to connect and say
/// hello. Separate from the action deadlines: this one is about a program
/// starting, not about a plugin thinking.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a describe may take: the handshake's ceiling, or a quirks-table
/// control deadline set longer than it.
///
/// Describe is filed with the control actions, but the first one is not a
/// plugin thinking — it is the broker **opening the module from disk** and
/// asking every plugin in it what it is, on a process that has only just said
/// hello. Under the two-second control deadline that is a cold cdylib load
/// plus a virus scan plus whatever else the machine is doing, and on the
/// Windows CI runner it was missed once in seven runs with the test module
/// alone. Nothing on the audio path waits on describe, and a describe that
/// genuinely hangs costs ten seconds instead of two, so it takes the ceiling
/// sized for a program starting.
pub(crate) fn describe_deadline(quirks: &crate::quirks::Quirks) -> Duration {
    HANDSHAKE_TIMEOUT.max(quirks.control_timeout)
}

/// The environment variable that overrides where the broker executable is, for
/// a test or for a developer running from a build tree.
pub const BROKER_EXE_ENV: &str = "LUMIT_APLUG_BROKER";

/// The plugins the user has switched off, shared with whoever edits the list.
///
/// One list, read in two places for one reason each: **before describe**, so a
/// switched-off plugin is never created and its code never runs, and **at the
/// top of every block batch**, so a switch flicked mid-session is honoured on
/// the next batch rather than at the next restart. The owner of the
/// list is whoever reads `lumit_project::PluginPrefs`; this crate only reads
/// what it is handed, which is what keeps the plugin host free of a dependency
/// on the project format.
pub type DisableList = Arc<Mutex<BTreeSet<String>>>;

/// An empty list — nothing switched off.
#[must_use]
pub fn nothing_disabled() -> DisableList {
    Arc::new(Mutex::new(BTreeSet::new()))
}

/// **This session's** switched-off list, which is the one a live instance
/// reads.
///
/// A scan is handed its list by the caller, because a scan is a thing somebody
/// asks for with options in hand. A *block* is not: the chain opens a plugin
/// deep inside a mix, with nobody around to pass preferences down, so the
/// process keeps one list and the composition root writes the user's answer
/// into it ([`set_disabled`]) as it does for the OFX host.
static SESSION_DISABLED: std::sync::OnceLock<DisableList> = std::sync::OnceLock::new();

/// The session's switched-off list, empty until somebody writes to it.
#[must_use]
pub fn session_disabled() -> DisableList {
    Arc::clone(SESSION_DISABLED.get_or_init(nothing_disabled))
}

/// Replace the session's switched-off list with `identifiers` — what the
/// composition root calls once the preferences are read, and again whenever the
/// user flicks a switch. A plugin already open keeps playing until the next
/// batch, which is where the broker reads the list.
pub fn set_disabled(identifiers: &BTreeSet<String>) {
    if let Ok(mut list) = session_disabled().lock() {
        list.clone_from(identifiers);
    }
}

/// Switch one plugin on or off in the session's list.
pub fn set_enabled(identifier: &str, enabled: bool) {
    if let Ok(mut list) = session_disabled().lock() {
        if enabled {
            list.remove(identifier);
        } else {
            list.insert(identifier.to_owned());
        }
    }
}

/// The brokers this session has started, one per `.clap` module.
///
/// Strong handles, so a module's process lives as long as the session once
/// anything in it has played: a chain is opened and dropped every time a mix is
/// rebuilt, and spawning a process per rebuild would be the cost the whole
/// broker design exists to avoid paying per block.
///
/// ponytail: nothing ever evicts one. A user who scans a folder of forty
/// vendors and plays one layer from each ends the session holding forty idle
/// processes; close the least-recently-used when that is a machine somebody
/// actually has, rather than guessing at a ceiling now.
static BROKERS: std::sync::OnceLock<Mutex<BTreeMap<PathBuf, Arc<Mutex<Broker>>>>> =
    std::sync::OnceLock::new();

/// The broker hosting `module`, started and described if this is the first
/// plugin from it.
///
/// # Errors
///
/// [`BrokerError`] — the executable would not start, or the module would not
/// describe. Either way the caller leaves that link out of the chain and the
/// sound goes through dry.
pub fn module_broker(module: &std::path::Path) -> Result<Arc<Mutex<Broker>>, BrokerError> {
    let key = module.to_path_buf();
    let table = BROKERS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut held = table.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(existing) = held.get(&key) {
        return Ok(Arc::clone(existing));
    }
    let mut config = BrokerConfig::new(&key);
    config.disabled = session_disabled();
    let mut broker = Broker::spawn(config)?;
    // Describe before anything asks for an instance: the broker's own
    // descriptor cache is what a restart replays from.
    broker.describe()?;
    let broker = Arc::new(Mutex::new(broker));
    held.insert(key, Arc::clone(&broker));
    Ok(broker)
}

/// The broker executable's file name.
#[must_use]
pub fn broker_exe_name() -> &'static str {
    if cfg!(windows) {
        "lumit-aplug-broker.exe"
    } else {
        "lumit-aplug-broker"
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
    Ring(#[from] RingError),
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
    /// The broker refused, in its own words.
    #[error("{0}")]
    Refused(String),
    /// More instances than a handle can name.
    #[error("this session has made every instance a handle can name")]
    NoMoreHandles,
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
}

impl Fault {
    /// The sentence a failed block carries, which is what badges the layer.
    fn sentence(&self) -> String {
        match self {
            Self::Timeout => "the plugin missed its deadline".to_owned(),
            Self::Gone => "the plugin stopped".to_owned(),
            Self::Refused(why) => why.clone(),
        }
    }
}

/// How a module should be hosted.
pub struct BrokerConfig {
    /// The `.clap` file.
    pub module: PathBuf,
    /// The deadlines and workarounds for it.
    pub quirks: Quirks,
    /// Where the broker executable is, if not beside Lumit's own.
    pub exe: Option<PathBuf>,
    /// Extra environment for the child. Lumit sets none of its own; the tests
    /// use it to tell a plugin to misbehave on purpose, which is the only way
    /// to reach a plugin that is not in the test's own process.
    pub env: Vec<(String, String)>,
    /// The switched-off list, read before describe and per block batch.
    pub disabled: DisableList,
}

impl BrokerConfig {
    /// The common case: a module and the shipped defaults.
    #[must_use]
    pub fn new(module: impl Into<PathBuf>) -> Self {
        Self {
            module: module.into(),
            quirks: Quirks::default(),
            exe: None,
            env: Vec::new(),
            disabled: nothing_disabled(),
        }
    }
}

/// What the host remembers about one instance, which is everything needed to
/// make it again.
#[derive(Clone)]
struct InstanceRecord {
    bring: Bring,
}

/// One instance, as it came up.
pub struct Created {
    /// The handle to name it by from now on.
    pub instance: InstanceId,
    /// What it reports now that it is active.
    pub latency: u32,
    /// What went wrong bringing it up that did not stop it coming up.
    pub warning: Option<String>,
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

/// One `.clap` module, hosted in a process of its own.
pub struct Broker {
    config: BrokerConfig,
    ring: Ring,
    link: Option<Link>,
    descriptors: Vec<PluginDescriptor>,
    rejected: Vec<Refusal>,
    instances: BTreeMap<InstanceId, InstanceRecord>,
    next_index: u32,
    next_slot: Slot,
    strikes: u32,
    disabled: bool,
    restarts: usize,
}

/// A counter, so two brokers in one process never pick the same pipe name.
static PIPE_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Broker {
    /// Start a broker for one module.
    ///
    /// # Errors
    ///
    /// [`BrokerError`] — the executable, the pipe, the ring, or a broker that
    /// speaks another protocol.
    pub fn spawn(config: BrokerConfig) -> Result<Self, BrokerError> {
        let identifier = next_identifier();
        let mut ring_path = std::env::temp_dir();
        ring_path.push(format!("lumit-aplug-{identifier}.ring"));
        let ring = Ring::create(&ring_path)?;

        let mut broker = Self {
            config,
            ring,
            link: None,
            descriptors: Vec::new(),
            rejected: Vec::new(),
            instances: BTreeMap::new(),
            next_index: 0,
            next_slot: 0,
            strikes: 0,
            disabled: false,
            restarts: 0,
        };
        broker.start(&identifier)?;
        Ok(broker)
    }

    /// Bring a broker process up and hand it the ring.
    fn start(&mut self, identifier: &str) -> Result<(), BrokerError> {
        let name = pipe::pipe_name(identifier);
        let listener = pipe::listen(&name)?;

        let exe = self.config.exe.clone().unwrap_or_else(broker_exe);
        let mut command = Command::new(exe);
        command
            .arg(&self.config.module)
            .arg(&name)
            // The child's own output is its own: a plugin that prints must not
            // be able to reach the protocol, which is why the protocol is not
            // on standard output in the first place (see `ipc::pipe`).
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        no_console(&mut command);
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

    /// The switched-off list, as it stands right now.
    fn disabled_now(&self) -> BTreeSet<String> {
        self.config
            .disabled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether one plugin is switched off right now. Read at the top of a block
    /// batch, never per block.
    #[must_use]
    pub fn is_switched_off(&self, plugin_id: &str) -> bool {
        self.config
            .disabled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(plugin_id)
    }

    /// Ask the module what is in it, and remember the answer.
    ///
    /// The module is opened **inside the broker**: a `clap_entry.init` is
    /// already third-party code running, so it runs where a crash costs a
    /// process rather than a session (docs/impl/audio-plugins.md §5).
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn describe(&mut self) -> Result<&[PluginDescriptor], BrokerError> {
        let disabled = self.disabled_now().into_iter().collect();
        let deadline = describe_deadline(&self.config.quirks);
        match self.action(&HostMessage::Describe { disabled }, deadline) {
            Ok(BrokerMessage::Described { plugins, rejected }) => {
                self.descriptors = plugins;
                self.rejected = rejected;
                Ok(&self.descriptors)
            }
            Ok(_) => Err(BrokerError::Unexpected(
                "something other than a description",
            )),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// What the module holds, as last described.
    #[must_use]
    pub fn descriptors(&self) -> &[PluginDescriptor] {
        &self.descriptors
    }

    /// One calm line per plugin in the module that cannot be hosted.
    #[must_use]
    pub fn rejected(&self) -> &[Refusal] {
        &self.rejected
    }

    /// Make an instance of one of the plugins.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn create_instance(&mut self, bring: Bring) -> Result<Created, BrokerError> {
        let handle =
            Handle::encode(KIND_INSTANCE, self.next_index).ok_or(BrokerError::NoMoreHandles)?;
        self.next_index = self.next_index.saturating_add(1);
        let instance = handle.bits();
        let record = InstanceRecord {
            bring: bring.clone(),
        };
        let message = HostMessage::CreateInstance { instance, bring };
        let control = self.config.quirks.control_timeout;
        match self.action(&message, control) {
            Ok(BrokerMessage::Created { latency, warning }) => {
                self.instances.insert(instance, record);
                Ok(Created {
                    instance,
                    latency,
                    warning,
                })
            }
            Ok(_) => Err(BrokerError::Unexpected("something other than an instance")),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// One block of sound, in and out.
    ///
    /// `margin` is how much lookahead the caller has left; the deadline is that,
    /// floored at one block period by the quirks table. A block that does not
    /// come back — the plugin crashed, hung, refused, or has been put away — is
    /// an `Err` carrying the sentence, and the caller ships the block **dry**.
    /// It is never a stopped mix.
    ///
    /// # Errors
    ///
    /// The sentence to badge the layer with.
    pub fn process(
        &mut self,
        instance: InstanceId,
        input: &[f32],
        output: &mut [f32],
        events: &[ParamEvent],
        steady: i64,
        margin: Duration,
    ) -> Result<(), String> {
        if self.disabled {
            return Err("the plugin is disabled for this session".to_owned());
        }
        if !self.instances.contains_key(&instance) {
            return Err("no such plugin instance".to_owned());
        }

        let in_slot = self.take_slot();
        let out_slot = self.take_slot();
        if let Err(error) = self.ring.write_block(in_slot, input) {
            return Err(error.to_string());
        }

        let message = HostMessage::Process {
            instance,
            input: in_slot,
            output: out_slot,
            events: events.to_vec(),
            steady,
        };
        let deadline = self.config.quirks.block_deadline(margin);
        match self.action(&message, deadline) {
            Ok(BrokerMessage::Processed { slot }) => match self.ring.read_block(slot, output) {
                Ok(_) => Ok(()),
                Err(error) => Err(error.to_string()),
            },
            Ok(_) => Err("the broker answered out of turn".to_owned()),
            Err(fault) => Err(fault.sentence()),
        }
    }

    /// The blob to write into the `.lum`, and the one a restart replays.
    ///
    /// # Errors
    ///
    /// [`BrokerError`].
    pub fn save(&mut self, instance: InstanceId) -> Result<Vec<u8>, BrokerError> {
        let control = self.config.quirks.control_timeout;
        match self.action(&HostMessage::Save { instance }, control) {
            Ok(BrokerMessage::Saved { bytes }) => {
                // The record keeps the newest blob, so a restart replays the
                // plugin's memory of itself as of the last save rather than as
                // of the day the project was opened.
                if let Some(record) = self.instances.get_mut(&instance) {
                    record.bring.state = Some(bytes.clone());
                }
                Ok(bytes)
            }
            Ok(_) => Err(BrokerError::Unexpected("something other than a state blob")),
            Err(fault) => Err(self.fault_error(&fault)),
        }
    }

    /// Destroy an instance and forget it.
    pub fn destroy(&mut self, instance: InstanceId) {
        if self.instances.remove(&instance).is_none() {
            return;
        }
        let control = self.config.quirks.control_timeout;
        let _ = self.action(&HostMessage::Destroy { instance }, control);
    }

    /// Whether the module has used up its three strikes.
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

    // ------------------------------------------------------------ the wire --

    /// A failure the caller of a control action sees: the plugin being put away
    /// outranks whatever the last fault was.
    fn fault_error(&self, fault: &Fault) -> BrokerError {
        if self.disabled {
            BrokerError::Disabled
        } else {
            match fault {
                Fault::Timeout => BrokerError::Unexpected("nothing, before the deadline"),
                Fault::Gone => BrokerError::NoHandshake,
                Fault::Refused(why) => BrokerError::Refused(why.clone()),
            }
        }
    }

    /// One action, with its deadline and its consequences: a failure is a
    /// strike, and a strike is either a restart or the end of the plugin.
    fn action(
        &mut self,
        message: &HostMessage,
        deadline: Duration,
    ) -> Result<BrokerMessage, Fault> {
        if self.disabled {
            return Err(Fault::Refused(
                "the plugin is disabled for this session".to_owned(),
            ));
        }
        match self.exchange(message, deadline) {
            Ok(BrokerMessage::Failed { action, message }) => {
                // A plugin that answers "no" is not a plugin that has gone
                // wrong: the block is lost, the process is fine, and the next
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

    /// Send, then wait for the answer or the deadline.
    fn exchange(
        &mut self,
        message: &HostMessage,
        deadline: Duration,
    ) -> Result<BrokerMessage, Fault> {
        let expiry = Instant::now() + deadline;
        self.send(message).map_err(|_| Fault::Gone)?;
        let left = expiry.saturating_duration_since(Instant::now());
        self.wait_for(left)
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

    /// Start a broker again and put it back where the last one was: describe the
    /// module, make every instance, with the values and the blob the host holds.
    fn restart(&mut self) -> Result<(), BrokerError> {
        self.kill();
        self.restarts = self.restarts.saturating_add(1);
        self.start(&next_identifier())?;

        let control = self.config.quirks.control_timeout;
        let disabled = self.disabled_now().into_iter().collect();
        if let Ok(BrokerMessage::Described { plugins, rejected }) =
            self.exchange(&HostMessage::Describe { disabled }, control)
        {
            self.descriptors = plugins;
            self.rejected = rejected;
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
                    bring: record.bring,
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

/// A name no other broker in this process, or in another copy of Lumit, uses.
fn next_identifier() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        PIPE_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
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
