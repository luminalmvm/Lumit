//! Subtitle streams: captions carried inside a video file as text.
//!
//! # In plain terms
//!
//! An mp4 or a mov can hold timed text beside its picture and sound, which
//! players show as subtitles that can be switched on and off. Each caption is
//! encoded as `mov_text` and written to its own stream as its time comes,
//! with the stream's language set so a player can name it.
//!
//! # Thread role
//!
//! Whichever thread writes the file, which for an export is its writer.

use std::ffi::CString;
use std::path::Path;

use rsmpeg::avcodec::{AVCodec, AVCodecContext, AVPacket};
use rsmpeg::avformat::AVFormatContextOutput;
use rsmpeg::avutil::{AVDictionary, AVRational};
use rsmpeg::ffi;

use super::{
    at, dict_set, open_audio, open_video, set_container_metadata, AudioSettings, Encoder, Metadata,
    VideoSettings,
};
use crate::MediaError;

/// Subtitle times are counted in milliseconds.
const MILLISECONDS: AVRational = AVRational { num: 1, den: 1000 };

/// One subtitle stream: its `mov_text` encoder and where it sits in the file.
pub(super) struct SubtitleStream {
    ctx: AVCodecContext,
    stream_index: usize,
}

impl Encoder {
    /// As [`Encoder::open`], with a subtitle stream after the picture and
    /// sound for each of `languages`, each named by its language code (`en`
    /// or `eng`; anything else is written as undetermined). Only the mp4 and
    /// mov family can carry one.
    pub fn open_with_subtitles(
        path: &Path,
        video: Option<&VideoSettings>,
        audio: Option<&AudioSettings>,
        languages: &[String],
        metadata: &Metadata,
    ) -> Result<Self, MediaError> {
        open(path, video, audio, 0, languages, metadata)
    }

    /// Write one caption to subtitle stream `stream`, counted from nought:
    /// `lines` shown from `start_ms` to `end_ms` on the file's clock. Captions
    /// go to a stream in order of time.
    pub fn write_subtitle(
        &mut self,
        stream: usize,
        start_ms: i64,
        end_ms: i64,
        lines: &[String],
    ) -> Result<(), MediaError> {
        let Self {
            output, subtitles, ..
        } = self;
        let sub = subtitles
            .get_mut(stream)
            .ok_or_else(|| MediaError::Ffmpeg("this file has no such subtitle stream".into()))?;
        let bytes = encode(&mut sub.ctx, &dialogue(lines))?;
        let mut packet = AVPacket::new();
        let size = i32::try_from(bytes.len())
            .map_err(|_| MediaError::Ffmpeg("a caption is too long".into()))?;
        // SAFETY: the packet was just allocated and is ours alone.
        // `av_new_packet` gives it a buffer of `size` bytes, checked below
        // before the copy, which writes exactly that many from `bytes`.
        #[allow(unsafe_code)]
        unsafe {
            let raw = packet.as_mut_ptr();
            if ffi::av_new_packet(raw, size) < 0 {
                return Err(MediaError::Ffmpeg("no memory for a caption".into()));
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*raw).data, bytes.len());
        }
        packet.set_pts(start_ms);
        packet.set_dts(start_ms);
        packet.set_duration(end_ms.saturating_sub(start_ms).max(1));
        packet.set_stream_index(
            i32::try_from(sub.stream_index)
                .map_err(|_| MediaError::Ffmpeg("stream index overflows".into()))?,
        );
        let stream_tb = output
            .streams()
            .get(sub.stream_index)
            .map(|s| s.time_base)
            .ok_or_else(|| MediaError::Ffmpeg("output stream missing".into()))?;
        packet.rescale_ts(MILLISECONDS, stream_tb);
        output
            .interleaved_write_frame(&mut packet)
            .map_err(at("writing a caption"))
    }
}

/// Open a container with every stream it is asked for, and write its header.
pub(super) fn open(
    path: &Path,
    video: Option<&VideoSettings>,
    audio: Option<&AudioSettings>,
    more_sound: usize,
    languages: &[String],
    metadata: &Metadata,
) -> Result<Encoder, MediaError> {
    if video.is_none() && audio.is_none() {
        return Err(MediaError::Ffmpeg(
            "an export must carry a video or an audio stream".into(),
        ));
    }
    let cpath =
        CString::new(path.to_str().ok_or(MediaError::BadPath)?).map_err(|_| MediaError::BadPath)?;
    let mut output = AVFormatContextOutput::create(&cpath).map_err(at("opening the file"))?;
    let global_header = (output.oformat().flags & ffi::AVFMT_GLOBALHEADER as i32) != 0;

    let video_track = match video {
        Some(v) => Some(open_video(&mut output, v, global_header)?),
        None => None,
    };
    let audio_track = match audio {
        Some(a) => Some(open_audio(&mut output, a, global_header)?),
        None => None,
    };
    let mut more_audio = Vec::new();
    if let Some(a) = audio {
        for _ in 0..more_sound {
            more_audio.push(open_audio(&mut output, a, global_header)?);
        }
    }
    let subtitles = languages
        .iter()
        .map(|language| open_subtitle(&mut output, language, global_header))
        .collect::<Result<Vec<_>, _>>()?;

    set_container_metadata(&mut output, metadata);

    // +faststart moves the index to the front of the file when the
    // trailer is written, so exports stream straight away (media-io §7).
    // Only the mp4/mov family understands it — which is exactly the family
    // that wants a global header — and a stray option would sit unread in
    // the dictionary rather than being an error, so the guard is honesty
    // rather than necessity.
    let mut header_opts = global_header
        .then(|| dict_set(None, "movflags", "+faststart"))
        .flatten();
    output
        .write_header(&mut header_opts)
        .map_err(at("writing the header"))?;

    Ok(Encoder {
        output,
        video: video_track,
        audio: audio_track,
        more_audio,
        subtitles,
        finished: false,
    })
}

/// Open a `mov_text` encoder and add its stream, in `language`.
fn open_subtitle(
    output: &mut AVFormatContextOutput,
    language: &str,
    global_header: bool,
) -> Result<SubtitleStream, MediaError> {
    let codec = AVCodec::find_encoder(ffi::AV_CODEC_ID_MOV_TEXT)
        .ok_or_else(|| MediaError::Ffmpeg("no mov_text encoder linked".into()))?;
    let mut ctx = AVCodecContext::new(&codec);
    ctx.set_time_base(MILLISECONDS);
    set_header(&mut ctx)?;
    if global_header {
        ctx.set_flags(ctx.flags | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
    }
    ctx.open(None).map_err(at("opening the subtitle encoder"))?;
    let mut stream = output.new_stream();
    stream.set_codecpar(ctx.extract_codecpar());
    stream.set_time_base(MILLISECONDS);
    let code = iso639_2(language).unwrap_or("und");
    stream.set_metadata(
        CString::new(code)
            .ok()
            .map(|code| AVDictionary::new(c"language", &code, 0)),
    );
    let stream_index = usize::try_from(stream.index).unwrap_or(0);
    Ok(SubtitleStream { ctx, stream_index })
}

/// The styling the encoder starts from, which it needs to be told: one
/// plain style that every caption uses.
const HEADER: &str = concat!(
    "[Script Info]\nScriptType: v4.00+\nPlayResX: 384\nPlayResY: 288\n\n",
    "[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, ",
    "OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, ",
    "Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n",
    "Style: Default,Arial,16,&Hffffff,&Hffffff,&H0,&H0,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1\n\n",
    "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
);

/// Give `ctx` its [`HEADER`], in memory of FFmpeg's own that the context
/// frees with itself.
fn set_header(ctx: &mut AVCodecContext) -> Result<(), MediaError> {
    let len = HEADER.len();
    let size = i32::try_from(len).map_err(|_| MediaError::Ffmpeg("header too long".into()))?;
    // SAFETY: the context is ours alone and not yet opened. The buffer is
    // allocated one byte longer than the header, filled with it and a NUL,
    // and handed to the context, which frees it in `avcodec_free_context`.
    #[allow(unsafe_code)]
    unsafe {
        let buf = ffi::av_mallocz(len + 1).cast::<u8>();
        if buf.is_null() {
            return Err(MediaError::Ffmpeg(
                "no memory for the subtitle header".into(),
            ));
        }
        std::ptr::copy_nonoverlapping(HEADER.as_ptr(), buf, len);
        let raw = ctx.as_mut_ptr();
        (*raw).subtitle_header = buf;
        (*raw).subtitle_header_size = size;
    }
    Ok(())
}

/// A caption's lines as the dialogue text the encoder reads, with every
/// field before the text left at its default.
///
/// The encoder reads that text as styled: anything in braces is taken for
/// styling and dropped, and a backslash before an `n` for a line break. There
/// is no way to escape either, so braces go in as brackets, and a backslash
/// is kept apart from the letter after it by a mark with no width.
fn dialogue(lines: &[String]) -> String {
    let plain = |line: &String| {
        line.replace('{', "(")
            .replace('}', ")")
            .replace('\\', "\\\u{2060}")
    };
    let lines: Vec<String> = lines.iter().map(plain).collect();
    format!("0,0,Default,,0,0,0,,{}", lines.join("\\N"))
}

/// Encode one caption's dialogue text as a `mov_text` sample.
fn encode(ctx: &mut AVCodecContext, text: &str) -> Result<Vec<u8>, MediaError> {
    let text = CString::new(text.replace('\0', ""))
        .map_err(|_| MediaError::Ffmpeg("a caption is not text".into()))?;
    // Two bytes of length, the text, and room for any styling the encoder adds.
    let mut buf = vec![0u8; 1024 + text.as_bytes().len() * 2];
    let size =
        i32::try_from(buf.len()).map_err(|_| MediaError::Ffmpeg("a caption is too long".into()))?;
    let sub = Subtitle::new(&text)?;
    // SAFETY: `ctx` is an opened subtitle encoder we own alone, `buf` is
    // writable for `size` bytes, and `sub` holds one well-formed text rect
    // allocated by FFmpeg's own allocator, freed when it drops.
    #[allow(unsafe_code)]
    let written =
        unsafe { ffi::avcodec_encode_subtitle(ctx.as_mut_ptr(), buf.as_mut_ptr(), size, &sub.raw) };
    drop(sub);
    let written = usize::try_from(written)
        .map_err(|_| MediaError::Ffmpeg(format!("encoding a caption failed ({written})")))?;
    buf.truncate(written);
    Ok(buf)
}

/// An `AVSubtitle` holding one text rect, its memory FFmpeg's, freed with
/// `avsubtitle_free` when it drops.
struct Subtitle {
    raw: ffi::AVSubtitle,
}

impl Subtitle {
    fn new(text: &std::ffi::CStr) -> Result<Self, MediaError> {
        let mut raw = ffi::AVSubtitle {
            format: 1,
            start_display_time: 0,
            end_display_time: 0,
            num_rects: 0,
            rects: std::ptr::null_mut(),
            pts: 0,
        };
        let failed = || MediaError::Ffmpeg("no memory for a caption".into());
        // SAFETY: every pointer stored comes from FFmpeg's allocator and is
        // checked for null before use. `num_rects` is raised only once the
        // rect it counts is in place, so `avsubtitle_free` frees exactly
        // what was made, whichever step fails.
        #[allow(unsafe_code)]
        unsafe {
            let rects = ffi::av_mallocz(std::mem::size_of::<*mut ffi::AVSubtitleRect>())
                .cast::<*mut ffi::AVSubtitleRect>();
            if rects.is_null() {
                return Err(failed());
            }
            raw.rects = rects;
            let mut made = Self { raw };
            let rect = ffi::av_mallocz(std::mem::size_of::<ffi::AVSubtitleRect>())
                .cast::<ffi::AVSubtitleRect>();
            if rect.is_null() {
                return Err(failed());
            }
            *made.raw.rects = rect;
            made.raw.num_rects = 1;
            (*rect).type_ = ffi::SUBTITLE_ASS;
            (*rect).ass = ffi::av_strdup(text.as_ptr());
            if (*rect).ass.is_null() {
                return Err(failed());
            }
            Ok(made)
        }
    }
}

impl Drop for Subtitle {
    fn drop(&mut self) {
        // SAFETY: the rects and their text were allocated by FFmpeg in
        // `Subtitle::new`, and nothing else holds them.
        #[allow(unsafe_code)]
        unsafe {
            ffi::avsubtitle_free(&mut self.raw);
        }
    }
}

/// The three-letter code an mp4 or a mov names a language by, from the
/// two-letter one captions usually carry: the code the language has in its
/// own tongue (`deu`, `fra`), which is the one an mp4 asks for. A
/// three-letter code is taken as it is, its English form (`ger`, `fre`) too,
/// and a region after it, as in `en-GB`, is let go.
fn iso639_2(language: &str) -> Option<&'static str> {
    const CODES: &str = "af:afr am:amh ar:ara as:asm az:aze ba:bak be:bel bg:bul bn:ben \
        bo:bod br:bre bs:bos ca:cat cs:ces cy:cym da:dan de:deu el:ell en:eng es:spa et:est \
        eu:eus fa:fas fi:fin fo:fao fr:fra ga:gle gl:glg gu:guj ha:hau he:heb hi:hin hr:hrv \
        ht:hat hu:hun hy:hye id:ind is:isl it:ita ja:jpn jv:jav jw:jav ka:kat kk:kaz km:khm \
        kn:kan ko:kor la:lat lb:ltz ln:lin lo:lao lt:lit lv:lav mg:mlg mi:mri mk:mkd ml:mal \
        mn:mon mr:mar ms:msa mt:mlt my:mya nb:nob ne:nep nl:nld nn:nno no:nor oc:oci pa:pan \
        pl:pol ps:pus pt:por ro:ron ru:rus sa:san sd:snd si:sin sk:slk sl:slv sn:sna so:som \
        sq:sqi sr:srp su:sun sv:swe sw:swa ta:tam te:tel tg:tgk th:tha tk:tuk tl:tgl tr:tur \
        tt:tat uk:ukr ur:urd uz:uzb vi:vie yi:yid yo:yor zh:zho";
    // The twenty languages with an English form as well.
    const ENGLISH: &str = "tib:bo cze:cs wel:cy ger:de gre:el baq:eu per:fa fre:fr arm:hy \
        ice:is geo:ka mao:mi mac:mk may:ms bur:my dut:nl rum:ro slo:sk alb:sq chi:zh";
    let code = language.split(['-', '_']).next()?.to_ascii_lowercase();
    let code = ENGLISH
        .split_whitespace()
        .filter_map(|pair| pair.split_once(':'))
        .find_map(|(english, two)| (english == code).then_some(two))
        .unwrap_or(code.as_str());
    CODES.split_whitespace().find_map(|pair| {
        let (two, three) = pair.split_once(':')?;
        (two == code || three == code).then_some(three)
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_caption_comes_back_out_of_the_file_it_was_embedded_in() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("captions.mp4");
        let video = VideoSettings {
            codec: super::super::VideoCodec::H264,
            width: 64,
            height: 64,
            fps_num: 25,
            fps_den: 1,
            bit_rate: None,
            max_rate: None,
            colour: super::super::ColourTags::Srgb,
            depth: super::super::BitDepth::Eight,
        };
        let mut encoder = Encoder::open_with_subtitles(
            &path,
            Some(&video),
            None,
            &["en".to_owned()],
            &Metadata::new(),
        )
        .unwrap();
        let frame = vec![128u8; 64 * 64 * 4];
        for n in 0..25 {
            encoder.write_rgba(&frame).unwrap();
            if n == 5 {
                encoder
                    .write_subtitle(0, 200, 800, &["Hello {you}".into(), r"C:\new".into()])
                    .unwrap();
            }
        }
        encoder.finish().unwrap();

        let cpath = CString::new(path.to_str().unwrap()).unwrap();
        let mut input = rsmpeg::avformat::AVFormatContextInput::open(&cpath).unwrap();
        let (index, stream) = input
            .streams()
            .iter()
            .enumerate()
            .find(|(_, s)| s.codecpar().codec_id == ffi::AV_CODEC_ID_MOV_TEXT)
            .expect("a subtitle stream");
        let language = stream.metadata().and_then(|m| {
            m.get(c"language", None, 0)
                .map(|v| v.value().to_string_lossy().into_owned())
        });
        assert_eq!(language.as_deref(), Some("eng"));
        let tb = stream.time_base;
        let mut found = None;
        while let Some(packet) = input.read_packet().unwrap() {
            if packet.stream_index as usize == index && packet.size > 2 {
                // SAFETY: the packet holds `size` bytes.
                #[allow(unsafe_code)]
                let bytes =
                    unsafe { std::slice::from_raw_parts(packet.data, packet.size as usize) };
                let ms = |t: i64| t * 1000 * i64::from(tb.num) / i64::from(tb.den);
                // A sample is the text's length in two bytes, then the text.
                let len = usize::from(u16::from_be_bytes([bytes[0], bytes[1]]));
                found = Some((
                    ms(packet.pts),
                    ms(packet.duration),
                    String::from_utf8_lossy(&bytes[2..2 + len]).into_owned(),
                ));
            }
        }
        // Braces and a backslash before an `n` are words, not styling.
        assert_eq!(
            found,
            Some((200, 600, "Hello (you)\nC:\\\u{2060}new".to_owned()))
        );
        assert_eq!(iso639_2("de-AT"), Some("deu"));
        assert_eq!(iso639_2("ger"), Some("deu"));
    }
}
