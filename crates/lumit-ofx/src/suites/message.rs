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

use crate::ffi::{message_types, OfxMessageSuiteV1};
use crate::host::{state, HostMessage};
use crate::status::Status;
use crate::suites::{cstr, guard_code};

/// The table handed out by `fetchSuite`.
pub static SUITE: OfxMessageSuiteV1 = OfxMessageSuiteV1 { message };

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
