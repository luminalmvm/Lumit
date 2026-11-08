//! Writing a file in the browser build. The page encodes it with the
//! browser's own encoders and saves it where the user says
//! (flutter_ui/web/media.js); the engine hands it each frame and the sound.

use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::JsValue;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_namespace = lumitMedia, js_name = exportCodecs)]
    fn page_codecs() -> Result<String, JsValue>;
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(catch, js_namespace = lumitMedia, js_name = exportOpen)]
    fn page_open(
        name: &str,
        codec: &str,
        width: u32,
        height: u32,
        fps_num: i32,
        fps_den: i32,
        bit_rate: f64,
        sound_rate: u32,
        sound_channels: u32,
        sound_bit_rate: f64,
    ) -> Result<u32, JsValue>;
    #[wasm_bindgen(catch, js_namespace = lumitMedia, js_name = exportState)]
    fn page_state(id: u32) -> Result<String, JsValue>;
    #[wasm_bindgen(catch, js_namespace = lumitMedia, js_name = exportFrame)]
    fn page_frame(id: u32, rgba: &[u8], n: usize) -> Result<(), JsValue>;
    #[wasm_bindgen(catch, js_namespace = lumitMedia, js_name = exportSound)]
    fn page_sound(id: u32, samples: &[f32], first: usize) -> Result<(), JsValue>;
    #[wasm_bindgen(catch, js_namespace = lumitMedia, js_name = exportFinish)]
    fn page_finish(id: u32) -> Result<(), JsValue>;
    #[wasm_bindgen(catch, js_namespace = lumitMedia, js_name = exportCancel)]
    fn page_cancel(id: u32) -> Result<(), JsValue>;
}

/// The export dialogue's keys for the formats this browser can write:
/// `h264`, `hevc`, `vp9`, `av1`, those it has an encoder for.
pub fn codecs() -> Vec<String> {
    page_codecs()
        .unwrap_or_default()
        .split(',')
        .filter(|codec| !codec.is_empty())
        .map(str::to_owned)
        .collect()
}

/// What a file is written as.
pub struct Settings<'a> {
    /// The file's name, which the page offers when it asks where to save it.
    pub name: &'a str,
    /// One of [`codecs`].
    pub codec: &'a str,
    pub width: u32,
    pub height: u32,
    pub fps_num: i32,
    pub fps_den: i32,
    /// Bits a second, or `None` for the encoder's own choice.
    pub bit_rate: Option<i64>,
    /// The sound's rate, channels and bits a second, or `None` for no sound.
    pub sound: Option<(u32, u16, i64)>,
}

/// Where a file being written has got to.
pub enum State {
    /// It can take another frame.
    Room,
    /// It can't yet: the user is choosing where it goes, or the encoder is
    /// behind.
    Busy,
    Done,
    Failed(String),
}

/// A file the page is writing.
pub struct Page {
    id: u32,
}

fn why(_: JsValue) -> String {
    "the page could not write the file".into()
}

impl Page {
    pub fn open(settings: &Settings) -> Result<Self, String> {
        let (sound_rate, sound_channels, sound_bit_rate) = settings.sound.unwrap_or_default();
        let id = page_open(
            settings.name,
            settings.codec,
            settings.width,
            settings.height,
            settings.fps_num,
            settings.fps_den,
            settings.bit_rate.unwrap_or(0) as f64,
            sound_rate,
            u32::from(sound_channels),
            sound_bit_rate as f64,
        )
        .map_err(why)?;
        Ok(Self { id })
    }

    pub fn state(&self) -> State {
        match page_state(self.id).map_err(why) {
            Ok(state) if state == "room" => State::Room,
            Ok(state) if state == "busy" => State::Busy,
            Ok(state) if state == "done" => State::Done,
            Ok(error) | Err(error) => State::Failed(error),
        }
    }

    /// Frame `n`, as RGBA at the size the file was opened with.
    pub fn frame(&self, rgba: &[u8], n: usize) -> Result<(), String> {
        page_frame(self.id, rgba, n).map_err(why)
    }

    /// Interleaved sound, starting `first` samples into the file.
    pub fn sound(&self, samples: &[f32], first: usize) -> Result<(), String> {
        page_sound(self.id, samples, first).map_err(why)
    }

    /// Everything is in: the page writes the end of the file.
    pub fn finish(&self) -> Result<(), String> {
        page_finish(self.id).map_err(why)
    }

    /// Stop, and leave no file.
    pub fn cancel(&self) {
        let _ = page_cancel(self.id);
    }
}
