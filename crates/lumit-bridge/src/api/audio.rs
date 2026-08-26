//! Hearing a composition.
//!
//! # In plain terms
//!
//! Playing a comp with sound is not one thing but two clocks that have to agree.
//! The picture is drawn frame by frame; the sound is handed to the operating
//! system in a continuous stream and plays on its own. Left alone they drift —
//! and drift between picture and sound is the one timing error everybody
//! notices.
//!
//! So the sound is the master. Once a mix is loaded, the frontend asks
//! [`audio_clock`] where playback actually *is* and draws that frame, rather
//! than counting frames and hoping. Until a mix is loaded — while it is still
//! being decoded, or on a machine with no sound device at all — `loaded` reads
//! false and the caller keeps its own wall clock, which is why silence never
//! stops the picture.
//!
//! **Preparing is asynchronous on purpose.** Building a mix means decoding every
//! contributing source, which is far too slow to block a play button. `prepare`
//! returns at once and the mix arrives when it arrives; an edit that does not
//! change what the comp sounds like is recognised and costs nothing.

use flutter_rust_bridge::frb;

use crate::api::{composition::CompositionReference, BridgeError};

/// Where playback is, as the transport needs to know it.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeAudioClock {
    /// Seconds into the composition.
    pub seconds: f64,
    pub playing: bool,
    /// False while no mix is loaded — no device, nothing prepared yet, or a
    /// prepare still running. The caller should drive the picture from its own
    /// clock until this turns true, and follow `seconds` after that.
    pub loaded: bool,
}

impl CompositionReference {
    /// Build or refresh this comp's mix in the background.
    ///
    /// Call it after an edit while audio is loaded or playing. An edit that does
    /// not change what the comp sounds like — moving a silent layer, renaming
    /// something — is recognised by a signature and costs nothing.
    #[frb(sync)]
    pub fn audio_prepare(&self) -> Result<(), BridgeError> {
        let document = self.document_snapshot()?;
        #[cfg(feature = "media")]
        crate::audio::prepare(self.id, document);
        #[cfg(not(feature = "media"))]
        let _ = document;
        Ok(())
    }

    /// Start playing this comp's audio from `start` seconds.
    #[frb(sync)]
    pub fn audio_play(&self, start: f64) -> Result<(), BridgeError> {
        let document = self.document_snapshot()?;
        #[cfg(feature = "media")]
        crate::audio::play(self.id, start, document);
        #[cfg(not(feature = "media"))]
        let _ = (start, document);
        Ok(())
    }

    /// The document as it stands, for the mix to be built from.
    ///
    /// Captured here rather than read on the worker thread: the mix must be of
    /// the comp as it was when playback was asked for, and the worker may not
    /// start for some milliseconds.
    #[frb(ignore)]
    fn document_snapshot(&self) -> Result<std::sync::Arc<lumit_core::Document>, BridgeError> {
        let state = self.project()?;
        let state = state.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(state.store.snapshot())
    }
}

/// Pause. The clock holds its position, so play resumes from here.
#[frb(sync)]
pub fn audio_pause() {
    #[cfg(feature = "media")]
    crate::audio::pause();
}

/// Move the clock to `secs` — a scrub. The play state is untouched, so
/// scrubbing while playing keeps playing.
#[frb(sync)]
pub fn audio_seek(secs: f64) {
    #[cfg(feature = "media")]
    crate::audio::seek(secs);
    #[cfg(not(feature = "media"))]
    let _ = secs;
}

/// Stop: pause and rewind to the start.
#[frb(sync)]
pub fn audio_stop() {
    #[cfg(feature = "media")]
    crate::audio::stop();
}

/// Where playback is. Polled every tick, so it allocates nothing and takes one
/// short lock.
#[frb(sync)]
pub fn audio_clock() -> BridgeAudioClock {
    #[cfg(feature = "media")]
    {
        let (seconds, playing, loaded) = crate::audio::clock();
        BridgeAudioClock {
            seconds,
            playing,
            loaded,
        }
    }
    // No decoder, so no mix and no device: the caller keeps its own clock,
    // which is the same path a machine with no sound card takes.
    #[cfg(not(feature = "media"))]
    BridgeAudioClock {
        seconds: 0.0,
        playing: false,
        loaded: false,
    }
}

/// One output the machine can play through.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAudioDevice {
    /// What `set_audio_device` is called with. Stable across restarts, so the
    /// frontend can store it and hand it back; opaque otherwise.
    pub id: String,
    /// What the sound system calls it, for the list.
    pub name: String,
    /// Whether this is the one the system plays through when nothing is chosen.
    pub is_default: bool,
}

/// What the machine offers, and what Lumit is actually playing through.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAudioDevices {
    pub devices: Vec<BridgeAudioDevice>,
    /// The id sound comes out of, or empty when the machine has no output at
    /// all. Not necessarily the id that was chosen — see `fell_back`.
    pub active: String,
    /// True when a device was chosen and it is not there any more, so the
    /// system default is being played through instead. The choice is kept, not
    /// rewritten: plug the device back in and it is used again.
    pub fell_back: bool,
}

/// Every output the machine offers, and which one is in use.
///
/// Asking the sound system is not free, so this is a call to make when a list
/// is about to be shown — never in a rebuild path.
#[frb(sync)]
pub fn list_audio_devices() -> BridgeAudioDevices {
    #[cfg(feature = "media")]
    let (list, active, fell_back) = crate::audio::devices();
    // Without a decoder nothing plays, but the machine's outputs are still
    // worth naming: the setting is stored either way, and a list that went
    // empty on a build with no FFmpeg would look like a broken sound card.
    #[cfg(not(feature = "media"))]
    let (list, active, fell_back) = {
        let list = lumit_audio::output_devices();
        let active = list.resolve(None);
        (list, active, false)
    };
    BridgeAudioDevices {
        devices: list
            .devices
            .iter()
            .map(|d| BridgeAudioDevice {
                id: d.id.clone(),
                name: d.name.clone(),
                is_default: list.default_id.as_deref() == Some(d.id.as_str()),
            })
            .collect(),
        active: active.unwrap_or_default(),
        fell_back,
    }
}

/// Play through the output with this id; an empty string means the system
/// default, which is what Lumit ships following.
///
/// Sound stops until the next play: a stream cannot be moved from one device to
/// another, so the open one is closed and the next mix opens the new one. The
/// choice is the machine's, not the project's — the frontend stores it in the
/// settings file and hands it over on every boot, which is a no-op when it has
/// not changed.
#[frb(sync)]
pub fn set_audio_device(id: String) {
    #[cfg(feature = "media")]
    crate::audio::set_device(Some(id).filter(|s| !s.is_empty()));
    #[cfg(not(feature = "media"))]
    let _ = id;
}
