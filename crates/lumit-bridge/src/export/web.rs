/// The extension of a format only the page writes. Its WebM codecs make a
/// picture the way H.264 does (eight bits, no alpha, a bitrate), so the
/// engine knows them as that.
pub(super) fn extension(codec: &str) -> Option<&'static str> {
    matches!(codec, "vp9" | "av1").then_some("webm")
}

pub(super) fn offers(codec: &str) -> bool {
    lumit_media::web::export::codecs()
        .iter()
        .any(|c| c == codec)
}

pub(super) fn start(
    doc: std::sync::Arc<lumit_core::model::Document>,
    comp: uuid::Uuid,
    audio: &[lumit_render::mix::jobs::AudioJob],
    out_path: &str,
    spec: lumit_render::export::ExportSpec,
    codec: &str,
) -> Result<lumit_render::export::ExportHandle, String> {
    let (handle, mut export) =
        lumit_render::export::web::start(doc, comp, audio, out_path.into(), spec, codec)?;
    crate::turns::start(move || match export.step() {
        Some(wait) => {
            crate::turns::pause(wait);
            true
        }
        None => false,
    });
    Ok(handle)
}
