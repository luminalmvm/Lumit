# Luminal — instructions for AI-assisted work in this repo

Luminal is a native, Windows-first motion-graphics and compositing editor (Rust + wgpu +
egui, GPLv3), specified docs-first. **The documents in `docs/` are canonical**: when code
and docs disagree, the docs win; when a doc must change, change it in the same commit and —
if it reverses a numbered decision — append to `docs/02-DECISIONS.md` (never edit history).

## Before doing anything

1. Read `docs/01-GLOSSARY.md`. Its terms are binding in code identifiers, UI strings,
   comments, commits, and conversation. The banned-terms table (§9) is enforced — in
   particular: *layer* not *track*, *speed* not *velocity*, *Retime* not *time remap*,
   *export* not *render* (for user-facing output), *clip* only inside Sequence layers.
2. Read `docs/02-DECISIONS.md`. DECIDED entries are locked; PROPOSED entries are strong
   defaults — do not silently contradict either.
3. For any code: `docs/14-ENGINEERING-RULES.md` is the binding rulebook (typed rational
   time, no panics in engine crates, no locks across await/GPU/FFI, budgeted allocations,
   cancellation everywhere, determinism). `docs/13-PERFORMANCE-RULES.md` budgets gate merges.

## Repo shape

- `docs/` — the specification set (00–16, see README index) plus `docs/research/` (the web
  research that informed them; background, not canonical).
- `docs/impl/` — **read the matching note before implementing anything it covers** (rational
  time, keyframe/Retime cubic solving, wgpu foundation, media I/O and hardware decode,
  playback scheduler, optical flow, OFX hosting, beat detection, expressions). They pin the
  algorithms, formulas, traps, and test plans so those choices are not re-derived; specs
  say *what*, these notes are the authoritative *how* for their topics. Implement each
  note's test plan alongside the feature.
- Application code will be a Cargo workspace per `docs/05-ARCHITECTURE.md` (engine crates
  never depend on the UI crate).

## Design

Luminal follows the household Aizome design language in its **dark-first** adaptation —
`docs/15-DESIGN.md`, a recorded deviation (K-004) from the paper-light default in the
household `HOUSEHOLD-DESIGN.md`. All colours come from the theme struct; hex literals in
widget code are a defect. Voice: British English, sentence case, calm, no exclamation
marks, no emoji, no punishment UI.

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
