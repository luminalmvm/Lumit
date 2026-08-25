Lumit is a native, Windows-first motion-graphics and compositing editor: a Rust + wgpu
engine with a Flutter frontend (decision K-174), GPLv3, specified docs-first. **The
documents in `docs/` are canonical**: when code and docs disagree, the docs win; when a doc
must change, change it in the same commit and - if it reverses a numbered decision - append
to `docs/02-DECISIONS.md` (never edit history). Start at `docs/README.md` for the index.

## Before doing anything

1. Read `docs/01-GLOSSARY.md` (short). Its terms are binding in code identifiers, UI strings,
comments, commits, and conversation. The banned-terms table (§9) is enforced - in
particular: *layer* not *track*, *speed* not *velocity*, *Retime* not *time remap*,
*export* not *render* (for user-facing output), *clip* only inside Sequence layers.
2. **Consult** `docs/02-DECISIONS.md` - do **not** read it end to end. It is a long,
append-only log; reading all of it wastes context and invites hallucination. Instead,
search it for the entries that touch your task (by topic keyword, or by the `K-###
numbers the relevant spec cites). DECIDED entries are locked; PROPOSED entries are strong
defaults; where two entries conflict, the later one that says it supersedes the earlier
wins. Don't silently contradict a decision, and when a change reverses one, append a new
superseding entry (never edit history).
3. For any code: `docs/14-ENGINEERING-RULES.md is the binding rulebook (typed rational
time, no panics in engine crates, no locks across await/GPU/FFI, budgeted allocations,
cancellation everywhere, determinism). docs/13-PERFORMANCE-RULES.md budgets gate merges.
4. If this `CLAUDE.md` file is ever changed, also update the corresponding `AGENTS.md` file.

## Repo shape

- `docs/README.md` - the documentation index; read it first. It explains the three kinds
of doc (durable spec / living state / frozen history) and which file covers what.
`docs/ - the specification set (00-17). docs/17-BRIDGE-CONTRACT.md` is the single
source of truth for the Flutter/Rust front/back boundary; read it before touching that
seam. `docs/research/` is the background research (not canonical).
`docs/TODO.md` - the living work backlog (Now / Next / Later). Delete an item when it
lands; its regression test is the record.
`docs/impl/` - **read the matching note before implementing anything it covers** (rational
time, keyframe/Retime cubic solving, wgpu foundation, media I/O and hardware decode,
playback scheduler, optical flow, OFX hosting, beat detection, expressions). They pin the
algorithms, formulas, traps, and test plans so those choices are not re-derived; specs
say *what*, these notes are the authoritative *how* for their topics. Implement each
note's test plan alongside the feature.
`docs/archive/` - frozen, dated material (audits, superseded ledgers, the egui-to-Flutter
port notes). Read-only history; never update it.
The engine is a Cargo workspace under `crates/` per `docs/05-ARCHITECTURE.md` (engine
crates never depend on the UI or the bridge). The Flutter frontend is under `flutter_ui/`
and talks to the engine through `crates/lumit-bridge` (`docs/17-BRIDGE-CONTRACT.md`).
`web/` (lumitlab.com) and `web-docs/` (docs.lumitlab.com) are the public site — two small
Astro projects, outside the Cargo workspace and depended on by nothing (K-279).

## Design

Lumit follows the household Aizome design language in its **dark-first** adaptation —
`docs/15-DESIGN.md`, a recorded deviation (K-004) from the paper-light default in the
household `HOUSEHOLD-DESIGN.md`. All colours come from the theme struct; hex literals in
widget code are a defect. Voice: British English, sentence case, calm, no exclamation
marks, no emoji, no punishment UI.

## Strings, translations and the engine's words (binding, K-005, K-303)

**Every user-facing string goes through `flutter_ui/lib/l10n/app_en.arb`** — the Crowdin
source file — and is read as `l10n.someKey`, never written inline in a widget. If a change
adds, renames or reworks a string, the arb entry lands **in the same commit**, with its
`@key` description saying where the string appears (a translator sees the phrase, not the
screen). `flutter pub get` regenerates the Dart from it.

Two things are easy to miss. The first fails CI; the second is worse, because nothing
catches it:

- **A string the *engine* can send needs two entries, not one.** Keymap action
  descriptions, effect and category labels, and anything else Rust hands over as English
  text must be added to `flutter_ui/lib/l10n/engine_labels.dart` **and** given a matching
  `app_en.arb` key. `engine_labels_test.dart` walks the engine's own tables and fails on
  any label with no entry — that is the gate, and it is easy to trip by adding a keymap
  action without thinking of it as "a string".
- **The other `app_*.arb` files are Crowdin's, not ours.** Never hand-edit a translation.
  Adding an English key leaves the other languages short, which is expected and handled
  (a missing translation falls back to English) — but it is not silent: **say so in the
  commit message and in the pull request**, listing the new keys, so the Crowdin upload
  is not forgotten. A shipped string nobody was told about is a string that stays English
  in every other language for a release.

## Readability and coverage (binding, K-007)

- **The docs and code must stay understandable to the project owner**, who knows editing
  software deeply but has never written Rust and hasn't worked with threads/GPUs.
  `docs/GUIDE.md` is the plain-English companion: whenever a new concept, crate, or
  mechanism enters the codebase, add a plain-English section for it to GUIDE.md **in the
  same commit**. New impl notes and complex modules open with a short "in plain terms"
  framing. Never assume Rust fluency in any doc outside code comments.
- **Near-full regression coverage is standing policy**: every feature lands with its tests;
  every bug fix lands with a regression test that fails without the fix; CI runs fmt,
  clippy (warnings are errors), the full suite on macOS + Windows, the engine-crate
  coverage gate (threshold rises only), and the no-hex-outside-theme lint. A red CI blocks
  everything else.

## Working style

- This is a public repo: nothing personal or machine-specific in committed files.
- Specs end with `## Open questions` — resolving one means editing the doc and, where it is
  decision-sized, logging it in 02-DECISIONS.
- Verification beats assertion: performance and never-crash claims in these docs become CI
  gates (`docs/16-ROADMAP.md` standing rules) — treat them as tests to write, not slogans.
