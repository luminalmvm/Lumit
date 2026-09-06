//! `lumit-ofx-broker` — the program a plugin lives in.
//!
//! # In plain terms
//!
//! This is a very small program whose whole job is to be expendable. Lumit
//! starts one of these per plugin bundle, hands it the bundle and a pipe, and
//! never loads the plugin itself. Everything the plugin does — describing
//! itself, asking for pictures, rendering — happens here, and every suite call
//! it makes lands in `lumit-ofx`'s own stub, in *this* process, where the memory
//! suite and the threading suite are answered on the spot and only clip images
//! and parameter values have to be asked for down the pipe.
//!
//! When it crashes, one frame is lost and Lumit starts another one. That is the
//! entire design (docs/12 §1, docs/impl/ofx-host.md §4).
//!
//! It reads two arguments — the `.ofx` binary inside the bundle, and the pipe
//! name — and speaks the protocol in `lumit_ofx::ipc::proto`. Its first word is
//! a version number, because two programs that disagree about the shape of a
//! message must find out before one of them acts on the other's bytes.
//!
//! Nothing here is clever, and nothing here recovers. Recovery is Lumit's job,
//! and it does it by starting this program again.

use std::collections::BTreeMap;
use std::process::ExitCode;

use lumit_eval::epoch::Epoch;
use lumit_ofx::bundle::Bundle;
use lumit_ofx::describe::{describe, Context, PluginDescriptor};
use lumit_ofx::image::{Frame16, RectI};
use lumit_ofx::instance::{time_key, Instance, ParamSnapshot};
use lumit_ofx::ipc::pipe::{self, RecvHalf, SendHalf};
use lumit_ofx::ipc::proto::{
    BrokerMessage, FrameWanted, HostMessage, InstanceId, PROTOCOL_VERSION,
};
use lumit_ofx::ipc::shm::Ring;
use lumit_ofx::render::{render_with_prefetch, RenderError, RenderRequest};

/// The protocol version this broker announces. Normally
/// [`PROTOCOL_VERSION`]; an environment variable overrides it so that a test
/// can put a host and a broker that disagree in the same room and watch the
/// host refuse rather than deserialise.
const PROTOCOL_ENV: &str = "LUMIT_OFX_BROKER_PROTOCOL";

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(bundle), Some(pipe_name)) = (args.next(), args.next()) else {
        eprintln!("usage: lumit-ofx-broker <bundle.ofx> <pipe name>");
        return ExitCode::FAILURE;
    };
    let Some(pipe_name) = pipe_name.to_str().map(str::to_owned) else {
        eprintln!("the pipe name is not text");
        return ExitCode::FAILURE;
    };

    match run(&bundle, &pipe_name) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("lumit-ofx-broker: {why}");
            ExitCode::FAILURE
        }
    }
}

/// Everything, once the arguments are known.
fn run(bundle_path: &std::ffi::OsStr, pipe_name: &str) -> Result<(), String> {
    let stream = pipe::connect(pipe_name).map_err(|error| error.to_string())?;
    let (receiver, mut sender) = pipe::split(stream);

    let version = std::env::var(PROTOCOL_ENV)
        .ok()
        .and_then(|text| text.parse::<u32>().ok())
        .unwrap_or(PROTOCOL_VERSION);
    say(&mut sender, &BrokerMessage::Hello { version })?;

    let mut bundle = Bundle::open(bundle_path).map_err(|error| error.to_string())?;
    bundle.load();

    let mut session = Session {
        bundle,
        receiver,
        sender,
        ring: None,
        described: Vec::new(),
        instances: BTreeMap::new(),
    };
    session.serve()
}

/// Write one message.
fn say(sender: &mut SendHalf, message: &BrokerMessage) -> Result<(), String> {
    pipe::send(sender, message).map_err(|error| error.to_string())
}

/// One plugin's instance, and which plugin it is.
struct Live {
    plugin: usize,
    instance: Instance,
}

/// The broker's whole state. One bundle, one pipe, one ring.
struct Session {
    bundle: Bundle,
    receiver: RecvHalf,
    sender: SendHalf,
    ring: Option<Ring>,
    /// The plugins that described themselves, and where each sits in the
    /// bundle. The index into this list is what the host names.
    described: Vec<(usize, PluginDescriptor)>,
    instances: BTreeMap<InstanceId, Live>,
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
            HostMessage::Describe => {
                let plugins = self.describe_all();
                self.reply(&BrokerMessage::Described { plugins })
            }
            HostMessage::CreateInstance {
                instance,
                plugin,
                context,
                params,
            } => self.create(instance, plugin, context, params),
            HostMessage::ParamSnapshot { instance, params } => {
                if let Some(live) = self.instances.get(&instance) {
                    let _ = live.instance.set_params(params);
                }
                self.reply(&BrokerMessage::Done)
            }
            HostMessage::Press {
                instance,
                name,
                time,
                source,
            } => {
                let Some(live) = self.instances.get(&instance) else {
                    return self.failed("press", "no such instance");
                };
                let Some(plugin) = self.bundle.plugins().get(live.plugin) else {
                    return self.failed("press", "no such plugin in this bundle");
                };
                let frame = self
                    .ring
                    .as_ref()
                    .and_then(|ring| ring.read_frame(source.slot).ok())
                    .map(|(_, frame)| frame);
                let Some(frame) = frame else {
                    return self.failed("press", "the frame did not arrive");
                };
                // This call is as long as the plugin wants it to be. Looks
                // stays in here until its editor is closed.
                match live.instance.press(plugin, &name, time, &frame) {
                    Ok(params) => self.reply(&BrokerMessage::Pressed { params }),
                    Err(status) => self.failed("press", &format!("{status:?}")),
                }
            }
            HostMessage::Render {
                instance,
                time,
                bounds,
                order,
                inputs,
                output,
            } => {
                let request = self.request(time, bounds, order, &inputs);
                self.render(instance, request, output)
            }
            HostMessage::Destroy { instance } => {
                if let Some(live) = self.instances.remove(&instance) {
                    if let Some(plugin) = self.bundle.plugins().get(live.plugin) {
                        let _ = live.instance.destroy(plugin);
                    }
                }
                self.reply(&BrokerMessage::Done)
            }
            // A shipment nobody asked for is dropped: the only place one is
            // expected is inside a render, where it is read there and then.
            HostMessage::Frames { .. } | HostMessage::Shutdown => Ok(()),
        }
    }

    /// Describe every plugin in the bundle, keeping the ones that succeeded.
    fn describe_all(&mut self) -> Vec<PluginDescriptor> {
        self.described.clear();
        for (index, plugin) in self.bundle.plugins().iter().enumerate() {
            if !plugin.is_supported_image_effect() {
                continue;
            }
            if let Ok(descriptor) = describe(plugin) {
                self.described.push((index, descriptor));
            }
        }
        self.described
            .iter()
            .map(|(_, descriptor)| descriptor.clone())
            .collect()
    }

    /// Make one instance.
    fn create(
        &mut self,
        instance: InstanceId,
        plugin: u32,
        context: Context,
        params: ParamSnapshot,
    ) -> Result<(), String> {
        let index = plugin as usize;
        let Some((bundle_index, descriptor)) = self.described.get(index).cloned() else {
            return self.failed("createInstance", "no such plugin in this bundle");
        };
        let Some(plugin_ref) = self.bundle.plugins().get(bundle_index) else {
            return self.failed("createInstance", "no such plugin in this bundle");
        };
        match Instance::create(plugin_ref, &descriptor, context, &params) {
            Ok(created) => {
                self.instances.insert(
                    instance,
                    Live {
                        plugin: bundle_index,
                        instance: created,
                    },
                );
                self.reply(&BrokerMessage::Created)
            }
            Err(status) => self.failed("createInstance", &format!("{status:?}")),
        }
    }

    /// Build a render request out of the slots the host named.
    fn request(
        &self,
        time: f64,
        bounds: RectI,
        order: lumit_ofx::image::RowOrder,
        inputs: &[lumit_ofx::ipc::proto::FrameRef],
    ) -> RenderRequest {
        let mut frames = BTreeMap::new();
        if let Some(ring) = self.ring.as_ref() {
            for input in inputs {
                if let Ok((_, frame)) = ring.read_frame(input.slot) {
                    frames.insert(input.clip.clone(), frame);
                }
            }
        }
        RenderRequest {
            time,
            bounds,
            order,
            inputs: frames,
        }
    }

    /// Drive one render, fetching whatever `getFramesNeeded` asks for in one
    /// go, and put the answer in the ring.
    fn render(
        &mut self,
        instance: InstanceId,
        request: RenderRequest,
        output: u32,
    ) -> Result<(), String> {
        // The instance, the plugin and the two pipe halves are all borrowed out
        // of `self` at once and they are different fields, which is what lets
        // the prefetch closure below write to the pipe in the middle of a
        // render without borrowing the whole session.
        let Some(live) = self.instances.get(&instance) else {
            return self.failed("render", "no such instance");
        };
        let Some(plugin) = self.bundle.plugins().get(live.plugin) else {
            return self.failed("render", "no such plugin in this bundle");
        };
        let ring = self.ring.as_ref();
        let sender = &mut self.sender;
        let receiver = &mut self.receiver;

        // Never cancelled here: cancellation lives in the host, which drops the
        // frame it no longer wants. Carrying the epoch across the pipe is worth
        // doing only once there is a plugin slow enough to need it, and the
        // watchdog covers the case that matters.
        let epoch = Epoch::new();
        let token = epoch.token();
        let time = request.time;

        let mut prefetch = |needed: &BTreeMap<String, (f64, f64)>| {
            let wanted = frames_wanted(needed, time);
            if wanted.is_empty() {
                return Ok(BTreeMap::new());
            }
            fetch(sender, receiver, ring, &wanted)
        };

        let rendered =
            render_with_prefetch(plugin, &live.instance, &request, &token, &mut prefetch);
        let (frame, frames_needed, identity_of) = match rendered {
            Ok(rendered) => (rendered.frame, rendered.frames_needed, rendered.identity_of),
            Err(error) => return self.failed("render", &error.to_string()),
        };

        let bounds = request.bounds;
        let Some(ring) = self.ring.as_mut() else {
            return self.failed("render", "the frame ring was never opened");
        };
        if ring.write_frame(output, &frame, bounds, true).is_err() {
            return self.failed("render", "the frame does not fit the ring");
        }
        self.reply(&BrokerMessage::Rendered {
            slot: output,
            frames_needed,
            identity_of,
        })
    }

    /// Answer one message, sending on anything the plugin said to the user
    /// first.
    ///
    /// The messages are drained here rather than forwarded from inside the
    /// message suite deliberately: the suite is called with the host state's
    /// lock held, and writing to a pipe with a lock held is exactly what
    /// docs/14 §1 forbids. So they queue in the host state's own capped log —
    /// which is where a chatty plugin's memory use stops — and go out between
    /// actions, before the answer they belong to.
    fn reply(&mut self, message: &BrokerMessage) -> Result<(), String> {
        let said = {
            let mut state = lumit_ofx::host::state();
            std::mem::take(&mut state.messages)
        };
        for note in said {
            say(
                &mut self.sender,
                &BrokerMessage::Note {
                    kind: note.message_type,
                    text: note.text,
                },
            )?;
        }
        say(&mut self.sender, message)
    }

    /// Say that something did not work, in a sentence the host can badge.
    fn failed(&mut self, action: &str, message: &str) -> Result<(), String> {
        self.reply(&BrokerMessage::Failed {
            action: action.to_owned(),
            message: message.to_owned(),
        })
    }
}

/// Every frame `getFramesNeeded` asked for that is not the one being rendered,
/// as one flat list. The ranges come back per clip as a first and a last frame;
/// a retimer's t±5 is eleven frames, and eleven is what goes on the wire — once.
fn frames_wanted(needed: &BTreeMap<String, (f64, f64)>, time: f64) -> Vec<FrameWanted> {
    /// A range no plugin means, and a list no host should build. A plugin that
    /// asks for a thousand frames of one output frame has asked for something
    /// the ring cannot hold anyway.
    const MAX_FRAMES: usize = 256;

    let mut wanted = Vec::new();
    for (clip, (first, last)) in needed {
        if !first.is_finite() || !last.is_finite() || last < first {
            continue;
        }
        let mut frame = first.floor();
        while frame <= *last && wanted.len() < MAX_FRAMES {
            if time_key(frame) != time_key(time) {
                wanted.push(FrameWanted {
                    clip: clip.clone(),
                    time: frame,
                });
            }
            frame += 1.0;
        }
    }
    wanted
}

/// Ask the host for frames, and wait for the one shipment that answers.
fn fetch(
    sender: &mut SendHalf,
    receiver: &mut RecvHalf,
    ring: Option<&Ring>,
    wanted: &[FrameWanted],
) -> Result<BTreeMap<(String, i64), Frame16>, RenderError> {
    let message = BrokerMessage::NeedFrames {
        frames: wanted.to_vec(),
    };
    if pipe::send(sender, &message).is_err() {
        return Ok(BTreeMap::new());
    }
    let reply: HostMessage = match pipe::recv(receiver) {
        Ok(reply) => reply,
        Err(_) => return Ok(BTreeMap::new()),
    };
    let HostMessage::Frames { frames } = reply else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    let Some(ring) = ring else {
        return Ok(out);
    };
    for shipped in frames {
        if let Ok((_, frame)) = ring.read_frame(shipped.slot) {
            out.insert((shipped.clip, time_key(shipped.time)), frame);
        }
    }
    Ok(out)
}
