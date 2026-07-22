# The Flutter frontend alternative — documentation set

This folder documents the experimental port of Lumit's frontend from egui to
Flutter (decision K-174). It is the working spec for the `flutter-frontend-alternative`
branch; the numbered docs (00–16) remain canonical for what the application *does* —
these notes only cover how the same interface is rebuilt on Flutter.

The goal of the first pass is a **one-for-one port**: everything the egui frontend
does today, reproduced in Flutter, before any of the interface's known rough edges
are redesigned. Fix-ups come after parity, so there is always a truthful baseline
to compare against.

| Doc | What it covers |
|---|---|
| [01-STRATEGY.md](01-STRATEGY.md) | Why, the ground rules, the phase plan, and how the port is verified |
| [02-UI-INVENTORY.md](02-UI-INVENTORY.md) | Every surface the egui frontend ships — panels, dialogs, chrome, shortcuts, persisted state — with source pointers |
| [03-ARCHITECTURE.md](03-ARCHITECTURE.md) | How Flutter talks to the Rust engine: the bridge, the Viewer texture path, threading |
| [04-WIDGET-MAP.md](04-WIDGET-MAP.md) | egui concept → Flutter equivalent, one table row per mechanism |
| [05-PARITY-CHECKLIST.md](05-PARITY-CHECKLIST.md) | The living tick-list tracking the one-for-one port, updated every session |
| [06-REMAINING-WORK.md](06-REMAINING-WORK.md) | The delete-on-done ledger: blocked and deferred rows, each with its evidence |
| [07-AUDIT-2026-07-22.md](07-AUDIT-2026-07-22.md) | The round-5 audit: the three desk-test fixes, the navigation guide, open items |

The Flutter application itself lives in `flutter_ui/` at the repository root
(a Dart package named `lumit_flutter`). The Rust crates are untouched by this
branch except where the bridge eventually needs `pub` surface.

Read `docs/GUIDE.md` §9 for the plain-English framing of what Flutter and Dart
are and how this experiment relates to the existing application.
