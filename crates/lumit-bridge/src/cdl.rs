//! ASC CDL files: reading a grade out of a `.cdl`, `.cc` or `.ccc`, and
//! writing one as a `.cdl` or `.cc`.
//!
//! All three hold the same `ColorCorrection` block, a `SOPNode` with Slope,
//! Offset and Power and a `SatNode` with Saturation, under different
//! wrappers: a `.cc` is one block, a `.ccc` a collection, a `.cdl` a decision
//! list. A missing node reads as the identity, as the list says.

/// One `ColorCorrection`'s numbers.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CdlValues {
    pub id: Option<String>,
    pub slope: [f32; 3],
    pub offset: [f32; 3],
    pub power: [f32; 3],
    pub saturation: f32,
}

impl CdlValues {
    pub(crate) fn identity(id: Option<String>) -> Self {
        Self {
            id,
            slope: [1.0; 3],
            offset: [0.0; 3],
            power: [1.0; 3],
            saturation: 1.0,
        }
    }
}

/// Element names compared as the files in the wild write them, `SatNode` and
/// `SATNode` alike.
fn named(n: &roxmltree::Node<'_, '_>, name: &str) -> bool {
    n.is_element() && n.tag_name().name().eq_ignore_ascii_case(name)
}

fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| named(c, name))
}

/// Three numbers from `n`'s child `name`, or `fallback` when it is missing.
/// `None` when it is there but isn't three numbers.
fn triple(n: roxmltree::Node<'_, '_>, name: &str, fallback: [f32; 3]) -> Option<[f32; 3]> {
    let Some(text) = child(n, name).map(|c| c.text().unwrap_or("")) else {
        return Some(fallback);
    };
    let v: Vec<f32> = text
        .split_whitespace()
        .map(str::parse::<f32>)
        .collect::<Result<_, _>>()
        .ok()?;
    match v.as_slice() {
        [r, g, b] if v.iter().all(|x| x.is_finite()) => Some([*r, *g, *b]),
        _ => None,
    }
}

/// The `ColorCorrection` called `id` in `text`, or the first when `id` is
/// `None`. `None` when the text isn't XML, holds none, or its numbers don't
/// read.
pub(crate) fn parse(text: &str, id: Option<&str>) -> Option<CdlValues> {
    let doc = roxmltree::Document::parse(text).ok()?;
    let cc = doc
        .descendants()
        .filter(|n| named(n, "ColorCorrection"))
        .find(|n| id.is_none_or(|want| n.attribute("id") == Some(want)))?;
    let mut out = CdlValues::identity(cc.attribute("id").map(str::to_owned));
    if let Some(sop) = child(cc, "SOPNode") {
        out.slope = triple(sop, "Slope", out.slope)?;
        out.offset = triple(sop, "Offset", out.offset)?;
        out.power = triple(sop, "Power", out.power)?;
    }
    if let Some(sat) = child(cc, "SatNode") {
        if let Some(s) = child(sat, "Saturation") {
            out.saturation = s
                .text()
                .unwrap_or("")
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())?;
        }
    }
    Some(out)
}

/// Every `ColorCorrection`'s id in `text`, blank for one without, or `None`
/// when the text isn't XML.
pub(crate) fn ids(text: &str) -> Option<Vec<String>> {
    let doc = roxmltree::Document::parse(text).ok()?;
    Some(
        doc.descendants()
            .filter(|n| named(n, "ColorCorrection"))
            .map(|n| n.attribute("id").unwrap_or("").to_owned())
            .collect(),
    )
}

/// `s` safe inside an XML attribute.
fn escaped(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `v` as a `.cc` (`decision_list` false) or a `.cdl`.
pub(crate) fn write(v: &CdlValues, decision_list: bool) -> String {
    let three = |a: [f32; 3]| format!("{:.6} {:.6} {:.6}", a[0], a[1], a[2]);
    let id =
        v.id.as_deref()
            .map(|s| format!(" id=\"{}\"", escaped(s)))
            .unwrap_or_default();
    let pad = if decision_list { "    " } else { "" };
    let ns = " xmlns=\"urn:ASC:CDL:v1.01\"";
    let block = format!(
        "{pad}<ColorCorrection{id}{ns_cc}>\n\
         {pad}  <SOPNode>\n\
         {pad}    <Slope>{}</Slope>\n\
         {pad}    <Offset>{}</Offset>\n\
         {pad}    <Power>{}</Power>\n\
         {pad}  </SOPNode>\n\
         {pad}  <SatNode>\n\
         {pad}    <Saturation>{:.6}</Saturation>\n\
         {pad}  </SatNode>\n\
         {pad}</ColorCorrection>\n",
        three(v.slope),
        three(v.offset),
        three(v.power),
        v.saturation,
        ns_cc = if decision_list { "" } else { ns },
    );
    let head = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
    if decision_list {
        format!(
            "{head}<ColorDecisionList{ns}>\n  <ColorDecision>\n{block}  </ColorDecision>\n</ColorDecisionList>\n"
        )
    } else {
        format!("{head}{block}")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A grade written out reads back as it went, from either wrapper, and a
    /// collection gives up the block named.
    #[test]
    fn a_cdl_reads_back_as_written() {
        let v = CdlValues {
            id: Some("shot <1>".to_owned()),
            slope: [1.1, 0.95, 1.02],
            offset: [0.01, -0.02, 0.0],
            power: [0.9, 1.0, 1.1],
            saturation: 0.85,
        };
        for list in [false, true] {
            assert_eq!(parse(&write(&v, list), None), Some(v.clone()));
        }
        let ccc = "<ColorCorrectionCollection xmlns=\"urn:ASC:CDL:v1.01\">\
            <ColorCorrection id=\"a\"><SOPNode><Slope>2 2 2</Slope></SOPNode></ColorCorrection>\
            <ColorCorrection id=\"b\"><SATNode><Saturation>0.5</Saturation></SATNode></ColorCorrection>\
            </ColorCorrectionCollection>";
        let b = parse(ccc, Some("b")).unwrap();
        assert_eq!((b.slope, b.saturation), ([1.0; 3], 0.5));
        assert_eq!(parse(ccc, None).unwrap().slope, [2.0; 3]);
        assert_eq!(parse(ccc, Some("c")), None);
    }
}
