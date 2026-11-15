//! A transcript as a Word document: the three parts a `.docx` cannot open
//! without, and a paragraph to each of the transcript's, the speaker's name
//! in bold.

use std::io::{self, Write as _};

use zip::write::SimpleFileOptions;

use super::{escaped, heading, paragraphs, Clock, Said};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;

const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

/// One run of text, bold or italic when asked. The control characters XML
/// does not allow are left out, so the file still opens.
fn run(text: &str, bold: bool, italic: bool) -> String {
    let text: String = text
        .chars()
        .filter(|c| *c >= ' ' || matches!(c, '\t' | '\n' | '\r'))
        .filter(|c| !matches!(c, '\u{fffe}' | '\u{ffff}'))
        .collect();
    let look = match (bold, italic) {
        (true, _) => "<w:rPr><w:b/></w:rPr>",
        (false, true) => "<w:rPr><w:i/></w:rPr>",
        (false, false) => "",
    };
    format!(
        "<w:r>{look}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
        escaped(&text)
    )
}

/// The document's bytes.
pub(super) fn write(
    words: &[Said],
    speakers: bool,
    timecodes: Option<Clock>,
) -> io::Result<Vec<u8>> {
    let mut body = String::new();
    for p in paragraphs(words) {
        body.push_str("<w:p>");
        if let Some((name, time)) = heading(&p, speakers, timecodes) {
            if let Some(name) = name {
                body.push_str(&run(&format!("{name} "), true, false));
            }
            if let Some(time) = time {
                body.push_str(&run(&format!("{time} "), false, true));
            }
        }
        body.push_str(&run(&p.text, false, false));
        body.push_str("</w:p>");
    }
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}</w:body></w:document>"
    );
    let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, part) in [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", RELS),
        ("word/document.xml", document.as_str()),
    ] {
        zip.start_file(name, options).map_err(io::Error::other)?;
        zip.write_all(part.as_bytes())?;
    }
    Ok(zip.finish().map_err(io::Error::other)?.into_inner())
}
