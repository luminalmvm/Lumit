//! The browser build's side of everything that needs FFmpeg. The page decodes
//! video and sound itself, so here opening a decoder, a reader or an encoder
//! answers an error, and the methods of one can never be reached.

use std::path::Path;

use crate::audio::AudioReader;
use crate::decode::{DecodedFrame, LumaFrame, VideoDecoder};
use crate::encode::{
    AudioSettings, BitDepth, ColourTags, Encoder, GifEncoder, GifOptions, ImageFormat,
    ImageSequenceEncoder, Metadata, VideoSettings,
};
use crate::index::FrameIndex;
use crate::sequence::MediaSource;
use crate::{no_ffmpeg, MediaError};

impl VideoDecoder {
    pub fn open(_src: impl Into<MediaSource>, _index: FrameIndex) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn open_with(
        _src: impl Into<MediaSource>,
        _index: FrameIndex,
        _allow_hardware: bool,
    ) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn frame_count(&self) -> usize {
        match self.0 {}
    }

    pub fn seeks(&self) -> usize {
        match self.0 {}
    }

    pub fn is_hardware(&self) -> bool {
        match self.0 {}
    }

    pub fn frame_rgba(
        &mut self,
        _n: usize,
        _target_width: Option<u32>,
    ) -> Result<DecodedFrame, MediaError> {
        match self.0 {}
    }

    pub fn frame_luma(&mut self, _n: usize) -> Result<LumaFrame, MediaError> {
        match self.0 {}
    }
}

impl AudioReader {
    pub fn open(_path: &Path, _target_rate: u32) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn open_stream(_path: &Path, _stream: u32, _target_rate: u32) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    #[must_use]
    pub fn lands(&self) -> bool {
        match self.0 {}
    }

    #[must_use]
    pub fn position(&self) -> u64 {
        match self.0 {}
    }

    #[must_use]
    pub fn reaches(&self, _start: u64, _exact: bool) -> Option<u64> {
        match self.0 {}
    }

    #[must_use]
    pub fn total(&self) -> Option<u64> {
        match self.0 {}
    }

    pub fn set_total(&mut self, _total: u64) {
        match self.0 {}
    }

    pub fn read(
        &mut self,
        _start: u64,
        _frames: usize,
        _exact: bool,
    ) -> Result<(Vec<f32>, bool), MediaError> {
        match self.0 {}
    }

    pub fn length(&mut self) -> Result<u64, MediaError> {
        match self.0 {}
    }
}

impl ImageSequenceEncoder {
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        _path: &Path,
        _format: ImageFormat,
        _width: u32,
        _height: u32,
        _fps_num: i32,
        _fps_den: i32,
        _depth: BitDepth,
        _colour: ColourTags,
    ) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn write_rgba(&mut self, _rgba: &[u8]) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn finish(&mut self) -> Result<(), MediaError> {
        match self.0 {}
    }
}

impl GifEncoder {
    pub fn open(
        _path: &Path,
        _width: u32,
        _height: u32,
        _fps_num: i32,
        _fps_den: i32,
        _options: GifOptions,
    ) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn write_rgba(&mut self, _rgba: &[u8]) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn finish(&mut self) -> Result<(), MediaError> {
        match self.0 {}
    }
}

impl Encoder {
    pub fn open(
        _path: &Path,
        _video: Option<&VideoSettings>,
        _audio: Option<&AudioSettings>,
        _metadata: &Metadata,
    ) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn open_with_sound(
        _path: &Path,
        _video: Option<&VideoSettings>,
        _audio: Option<&AudioSettings>,
        _more_sound: usize,
        _metadata: &Metadata,
    ) -> Result<Self, MediaError> {
        Err(no_ffmpeg())
    }

    pub fn encoder_name(&self) -> &'static str {
        match self.0 {}
    }

    pub fn encoder_label(&self) -> &'static str {
        match self.0 {}
    }

    pub fn has_video(&self) -> bool {
        match self.0 {}
    }

    pub fn write_rgba(&mut self, _rgba: &[u8]) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn write_audio(&mut self, _interleaved: &[f32]) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn write_audio_stream(
        &mut self,
        _stream: usize,
        _interleaved: &[f32],
    ) -> Result<(), MediaError> {
        match self.0 {}
    }

    pub fn finish(&mut self) -> Result<(), MediaError> {
        match self.0 {}
    }
}
