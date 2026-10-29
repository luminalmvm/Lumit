//! The process side of the supervisor: naming a broker, starting it with no
//! console, and reading what it says. Split out of [`crate::ipc::broker`],
//! which keeps the conversation.
//!
//! The reading thread holds no lock and takes none.

use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::mpsc::{self, Receiver};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{Listener, SendHalf};

use crate::ipc::broker::BrokerError;
use crate::ipc::pipe;
use crate::ipc::proto::BrokerMessage;

/// Start the broker with no console window of its own.
///
/// A broker is a console program and Lumit is a windowed one, so on Windows
/// every spawn opens a console window in front of the editor — one per plugin
/// file, all at once, during the start-up scan. `CREATE_NO_WINDOW` gives the
/// child no console at all instead. Nothing is lost by it: the protocol was
/// never on the child's standard streams (see `ipc::pipe`), and its output is
/// already sent to nowhere.
pub(crate) fn no_console(command: &mut Command) {
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

/// The live connection to one broker process.
pub(crate) struct Link {
    pub(crate) child: Child,
    pub(crate) sender: SendHalf,
    pub(crate) incoming: Receiver<Incoming>,
}

/// What the reading thread hands back.
pub(crate) enum Incoming {
    /// The broker connected; here is the half to write to.
    Connected(SendHalf),
    /// A message.
    Message(Box<BrokerMessage>),
    /// The pipe closed — which, for a child process, means it died.
    Gone,
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
pub(crate) fn fresh_identifier() -> Result<String, BrokerError> {
    Ok(lumit_peer::Token::generate()
        .map_err(BrokerError::Peer)?
        .as_name())
}

/// Where the ring with this name lives.
pub(crate) fn ring_path(identifier: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("lumit-ofx-{identifier}.ring"));
    path
}

/// Let the plugin's window come to the front. Windows only lets the process
/// that has the foreground pass that right on, and the plugin lives in the
/// broker, so without this its editor opens behind Lumit.
#[cfg(windows)]
pub(crate) fn allow_foreground(broker: Option<u32>) {
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
pub(crate) fn allow_foreground(_broker: Option<u32>) {}

/// The reading thread: accept the one connection, hand the writing half back,
/// then read until the pipe closes.
///
/// It holds no lock and takes none, which is what lets the host wait on a
/// deadline rather than on the plugin (docs/14 §1).
pub(crate) fn read_loop(listener: Listener, tx: &mpsc::Sender<Incoming>) {
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
    use crate::ipc::broker::{describe_deadline, DESCRIBE_TIMEOUT};
    use crate::quirks::Quirks;

    /// The shipped two-second control deadline is not what describe waits
    /// under: the first describe opens the module, which is a program starting.
    #[test]
    fn describe_takes_its_own_ceiling_by_default() {
        let quirks = Quirks::default();
        assert!(quirks.control_timeout < DESCRIBE_TIMEOUT);
        assert_eq!(describe_deadline(&quirks), DESCRIBE_TIMEOUT);
    }
}
