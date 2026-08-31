//! `lumit-aplug-broker` — the program an audio plugin lives in.
//!
//! # In plain terms
//!
//! This is a very small program whose whole job is to be expendable. Lumit
//! starts one of these per plugin module — a `.clap` file or a `.vst3` bundle,
//! and one binary serves both — hands it the module and a pipe, and never loads
//! the plugin itself. Everything the plugin does — being started,
//! describing itself, playing sound — happens here.
//!
//! When it crashes, one block of sound is lost and Lumit starts another one.
//! The layer plays that block **dry** and carries a calm badge. That is the
//! entire design (docs/12 §1, docs/impl/audio-plugins.md §5).
//!
//! It reads two arguments — the module and the pipe name — and speaks the
//! protocol in `lumit_aplug::ipc::proto`. Its first word is a version number,
//! because two programs that disagree about the shape of a message must find out
//! before one of them acts on the other's bytes.
//!
//! Nothing here is clever, and nothing here recovers. Recovery is Lumit's job,
//! and it does it by starting this program again and replaying what it holds.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

use lumit_aplug::abi::AnyModule;
use lumit_aplug::def::{AudioHost, InstanceSetup, LocalHost};
use lumit_aplug::describe::describe_module_except;
use lumit_aplug::ipc::handles::{Handle, Registry, KIND_INSTANCE};
use lumit_aplug::ipc::pipe::{self, RecvHalf, SendHalf};
use lumit_aplug::ipc::proto::{
    Bring, BrokerMessage, HostMessage, InstanceId, Slot, PROTOCOL_VERSION,
};
use lumit_aplug::ipc::ring::Ring;
use lumit_aplug::process::{ParamEvent, INTERLEAVED_LEN};

/// The protocol version this broker announces. Normally [`PROTOCOL_VERSION`];
/// an environment variable overrides it so that a test can put a host and a
/// broker that disagree in the same room and watch the host refuse rather than
/// deserialise.
const PROTOCOL_ENV: &str = "LUMIT_APLUG_BROKER_PROTOCOL";

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(module), Some(pipe_name)) = (args.next(), args.next()) else {
        eprintln!("usage: lumit-aplug-broker <module.clap|module.vst3> <pipe name>");
        return ExitCode::FAILURE;
    };
    let Some(pipe_name) = pipe_name.to_str().map(str::to_owned) else {
        eprintln!("the pipe name is not text");
        return ExitCode::FAILURE;
    };

    match run(PathBuf::from(module), &pipe_name) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("lumit-aplug-broker: {why}");
            ExitCode::FAILURE
        }
    }
}

/// Everything, once the arguments are known.
fn run(module_path: PathBuf, pipe_name: &str) -> Result<(), String> {
    let stream = pipe::connect(pipe_name).map_err(|error| error.to_string())?;
    let (receiver, mut sender) = pipe::split(stream);

    let version = std::env::var(PROTOCOL_ENV)
        .ok()
        .and_then(|text| text.parse::<u32>().ok())
        .unwrap_or(PROTOCOL_VERSION);
    say(&mut sender, &BrokerMessage::Hello { version })?;

    // The module is **not** opened yet. Opening it runs a `clap_entry.init` or
    // an `InitDll`, which is third-party code either way, and a host that has
    // only just said hello has not yet
    // told us which plugins the user switched off. It opens on the first
    // question that needs it, which is the describe that carries that list.
    let mut session = Session {
        module_path,
        module: None,
        receiver,
        sender,
        ring: None,
        instances: Registry::new(KIND_INSTANCE),
        scratch: vec![0.0; INTERLEAVED_LEN],
        played: vec![0.0; INTERLEAVED_LEN],
    };
    session.serve()
}

/// Write one message.
fn say(sender: &mut SendHalf, message: &BrokerMessage) -> Result<(), String> {
    pipe::send(sender, message).map_err(|error| error.to_string())
}

/// The broker's whole state. One module, one pipe, one ring.
struct Session {
    module_path: PathBuf,
    module: Option<AnyModule>,
    receiver: RecvHalf,
    sender: SendHalf,
    ring: Option<Ring>,
    /// The live plugins, by the handle the host minted. The handle carries a
    /// magic pattern and a kind, and a message quoting one that fails either is
    /// answered rather than followed.
    instances: Registry<LocalHost>,
    /// The block coming in, allocated once.
    scratch: Vec<f32>,
    /// The block going out, allocated once and never the same buffer as the
    /// one coming in.
    played: Vec<f32>,
}

impl Session {
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
            HostMessage::Open { ring } => {
                self.ring = Ring::open(&ring).ok();
                Ok(())
            }
            HostMessage::Describe { disabled } => self.describe(&disabled),
            HostMessage::CreateInstance { instance, bring } => self.create(instance, bring),
            HostMessage::Process {
                instance,
                input,
                output,
                events,
                steady,
            } => self.process(instance, input, output, &events, steady),
            HostMessage::Save { instance } => self.save(instance),
            HostMessage::Destroy { instance } => {
                self.instances.remove(Handle::from_bits(instance));
                say(&mut self.sender, &BrokerMessage::Done)
            }
            HostMessage::Shutdown => Ok(()),
        }
    }

    /// The module, opening it if this is the first question that needs it.
    ///
    /// Which standard it speaks is read off the file's own name (`AnyModule`),
    /// so **one broker binary serves both** — the pipe, the ring, the handle
    /// registry and the watchdog are the same code either way (K-707).
    fn module(&mut self) -> Option<AnyModule> {
        if self.module.is_none() {
            self.module = AnyModule::open(&self.module_path).ok();
        }
        self.module.clone()
    }

    /// Open the module and ask every plugin in it what it is — except the ones
    /// the user switched off, which are never created at all.
    fn describe(&mut self, disabled: &[String]) -> Result<(), String> {
        let skip: BTreeSet<String> = disabled.iter().cloned().collect();
        let Some(module) = self.module() else {
            return self.failed("describe", "the module did not load");
        };
        let report = describe_module_except(&module, &skip);
        say(
            &mut self.sender,
            &BrokerMessage::Described {
                plugins: report.described,
                rejected: report.rejected,
            },
        )
    }

    /// Make one instance, in the order [`lumit_aplug::HOST_ACTIONS`] pins.
    fn create(&mut self, instance: InstanceId, bring: Bring) -> Result<(), String> {
        let handle = Handle::from_bits(instance);
        if handle.kind() != Some(KIND_INSTANCE) {
            return self.failed("createInstance", "that is not an instance handle");
        }
        let Some(module) = self.module() else {
            return self.failed("createInstance", "the module did not load");
        };
        let setup = InstanceSetup {
            plugin_id: bring.plugin_id,
            state: bring.state,
            params: bring.params,
            offline: bring.offline,
        };
        match LocalHost::open(&module, &setup) {
            Ok(host) => {
                let latency = host.latency();
                let warning = host.warning().map(str::to_owned);
                if !self.instances.insert(handle, host) {
                    return self.failed("createInstance", "that is not an instance handle");
                }
                say(
                    &mut self.sender,
                    &BrokerMessage::Created { latency, warning },
                )
            }
            Err(error) => self.failed("createInstance", &error.to_string()),
        }
    }

    /// One block: read the input slot, play it, write the output slot.
    fn process(
        &mut self,
        instance: InstanceId,
        input: Slot,
        output: Slot,
        events: &[ParamEvent],
        steady: i64,
    ) -> Result<(), String> {
        match self.play(instance, input, output, events, steady) {
            Ok(()) => say(&mut self.sender, &BrokerMessage::Processed { slot: output }),
            Err(why) => self.failed("process", &why),
        }
    }

    /// The block itself, with every buffer taken out of `self` at once.
    ///
    /// The fields are destructured rather than reached through `self` one at a
    /// time so that the ring, the plugin and the two scratch buffers can all be
    /// held together — which is what lets a block cost **no allocation at all**
    /// (docs/14's budgeted allocations). The two buffers stay separate because
    /// in-place processing is where plugin bugs live (§9).
    fn play(
        &mut self,
        instance: InstanceId,
        input: Slot,
        output: Slot,
        events: &[ParamEvent],
        steady: i64,
    ) -> Result<(), String> {
        let Self {
            ring,
            instances,
            scratch,
            played,
            ..
        } = self;
        let ring = ring
            .as_mut()
            .ok_or_else(|| "the block ring was never opened".to_owned())?;
        ring.read_block(input, scratch)
            .map_err(|error| error.to_string())?;
        let host = instances
            .get(Handle::from_bits(instance))
            .ok_or_else(|| "no such plugin instance".to_owned())?;
        host.process(scratch, played, events, steady)
            .map_err(|error| error.to_string())?;
        ring.write_block(output, played)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// The plugin's own memory of itself, byte for byte.
    fn save(&mut self, instance: InstanceId) -> Result<(), String> {
        let handle = Handle::from_bits(instance);
        let Some(host) = self.instances.get(handle) else {
            return self.failed("save", "no such plugin instance");
        };
        match host.save() {
            Ok(bytes) => say(&mut self.sender, &BrokerMessage::Saved { bytes }),
            Err(error) => self.failed("save", &error.to_string()),
        }
    }

    /// Say that something did not work, in a sentence the host can badge.
    fn failed(&mut self, action: &str, message: &str) -> Result<(), String> {
        say(
            &mut self.sender,
            &BrokerMessage::Failed {
                action: action.to_owned(),
                message: message.to_owned(),
            },
        )
    }
}
