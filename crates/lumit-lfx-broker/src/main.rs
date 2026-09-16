//! `lumit-lfx-broker` - the program an LFX plugin lives in.
//!
//! # In plain terms
//!
//! This is a very small program whose whole job is to be expendable. Lumit
//! starts one of these per bundle, hands it the bundle's payload and a pipe,
//! and never loads the plugin itself. Everything the plugin does - being listed,
//! describing itself, rendering - happens here, and when it crashes one frame is
//! lost and Lumit starts another one. That is the entire design (docs/12 §1,
//! docs/impl/lfx.md §3).
//!
//! It reads two arguments - the `.lfx` payload inside the bundle, and the pipe
//! name - and speaks the protocol in `lumit_lfx::ipc::proto`. Its first word is
//! a nonce, and it opens nothing until the host has proved who it is.
//!
//! **Two orderings are the whole of what this program is for**, and both are
//! stricter than the OFX broker's.
//!
//! The **manifest is read before any of the plugin's code**. A
//! [`HostMessage::Manifest`] is answered out of `Contents/lfx.toml` with the
//! module shut, so Lumit can name, label and re-enable a plugin whose code has
//! never run. Parsing that file is the reason this program exists at all: it is
//! a stranger's structured text, and reading it in the process that holds the
//! project, the media handles and the windows is the one thing the broker
//! architecture exists to prevent (§11 item 7).
//!
//! And the **module is opened lazily, on the first describe**, which carries
//! the list of plugins the user has switched off. A switched-off plugin's
//! `init` never runs - not here and not anywhere (§5.4).
//!
//! Nothing here is clever, and nothing here recovers. Recovery is Lumit's job,
//! and it does it by starting this program again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;

use half::f16;
use lumit_lfx::describe::PluginDescriptor;
use lumit_lfx::ipc::handles::{Handle, Registry, KIND_INSTANCE};
use lumit_lfx::ipc::pipe::{self, RecvHalf, SendHalf};
use lumit_lfx::ipc::proto::{
    BrokerMessage, DescribedPlugin, HostAction, HostMessage, InstanceId, NoteKind, ParamValue,
    PixelDepth, ProcessRequest, RectI, Slot, PROTOCOL_VERSION,
};
use lumit_lfx::ipc::ring::Ring;
use lumit_lfx::local::{LocalHost, LocalInstance, Pixels, PixelsMut, Request, Value};
use lumit_lfx::{manifest, LfxRejection};

/// The protocol version this broker announces. Normally [`PROTOCOL_VERSION`];
/// an environment variable overrides it so that a test can put a host and a
/// broker that disagree in the same room and watch the host refuse **after** the
/// proof rather than deserialise.
const PROTOCOL_ENV: &str = "LUMIT_LFX_BROKER_PROTOCOL";

/// How this broker should answer wrongly, so that a test can watch the host
/// refuse rather than believe it.
///
/// `"slot"` names the input slot in the answer instead of the output one, which
/// would have the host serve the **input** as the render with no badge and
/// nothing to see. `"size"` writes a one-pixel frame into the output slot,
/// which would hand the caller a four-element picture where a whole one was
/// asked for. `"reply"` answers the frame with a message that belongs to
/// another question altogether, and `"open"` answers the `Open` that hands the
/// ring over the same way: the two ends falling out of step, which is the one
/// failure a host that looked only for a `Failed` would count as a success. The
/// `Open` is the one exchange the host runs outside its own supervisor, so it
/// is the one place such an answer could be taken for an acknowledgement - the
/// ring's name unlinked on a mapping that never happened, and frames sent into
/// a ring nobody is reading. All four are what a broker holding a misbehaving
/// plugin could do and none is anything an honest one does; the environment is
/// the only way to reach a second process, as it is for the protocol above and
/// for the three dangerous personalities in the fixture.
const MISANSWER_ENV: &str = "LUMIT_LFX_BROKER_MISANSWER";

/// How this broker answers, read once at start-up rather than per frame: the
/// shipping answer is the first arm and nothing in the render path should be
/// looking at the environment to find that out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Misanswer {
    /// The honest answer: the output slot, at the size that was asked for.
    No,
    /// Name the input slot instead.
    Slot,
    /// Write one pixel where a picture was asked for.
    Size,
    /// Answer the frame with a reply to another question entirely.
    Reply,
    /// Answer the `Open` with a reply to another question entirely.
    Open,
}

impl Misanswer {
    /// What [`MISANSWER_ENV`] says, or [`Misanswer::No`] where it says nothing
    /// this build knows.
    fn from_env() -> Self {
        match std::env::var(MISANSWER_ENV).unwrap_or_default().as_str() {
            "slot" => Misanswer::Slot,
            "size" => Misanswer::Size,
            "reply" => Misanswer::Reply,
            "open" => Misanswer::Open,
            _ => Misanswer::No,
        }
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(module), Some(pipe_name)) = (args.next(), args.next()) else {
        eprintln!("usage: lumit-lfx-broker <payload.lfx> <pipe name>");
        return ExitCode::FAILURE;
    };
    let Some(pipe_name) = pipe_name.to_str().map(str::to_owned) else {
        eprintln!("the pipe name is not text");
        return ExitCode::FAILURE;
    };

    match run(Path::new(&module), &pipe_name) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("lumit-lfx-broker: {why}");
            ExitCode::FAILURE
        }
    }
}

/// Everything, once the arguments are known.
fn run(module: &Path, pipe_name: &str) -> Result<(), String> {
    // The credential, first thing, before the pipe and long before the plugin.
    // It arrives on standard input rather than on the command line, which every
    // process on the machine can read.
    let secret = lumit_peer::Secret::from_stdin().map_err(|error| error.to_string())?;

    let stream = pipe::connect(pipe_name).map_err(|error| error.to_string())?;
    let (mut receiver, mut sender) = pipe::split(stream);

    // A nonce, and nothing else. Whoever is listening at that name gets this
    // much; it proves nothing and gives nothing away.
    let ours = lumit_peer::Nonce::generate().map_err(|error| error.to_string())?;
    say(&mut sender, &BrokerMessage::Ready { nonce: ours })?;

    // The host's answer to it. **This is the gate the plugin sits behind**: an
    // impostor at that endpoint cannot produce this, and neither the manifest
    // nor the module is touched until it has.
    let HostMessage::Challenge { nonce, proof } =
        pipe::recv::<_, HostMessage>(&mut receiver).map_err(|error| error.to_string())?
    else {
        return Err("the host said something other than a challenge".into());
    };
    if !proof.matches(&lumit_peer::Proof::host(&secret, ours)) {
        return Err("the host on this pipe could not prove who it is".into());
    }

    let version = std::env::var(PROTOCOL_ENV)
        .ok()
        .and_then(|text| text.parse::<u32>().ok())
        .unwrap_or(PROTOCOL_VERSION);
    say(
        &mut sender,
        &BrokerMessage::Hello {
            version,
            proof: lumit_peer::Proof::broker(&secret, nonce),
        },
    )?;

    // Nothing is open. The module waits for the first describe, and the
    // `OnceLock` is what makes "opened lazily, and opened once" a fact about
    // the type: it hands out a borrow that lives as long as this function, so
    // the instances below can hold one while the map they live in is edited.
    //
    // The two locals are declared in this order deliberately. Rust drops them
    // in reverse, so the session - and every instance in it - is taken down
    // before the module it borrows, which is what the header asks for: destroy
    // every instance, then `deinit`, then unload.
    let module_cell: OnceLock<LocalHost> = OnceLock::new();
    let mut session = Session {
        module_path: module.to_path_buf(),
        module: &module_cell,
        receiver,
        sender,
        ring: None,
        described: BTreeMap::new(),
        instances: Registry::new(KIND_INSTANCE),
        notes_sent: 0,
        misanswer: Misanswer::from_env(),
    };
    session.serve()
}

/// Write one message.
fn say(sender: &mut SendHalf, message: &BrokerMessage) -> Result<(), String> {
    pipe::send(sender, message).map_err(|error| error.to_string())
}

/// One live effect, and what the host says its controls are set to.
struct Live<'module> {
    /// Which plugin, by the id its descriptor declared.
    plugin: String,
    /// The instance itself. `process` takes `&mut self`, so one instance cannot
    /// be re-entered by construction.
    instance: LocalInstance<'module>,
    /// Every value, in declaration order. The host owns them and sends them;
    /// the plugin holds none of its own (D8), which is what makes a restart a
    /// replay.
    values: Vec<Value>,
    /// The window the plugin declared, which is what `frames_needed` answers
    /// until there is an extension for an instance to answer through.
    window: (i32, i32),
}

/// The broker's whole state. One module, one pipe, one ring.
struct Session<'module> {
    module_path: PathBuf,
    module: &'module OnceLock<LocalHost>,
    receiver: RecvHalf,
    sender: SendHalf,
    ring: Option<Ring>,
    /// Every plugin that described itself, by its own id. `CreateInstance`
    /// names one of these; nothing anywhere names a position in a list.
    described: BTreeMap<String, PluginDescriptor>,
    instances: Registry<Live<'module>>,
    /// How many of the module's log lines have been forwarded. The module's own
    /// list is capped and append-only, so this is where the next one starts.
    notes_sent: usize,
    /// Whether this broker has been told to answer a frame wrongly on purpose,
    /// which only a test ever does.
    misanswer: Misanswer,
}

impl<'module> Session<'module> {
    /// Read messages until the host stops asking or the pipe closes.
    fn serve(&mut self) -> Result<(), String> {
        loop {
            let message: HostMessage = match pipe::recv(&mut self.receiver) {
                Ok(message) => message,
                // The host closing the pipe is the ordinary way this program
                // ends. It is not a failure and it is not reported as one.
                Err(_) => return Ok(()),
            };
            match message {
                HostMessage::Shutdown => return Ok(()),
                other => self.handle(other)?,
            }
        }
    }

    /// One message.
    fn handle(&mut self, message: HostMessage) -> Result<(), String> {
        match message {
            // The handshake happened before anything was opened, in `run`. A
            // second challenge, once the conversation is under way, is not a
            // host that wants re-authenticating - nothing in the protocol asks
            // for one - so it is refused rather than answered.
            HostMessage::Challenge { .. } => {
                Err("the host challenged twice on one connection".into())
            }
            HostMessage::Open { ring } => {
                self.ring = Ring::open(&ring).ok();
                // Say so, so the host can take the ring's name out of the
                // directory: on Unix the mapping outlives the name, and from
                // that point no third program can open it and the kernel
                // reclaims it when the last of us exits. And say so when it is
                // not, for the same host: silence here would cost it the whole
                // handshake timeout on every spawn, regrow and restart of a
                // session that could not render anyway.
                //
                // A reply to another question altogether, when a test asks for
                // it: the ring really is mapped, and what is wrong is the
                // sentence about it. This is the one exchange the host runs
                // outside its own supervisor, so it is the one place an answer
                // out of turn could be taken for an acknowledgement.
                let answer = if self.misanswer == Misanswer::Open {
                    BrokerMessage::Created
                } else if self.ring.is_some() {
                    BrokerMessage::RingOpened
                } else {
                    BrokerMessage::RingRefused
                };
                self.reply(&answer)
            }
            HostMessage::Manifest { path } => self.manifest(&path),
            HostMessage::Describe { disabled } => self.describe(&disabled),
            HostMessage::CreateInstance {
                instance,
                plugin,
                values,
            } => self.create(instance, &plugin, &values),
            HostMessage::Values { instance, values } => {
                if let Some(live) = self.instances.get_mut(Handle::from_bits(instance)) {
                    live.values = values.iter().map(Value::from_wire).collect();
                    self.reply(&BrokerMessage::Done)
                } else {
                    // A forged, stale or wrong-kind handle is **answered**,
                    // never followed, and `Failed` is the answer at every entry
                    // point - never "unsupported", which would tell a plugin
                    // the feature is missing when the truth is its handle is
                    // rubbish (docs/impl/lfx.md §3.5).
                    self.failed(HostAction::Values, "no such instance")
                }
            }
            HostMessage::Action { instance, param } => self.press(instance, &param),
            HostMessage::Process { instance, request } => self.process(instance, &request),
            HostMessage::Destroy { instance } => {
                // Dropping the instance is what destroys it, and it happens on
                // this thread, which is the control thread the header pins the
                // lifecycle to.
                //
                // A handle this broker never minted is a `Failed` here as it is
                // at every other entry point: the shipping host guards before
                // it sends, and a rule kept only by the caller is the shape the
                // handle registry exists to avoid.
                if self.instances.remove(Handle::from_bits(instance)).is_none() {
                    return self.failed(HostAction::Destroy, "no such instance");
                }
                self.reply(&BrokerMessage::Done)
            }
            // A shipment nobody asked for is dropped: this version offers no
            // extension a plugin could ask for a neighbour through, so there is
            // no exchange for one to arrive in the middle of.
            HostMessage::Frames { .. } | HostMessage::Shutdown => Ok(()),
        }
    }

    // ------------------------------------------------ before any of its code --

    /// Answer the bundle's own listing, with the module shut.
    fn manifest(&mut self, path: &str) -> Result<(), String> {
        match manifest::read(Path::new(path)) {
            Ok(entries) => self.reply(&BrokerMessage::Manifested { entries }),
            Err(why) => self.failed(HostAction::Manifest, &why.to_string()),
        }
    }

    /// Open the module - the first time any of the bundle's code runs at all -
    /// and ask every plugin in it, bar the switched-off ones, what it is.
    fn describe(&mut self, disabled: &std::collections::BTreeSet<String>) -> Result<(), String> {
        let module = match self.open_module() {
            Ok(module) => module,
            Err(why) => return self.failed(HostAction::Describe, &why),
        };
        self.described.clear();
        let mut plugins: Vec<DescribedPlugin> = Vec::new();
        let mut refused: Vec<(String, LfxRejection)> = Vec::new();
        for identity in module.plugins() {
            if disabled.contains(&identity.id) {
                continue;
            }
            // A plugin that refuses to describe itself is not catalogued and
            // the bundle carries on: eleven good effects are not a bundle to
            // throw away because the twelfth declined.
            //
            // **But it is refused rather than dropped.** The describe runs
            // here, so a failure discarded here reaches the host as an absence
            // and nothing else - not catalogued, not refused, not in the
            // report, and indistinguishable from a plugin the user switched
            // off. §5.3's `REFUSED` table is "what was turned away this
            // session, with its own sentence", and this is where the sentence
            // is.
            match module.describe(&identity.id) {
                Ok(descriptor) => {
                    plugins.push(DescribedPlugin::from(descriptor.clone()));
                    self.described.insert(identity.id.clone(), descriptor);
                }
                Err(why) => refused.push((identity.id.clone(), why.as_rejection(&identity.id))),
            }
        }
        let report = module.report().to_vec();
        self.reply(&BrokerMessage::Described {
            plugins,
            refused,
            report,
        })
    }

    /// The module, opened if this is the first ask.
    ///
    /// The borrow is the `OnceLock`'s own rather than `self`'s, which is what
    /// lets the caller go on editing the session while holding it - and what
    /// lets an instance live in a map inside the session while borrowing the
    /// module outside it.
    fn open_module(&self) -> Result<&'module LocalHost, String> {
        if let Some(open) = self.module.get() {
            return Ok(open);
        }
        let opened = LocalHost::open(&self.module_path).map_err(|why| why.to_string())?;
        let _ = self.module.set(opened);
        self.module
            .get()
            .ok_or_else(|| "the module was opened and then lost".to_owned())
    }

    // ------------------------------------------------------------ instances --

    /// Make one instance of one of the described plugins.
    fn create(
        &mut self,
        instance: InstanceId,
        plugin: &str,
        values: &[ParamValue],
    ) -> Result<(), String> {
        let handle = Handle::from_bits(instance);
        if handle.kind() != Some(KIND_INSTANCE) {
            return self.failed(HostAction::CreateInstance, "that is not an instance handle");
        }
        let Some(module) = self.module.get() else {
            return self.failed(HostAction::CreateInstance, "nothing has been described yet");
        };
        let Some(descriptor) = self.described.get(plugin).cloned() else {
            return self.failed(HostAction::CreateInstance, "no such effect in this bundle");
        };
        let window = descriptor
            .identity
            .traits
            .as_ref()
            .map_or((0, 0), |traits| (traits.temporal_lo, traits.temporal_hi));
        let made = match module.create(&descriptor) {
            Ok(made) => made,
            Err(why) => {
                let sentence = why.to_string();
                return self.failed(HostAction::CreateInstance, &sentence);
            }
        };
        let live = Live {
            plugin: plugin.to_owned(),
            instance: made,
            values: values.iter().map(Value::from_wire).collect(),
            window,
        };
        if !self.instances.insert(handle, live) {
            return self.failed(HostAction::CreateInstance, "that is not an instance handle");
        }
        self.reply(&BrokerMessage::Created)
    }

    /// One of an instance's `ACTION` rows was pressed.
    ///
    /// **Acknowledged and nothing else, in version 1.** The frozen entry table
    /// has `init`, `destroy`, `describe`, `process` and `get_extension`, and no
    /// press hook at all, so there is nowhere for this to go - `lfx.overlay` is
    /// where the hook that would take it belongs, and that is a reserved id
    /// with no version 1 header (docs/impl/lfx.md §13).
    ///
    /// The answer is [`BrokerMessage::Done`] rather than a
    /// [`BrokerMessage::Failed`], and that is deliberate: a `Failed` is a strike
    /// against the plugin, and three presses of a button would disable an effect
    /// that has done nothing wrong. A handle that names nothing is still a
    /// `Failed`, because that is a fault rather than a gap.
    ///
    /// *ponytail:* an author may declare an `ACTION` row today and it will draw,
    /// keyframe nothing and do nothing. Either the entry table grows a press
    /// hook or `ACTION` joins `PATH` and `STRING` as a kind version 1 reserves;
    /// that call is made at the header, not here.
    fn press(&mut self, instance: InstanceId, param: &str) -> Result<(), String> {
        let _ = param;
        if self.instances.get(Handle::from_bits(instance)).is_none() {
            return self.failed(HostAction::Action, "no such instance");
        }
        self.reply(&BrokerMessage::Done)
    }

    /// Render one frame.
    fn process(&mut self, instance: InstanceId, request: &ProcessRequest) -> Result<(), String> {
        let handle = Handle::from_bits(instance);
        if self.instances.get(handle).is_none() {
            return self.failed(HostAction::Process, "no such instance");
        }
        let Some(ring) = self.ring.as_ref() else {
            return self.failed(HostAction::Process, "the frame ring was never opened");
        };
        let input = match read_slot(ring, request.input.slot, request.depth) {
            Ok(input) => input,
            Err(why) => return self.failed(HostAction::Process, &why),
        };
        // The neighbours are read for the same reason the host ships them: so
        // that a slot the host filled is a slot somebody looked at, and a wrong
        // one is a refusal rather than a silence. They go no further, because
        // version 1 offers no extension table and there is no call a plugin can
        // make to ask for one (docs/impl/lfx.md §10).
        for neighbour in &request.neighbours {
            if let Err(why) = read_slot(ring, neighbour.slot, request.depth) {
                return self.failed(HostAction::Process, &why);
            }
        }

        let width = request.dod.width();
        let height = request.dod.height();
        // **The output starts as the input, not as nought.** Two cases need it
        // and neither refuses anything downstream. A plugin that answers
        // `LFX_STATUS_OK` and writes nothing at all is identity - that is what
        // the ABI means by it, and what the in-process half pins against a
        // caller-owned buffer - so through here "as it found it" has to be the
        // picture rather than a black frame. And an ROI is the output region
        // asked for, full-frame being the degenerate case rather than the
        // assumption, so a plugin that honours a partial one leaves everything
        // outside it untouched. A zero-filled buffer would send both back as
        // black picture with no strike, no sentence and no badge.
        //
        // The two buffers are the same depth and the same length by
        // construction: `LocalInstance::process` refuses any pair that is not
        // `dod`-sized with `LocalError::FrameSize`, and this is that pair
        // twice.
        let mut output = input.clone();

        let call = Request {
            time: request.time,
            width,
            height,
            roi: corners(request.roi),
            dod: corners(request.dod),
            value_stride: None,
        };
        let (status, window, plugin) = {
            let Some(live) = self.instances.get_mut(handle) else {
                return self.failed(HostAction::Process, "no such instance");
            };
            let status =
                live.instance
                    .process(&call, &live.values, input.borrowed(), output.borrowed_mut());
            (status, live.window, live.plugin.clone())
        };
        if let Err(why) = status {
            // **The output slot is not written.** A failed process leaves the
            // host's own picture where it was, which is what lets the layer
            // render identity byte for byte rather than through a depth
            // boundary that would change it very slightly.
            //
            // The sentence names the plugin, because one broker hosts a whole
            // bundle and a badge that said only "the effect answered status 2"
            // would not say which of twelve.
            let sentence = format!("{plugin}: {why}");
            return self.failed(HostAction::Process, &sentence);
        }

        let misanswer = self.misanswer;
        let Some(ring) = self.ring.as_mut() else {
            return self.failed(HostAction::Process, "the frame ring was never opened");
        };
        // One pixel where a picture was asked for, when a test asks for it.
        let (bounds, output) = if misanswer == Misanswer::Size {
            (RectI::of(1, 1), output.first_pixel())
        } else {
            (request.dod, output)
        };
        let written = match &output {
            Held::F16(halves) => ring.write_f16(request.output, halves, bounds, true),
            Held::F32(whole) => ring.write_f32(request.output, whole, bounds, true),
        };
        if let Err(why) = written {
            let sentence = why.to_string();
            return self.failed(HostAction::Process, &sentence);
        }
        // A reply to a question nobody asked, when a test asks for it. The
        // frame really is in the slot; what is wrong is the sentence about it,
        // which is the shape a broker one message behind on the pipe has.
        if misanswer == Misanswer::Reply {
            return self.reply(&BrokerMessage::Created);
        }
        let slot = if misanswer == Misanswer::Slot {
            request.input.slot
        } else {
            request.output
        };
        self.reply(&BrokerMessage::Processed {
            slot,
            frames_needed: offsets_in(window),
        })
    }

    // --------------------------------------------------------------- saying --

    /// Answer one message, sending on anything the plugin said first.
    ///
    /// The lines go out **before** the answer they belong to, so a note about a
    /// frame arrives while the host is still waiting on that frame - which is
    /// what [`BrokerMessage::is_interim`] is for on the other side. The module's
    /// own list is capped and append-only, so forwarding is a matter of sending
    /// whatever is new since last time.
    fn reply(&mut self, message: &BrokerMessage) -> Result<(), String> {
        // Only what is new. `reply` runs once per answered message, a rendered
        // frame included, so cloning the whole list here would make a plugin
        // that logged sixty-four lines during its first describe pay a lock and
        // sixty-four string clones on every frame for the life of this process -
        // the per-frame allocation docs/13 §3 says fails review.
        let first = self.notes_sent;
        let said = self
            .module
            .get()
            .map(|module| module.notes_from(first))
            .unwrap_or_default();
        for note in &said {
            let kind = NoteKind::from_log_level(note.level).unwrap_or(NoteKind::Info);
            say(
                &mut self.sender,
                &BrokerMessage::Note {
                    kind,
                    text: note.text.clone(),
                },
            )?;
        }
        self.notes_sent = first.saturating_add(said.len());
        say(&mut self.sender, message)
    }

    /// Say that something did not work, in a sentence the host can badge.
    fn failed(&mut self, action: HostAction, message: &str) -> Result<(), String> {
        self.reply(&BrokerMessage::Failed {
            action,
            message: message.to_owned(),
        })
    }
}

/// One picture this program owns while a plugin is looking at it.
#[derive(Clone)]
enum Held {
    /// Half floats.
    F16(Vec<f16>),
    /// Single floats.
    F32(Vec<f32>),
}

impl Held {
    /// The read-only view the ABI takes.
    fn borrowed(&self) -> Pixels<'_> {
        match self {
            Held::F16(halves) => Pixels::F16(halves),
            Held::F32(whole) => Pixels::F32(whole),
        }
    }

    /// The writable view, which is the only memory a plugin may write.
    fn borrowed_mut(&mut self) -> PixelsMut<'_> {
        match self {
            Held::F16(halves) => PixelsMut::F16(halves),
            Held::F32(whole) => PixelsMut::F32(whole),
        }
    }

    /// The first pixel and nothing else: a one-by-one frame, for the test that
    /// asks what the host does with an answer that is not the size it asked
    /// for. Nothing in the shipping path calls this.
    fn first_pixel(&self) -> Self {
        match self {
            Held::F16(halves) => Held::F16(halves.iter().take(4).copied().collect()),
            Held::F32(whole) => Held::F32(whole.iter().take(4).copied().collect()),
        }
    }
}

/// Read one slot at the depth the request asked for.
///
/// The depth is the request's rather than the header's: the ring refuses a slot
/// written at one depth and read at the other, and that refusal is what "the
/// host never converts a depth to accommodate a plugin" looks like from in here.
fn read_slot(ring: &Ring, slot: Slot, depth: PixelDepth) -> Result<Held, String> {
    match depth {
        PixelDepth::F16 => ring
            .read_f16(slot)
            .map(|(_, halves)| Held::F16(halves))
            .map_err(|why| why.to_string()),
        PixelDepth::F32 => ring
            .read_f32(slot)
            .map(|(_, whole)| Held::F32(whole))
            .map_err(|why| why.to_string()),
    }
}

/// A rectangle as the four corners `lfx_process` carries.
const fn corners(rect: RectI) -> (i32, i32, i32, i32) {
    (rect.x0, rect.y0, rect.x1, rect.y1)
}

/// Every offset a declared window covers, which is what `frames_needed`
/// answers.
///
/// **The declaration, because version 1 has nothing finer.** `frames_needed` is
/// meant to be the instance's own answer through `lfx.temporal`, and there is no
/// `lfx.temporal` table for it to answer through - so the honest reply is the
/// window the plugin declared, which is the number the host sized its ring from
/// and the set the next frame key is taken over. A plugin that declared nothing
/// asks for nothing.
fn offsets_in(window: (i32, i32)) -> Vec<i32> {
    let (lo, hi) = window;
    if lo > hi {
        return Vec::new();
    }
    (lo..=hi).filter(|offset| *offset != 0).collect()
}
