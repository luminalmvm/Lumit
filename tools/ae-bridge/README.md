# Lumit Bridge

The After Effects half of AE import: a script that walks the open project and writes a
**bundle folder** of JSON. It converts nothing — AE's ids, AE's float seconds, AE's match
names, recorded verbatim (K-410). All the translating happens later, in Rust.

## Export a project

1. Open the project in After Effects (any version 2024+).
2. If scripts cannot write files yet: Edit → Preferences → Scripting & Expressions →
   tick "Allow Scripts to Write Files and Access Network".
3. File → Scripts → Run Script File… → pick `lumit-bridge.jsx` from this folder.
4. Choose where the bundle goes (it offers the project's own folder).
5. Wait for the alert. It writes `<ProjectName>.lum-bundle/` there:

```
MyProject.lum-bundle/
  manifest.json    bundle version, AE version, Bridge version, export date
  capture.json     the walk: items, comps, layers, properties, keyframes
  report.json      properties the DOM would not read, with AE's own error text
```

Footage is not collected in v1: the capture records file paths, and Lumit relinks on
import. Nothing is modified in your project.

Some unreadable entries are expected and not a fault: AE's own scripting DOM cannot read
`CUSTOM_VALUE` data, so Curves' point list, Levels' histogram, Hue/Saturation's channel
ranges and a shape layer's gradient stops arrive as unreadables. The report says so
rather than guessing.

## Build the test fixture

`make-fixture.jsx` builds one deterministic project covering every row of the coverage
checklist in `docs/impl/ae-import.md` §5 — nested comps, the keyframe variety, both
generations of matte, masks, markers, retiming, text, shapes, a camera and a light,
expressions, and the effect spread including the unreadable one and one match name Lumit
does not ship. Its bundle **is** the Rust importer's golden fixture:
`fixtures/fixture.lum-bundle/` was built on After Effects 26.0 on 2026-08-20 and is what
`crates/lumit-import/tests/golden.rs` reads. Running the builder again rewrites it, so
do that only when the checklist itself changes — and expect the golden test's exact
counts to need updating alongside.

1. Save and close whatever you are working on — this builds a **new** project.
2. Same file-write preference as above.
3. File → Scripts → Run Script File… → pick `make-fixture.jsx`.
4. It writes `fixtures/fixture.aep` and `fixtures/fixture.lum-bundle/`, then reports any
   checklist row that did not apply.

Two things are not fixed by the fixture and should not be asserted on: the manifest's
export date, and the captured font name of the text layer (the script deliberately does
not name a font, so AE's default — whatever is installed — comes through).

## Notes for editing the scripts

ExtendScript is ES3: no `JSON` object, no `const`, no `Array.prototype.map`, no trailing
commas. `lumit-bridge.jsx` carries its own JSON writer, the same escaper
`tools/ae-audit/audit.jsx` uses. AE collections are 1-based. Every property read is
wrapped: a failure becomes a `report.json` row and the walk continues, because one broken
property must never abort an export.

Enum values go into the capture as **AE's own constant names, verbatim** — `SCREEN`,
`ALPHA_INVERTED`, `BEZIER`, `PIXEL_MOTION`. Do not lower-case or tidy them: re-spelling
is a conversion, conversions belong on the Rust side where tests can watch them, and the
Rust reader matches on these strings exactly (`docs/impl/ae-import.md` §2).
