//! `OfxMessageSuiteV1` — what a plugin says to the user.
//!
//! # In plain terms
//!
//! Plugins occasionally want to tell someone something: an error, a warning,
//! or a yes/no question. There is nowhere to put those yet — the panel that
//! shows them arrives with the out-of-process broker — so they are kept in a
//! short list on the host and a question is answered "you decide", which is
//! the reply OFX defines for a host that cannot ask.

use std::ffi::{c_char, c_int, c_void};

use crate::ffi::{message_types, OfxMessageSuiteV1, OfxMessageSuiteV2};
use crate::host::{state, HostMessage};
use crate::status::Status;
use crate::suites::{cstr, guard_code};

/// The table handed out by `fetchSuite`.
pub static SUITE: OfxMessageSuiteV1 = OfxMessageSuiteV1 { message };

/// Version 2: the same `message`, plus a persistent one. A persistent message
/// is one the plugin wants shown until it says otherwise — an unlicensed
/// plugin's "trial", a missing file's name — and this host has one place for
/// that already: the badge, which reads the message log (docs/12 §2.2). So a
/// persistent message is filed like any other and clearing it is a no-op the
/// spec allows; nothing stays up that the badge was not already showing
/// (K-757).
pub static SUITE_V2: OfxMessageSuiteV2 = OfxMessageSuiteV2 {
    message,
    set_persistent_message: message,
    clear_persistent_message,
};

unsafe extern "C" fn clear_persistent_message(_handle: *mut c_void) -> c_int {
    guard_code(|| Status::Ok)
}

unsafe extern "C" fn message(
    _handle: *mut c_void,
    message_type: *const c_char,
    message_id: *const c_char,
    format: *const c_char,
) -> c_int {
    guard_code(|| {
        // A message with an unreadable type or text is not worth failing an
        // action over; it is dropped and the plugin is told the message was
        // handled, because there is nothing it could do about it.
        // SAFETY: OFX string arguments, each checked for null inside `cstr`.
        let message_type = unsafe { cstr(message_type) }.unwrap_or_default();
        // SAFETY: as above; the identifier is optional and often null.
        let message_id = unsafe { cstr(message_id) }.unwrap_or_default();
        // SAFETY: as above.
        let text = unsafe { cstr(format) }.unwrap_or_default();

        state().push_message(HostMessage {
            message_type: message_type.to_owned(),
            message_id: message_id.to_owned(),
            text: text.to_owned(),
        });

        if message_type == message_types::QUESTION {
            Status::ReplyDefault
        } else {
            Status::Ok
        }
    })
}
