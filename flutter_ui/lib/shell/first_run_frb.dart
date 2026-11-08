// The first-run screen: a few questions, asked once.
//
// On the very first launch, a machine with no settings file, Lumit asks which
// workspace to start in. That is the one real question, so nothing is picked
// for it and Continue waits for an answer. Under it sit Lumit's own defaults,
// each marked as the one to keep unless you know you want another: the
// shortcuts, the style, the colour scheme and the language.
//
// Every setting it writes is an ordinary row in Settings afterwards, so
// nothing here is a decision anybody is stuck with. The look is applied as it
// is chosen, so the window behind the screen and the workspace drawings are
// the preview. Skipping puts the look back to what it was.

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';

import '../l10n/strings.dart';
import '../state/dock.dart';
import '../state/keymap.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/theme_swatches.dart';

/// The workspaces offered, in the strip's order.
const _workspaces = [
  WorkspacePreset.edit,
  WorkspacePreset.cut,
  WorkspacePreset.nodes,
];

/// What the screen comes back with. A null [workspace] is Skip.
typedef FirstRunAnswer = ({
  WorkspacePreset? workspace,
  BridgeKeymapPreset keys,
  bool autoUpdate,
});

/// Show the screen if this machine has never answered it, and record the
/// answer. Does nothing at all on any later launch, so callers can call it
/// unconditionally at start-up.
///
/// [keymap] is what the After Effects or Vegas shortcuts load into.
Future<void> maybeShowFirstRunFrb(BuildContext context, Workspace workspace,
    {KeymapState? keymap}) async {
  if (workspace.firstRunDone) return;
  // The look as it stood, which is what Skip goes back to.
  final shape = workspace.themeShape;
  final scheme = workspace.themeChoice;
  final language = workspace.interface.language;
  final answer = await showLumitModal<FirstRunAnswer>(
    context: context,
    // An id, so a stray click beside the window doesn't skip the questions.
    id: 'first-run',
    initialSize: const Size(640, 440),
    minSize: const Size(520, 380),
    builder: (close) => _FirstRun(workspace: workspace, onChoose: close),
  );
  // Escape is the same as Skip: the questions have been put, so they are not
  // put again, and Lumit's defaults stand.
  workspace.setAutoUpdate(answer?.autoUpdate ?? true);
  final preset = answer?.workspace;
  if (answer == null || preset == null) {
    workspace.setShape(shape);
    workspace.choose(scheme);
    workspace.interface.language = language;
    workspace.settingsChanged();
    workspace.skipFirstRun();
    return;
  }
  final vegas = answer.keys == BridgeKeymapPreset.vegas;
  // Cut brings footage in on Sequence layers, as an editor expects, and the
  // Vegas keys bring Vegas's speed graph with them.
  workspace.setEditingStyle(
      speedGraph: vegas, sequenceLayers: preset == WorkspacePreset.cut);
  if (answer.keys != BridgeKeymapPreset.lumit) {
    await keymap?.loadPreset(answer.keys);
  }
  if (vegas) {
    // Vegas's mouse too, and a clip dropped over another crossfades into it.
    workspace.interface
      ..cutVegasDrags = true
      ..cutAutoCrossfade = true;
    workspace.settingsChanged();
  }
  workspace.applyWorkspacePreset(preset);
}

class _FirstRun extends StatefulWidget {
  final Workspace workspace;
  final ValueChanged<FirstRunAnswer?> onChoose;
  const _FirstRun({required this.workspace, required this.onChoose});

  @override
  State<_FirstRun> createState() => _FirstRunState();
}

class _FirstRunState extends State<_FirstRun> {
  /// Nothing to begin with: this is the one question to answer yourself.
  WorkspacePreset? _workspace;

  BridgeKeymapPreset _keys = BridgeKeymapPreset.lumit;

  /// Permission to look for new versions, not to fetch them.
  bool _autoUpdate = true;

  void _answer(WorkspacePreset? workspace) => widget.onChoose((
        workspace: workspace,
        keys: _keys,
        autoUpdate: _autoUpdate,
      ));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final workspace = widget.workspace;
    return FloatSurface(
      child: SizedBox.expand(
        // The look is the workspace's, and each choice here changes it.
        child: ListenableBuilder(
          listenable: workspace,
          builder: (context, _) => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 14, 16, 12),
                child: Text(l10n.firstRunSetup,
                    style: t.bodyStrong.copyWith(color: t.textPrimary)),
              ),
              // Scrolls when the window is too short for all of it, so no
              // question is ever out of reach.
              Expanded(
                child: SingleChildScrollView(
                  padding: const EdgeInsets.symmetric(horizontal: 16),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Padding(
                        padding: const EdgeInsets.only(bottom: 8),
                        child: Text(t.kickerCase(l10n.settingsGroupWorkspace),
                            style: t.kicker),
                      ),
                      // Equal shares, every card as tall as the tallest, so a
                      // narrow window grows the row down, never sideways.
                      IntrinsicHeight(
                        child: Row(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            for (final preset in _workspaces) ...[
                              if (preset != _workspaces.first)
                                const SizedBox(width: 10),
                              Expanded(
                                child: _WorkspaceChoice(
                                  preset: preset,
                                  chosen: _workspace == preset,
                                  onTap: () =>
                                      setState(() => _workspace = preset),
                                ),
                              ),
                            ],
                          ],
                        ),
                      ),
                      const SizedBox(height: 18),
                      Container(height: 1, color: t.hairline),
                      const SizedBox(height: 14),
                      _row(
                        t,
                        l10n.settingsPageShortcuts,
                        _Chips<BridgeKeymapPreset>(
                          id: 'first-run-keys',
                          options: BridgeKeymapPreset.values,
                          value: _keys,
                          recommended: BridgeKeymapPreset.lumit,
                          label: (keys) => switch (keys) {
                            BridgeKeymapPreset.lumit => l10n.firstRunLumit,
                            BridgeKeymapPreset.afterEffects =>
                              l10n.keymapAfterEffects,
                            BridgeKeymapPreset.vegas => l10n.firstRunVegasName,
                          },
                          onChanged: (keys) => setState(() => _keys = keys),
                        ),
                      ),
                      _row(
                        t,
                        l10n.firstRunStyle,
                        _Chips<ThemeShape>(
                          id: 'first-run-style',
                          options: ThemeShape.values,
                          value: workspace.themeShape,
                          recommended: ThemeShape.studio,
                          label: (shape) => switch (shape) {
                            ThemeShape.studio => l10n.shapeStudio,
                            ThemeShape.desk => l10n.shapeDesk,
                            ThemeShape.lantern => l10n.shapeLantern,
                          },
                          onChanged: workspace.setShape,
                        ),
                      ),
                      _row(
                        t,
                        l10n.settingsColourScheme,
                        Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            SizedBox(
                              width: 190,
                              height: _rowControl,
                              child: BareDropdown<ThemeChoice>(
                                key: const ValueKey('first-run-scheme'),
                                value: workspace.themeChoice,
                                options: workspace.themeChoices,
                                label: (c) => c.label,
                                group: (c) => c.group,
                                onChanged: workspace.choose,
                                // Hovering a row previews that scheme, on the
                                // window behind too.
                                onPreview: workspace.previewChoice,
                                onPreviewEnd: () =>
                                    workspace.previewChoice(null),
                              ),
                            ),
                            const SizedBox(width: 8),
                            ThemeSwatchStrip(theme: workspace.theme),
                          ],
                        ),
                      ),
                      _row(
                        t,
                        l10n.settingsLanguage,
                        SizedBox(
                          width: 190,
                          height: _rowControl,
                          child: BareDropdown<String?>(
                            key: const ValueKey('first-run-language'),
                            value: workspace.interface.language,
                            options: [null, ...languageNames.keys],
                            // Each language names itself, so the list reads
                            // the same whichever one is in force.
                            label: (tag) => tag == null
                                ? l10n.languageFollowSystem
                                : languageNames[tag]!,
                            onChanged: (tag) {
                              workspace.interface.language = tag;
                              workspace.settingsChanged();
                            },
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 10, 16, 14),
                child: Row(
                  children: [
                    // The update tick, a smaller question. A page is newest.
                    if (kIsWeb)
                      const Spacer()
                    else ...[
                      HouseCheckbox(
                        key: const ValueKey('first-run-auto-update'),
                        value: _autoUpdate,
                        onChanged: (on) => setState(() => _autoUpdate = on),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          l10n.settingsAutomaticUpdates,
                          style: t.small.copyWith(color: t.textMuted),
                        ),
                      ),
                    ],
                    const SizedBox(width: 12),
                    HouseButton(
                      key: const ValueKey('first-run-skip'),
                      small: true,
                      frameless: true,
                      onPressed: () => _answer(null),
                      child: Text(l10n.skip, style: t.small),
                    ),
                    const SizedBox(width: 6),
                    HouseButton(
                      key: const ValueKey('first-run-continue'),
                      primary: true,
                      onPressed:
                          _workspace == null ? null : () => _answer(_workspace),
                      child: Text(l10n.firstRunContinue),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  /// A question that is one line: its name, then its control.
  Widget _row(LumitTheme t, String label, Widget control) => Padding(
        padding: const EdgeInsets.only(bottom: 10),
        child: Row(
          children: [
            SizedBox(width: 130, child: Text(label, style: t.body)),
            Flexible(child: control),
          ],
        ),
      );
}

/// How tall a one-line question's control stands.
const double _rowControl = 22;

/// A big button: one of the answers that has a picture to show.
///
/// Its own surface and not the house button's, because that is an action's
/// face and a stadium under Lantern, which is the wrong shape for something
/// holding a picture. A card on a page wears the section's corner, as the
/// welcome screen's do. The one in force takes the accent as an outline and a
/// tint in every style.
class _Card extends StatefulWidget {
  final String id;
  final bool chosen;
  final VoidCallback onTap;
  final Widget child;

  const _Card({
    required this.id,
    required this.chosen,
    required this.onTap,
    required this.child,
  });

  @override
  State<_Card> createState() => _CardState();
}

class _CardState extends State<_Card> {
  bool _hover = false;
  bool _down = false;

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final step = _hover ? scope.motion.hoverIn : scope.motion.hoverOut;
    // A press gives a little, as the house button's does: at Full only, and
    // never in Desk, whose controls stop dead.
    final gives = scope.animationLevel == AnimationLevel.all &&
        t.shape != ThemeShape.desk;
    return Semantics(
      button: true,
      selected: widget.chosen,
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          key: ValueKey<String>(widget.id),
          behavior: HitTestBehavior.opaque,
          onTapDown: (_) => setState(() => _down = true),
          onTapUp: (_) => setState(() => _down = false),
          onTapCancel: () => setState(() => _down = false),
          onTap: widget.onTap,
          child: AnimatedScale(
            scale: gives && _down ? 0.98 : 1,
            duration: scope.motion.hoverIn.duration,
            curve: scope.motion.hoverIn.curve,
            child: AnimatedContainer(
              duration: step.duration,
              curve: step.curve,
              padding: const EdgeInsets.all(8),
              decoration: BoxDecoration(
                color: widget.chosen
                    ? t.accent.withValues(alpha: _hover ? 0.24 : 0.16)
                    : _hover
                        ? t.surface4
                        : t.surface2,
                borderRadius: BorderRadius.circular(t.tokens.sectionRadius),
                border: Border.all(
                  color: widget.chosen
                      ? t.accent
                      : _hover
                          ? t.hairlineStrong
                          : t.hairline,
                ),
              ),
              child: widget.child,
            ),
          ),
        ),
      ),
    );
  }
}

/// One workspace: a drawing of the window as it arranges it, in the style in
/// force, then its name and what it is for.
class _WorkspaceChoice extends StatelessWidget {
  final WorkspacePreset preset;
  final bool chosen;
  final VoidCallback onTap;

  const _WorkspaceChoice({
    required this.preset,
    required this.chosen,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // The drawing's corner sits inside the card's, the padding between them.
    final inner = (t.tokens.sectionRadius - 8).clamp(0.0, 8.0);
    return _Card(
      id: 'first-run-workspace-${preset.name}',
      chosen: chosen,
      onTap: onTap,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          AspectRatio(
            aspectRatio: _drawing.aspectRatio,
            child: CustomPaint(
                painter: _WorkspacePainter(preset, t, inner.toDouble())),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(2, 8, 2, 2),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(preset.title, style: t.bodyPrimary),
                const SizedBox(height: 2),
                Text(
                  switch (preset) {
                    WorkspacePreset.cut => l10n.firstRunWorkspaceCut,
                    WorkspacePreset.nodes => l10n.firstRunWorkspaceNodes,
                    _ => l10n.firstRunWorkspaceEdit,
                  },
                  style: t.small.copyWith(color: t.textSecondary),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// The size the workspace drawings are made at, then scaled to the card.
const Size _drawing = Size(160, 100);

/// The window in outline, as one workspace arranges it and one style draws it.
///
/// The panes come from the workspace's own factory layout, so the drawing can
/// never disagree with what Continue opens. Each pane carries a hint of what
/// it holds: a picture, layer bars, clips on tracks, nodes and their wires,
/// faders, or a few rows. The style shows in the tools (a strip, a rail or a
/// pill) and in whether panes meet flush or stand apart as cards.
class _WorkspacePainter extends CustomPainter {
  final WorkspacePreset preset;
  final LumitTheme theme;
  final double corner;

  const _WorkspacePainter(this.preset, this.theme, this.corner);

  @override
  void paint(Canvas canvas, Size size) {
    final t = theme;
    final lantern = t.shape == ThemeShape.lantern;
    final desk = t.shape == ThemeShape.desk;
    final window =
        RRect.fromRectAndRadius(Offset.zero & size, Radius.circular(corner));
    canvas.save();
    canvas.clipRRect(window);
    canvas.scale(size.width / _drawing.width, size.height / _drawing.height);
    final full = Offset.zero & _drawing;
    canvas.drawRect(full, Paint()..color = lantern ? t.room : t.surface0);

    final pane = Paint()..color = t.surface1;
    final seam = Paint()..color = t.hairlineStrong;
    final mark = Paint()..color = t.textMuted;
    final lit = Paint()..color = t.accent;

    // The menu line, the same in all three.
    const menu = 7.0;
    if (!lantern) {
      canvas.drawRect(Rect.fromLTWH(0, 0, full.width, menu), pane);
    }
    for (var i = 0; i < 4; i++) {
      canvas.drawRect(Rect.fromLTWH(4 + i * 9.0, 3, 6, 1.5), mark);
    }

    // The tools: a strip, a rail, or a pill.
    var top = menu;
    var left = 0.0;
    if (desk) {
      const rail = 10.0;
      canvas.drawRect(Rect.fromLTWH(0, menu, rail, full.height - menu), pane);
      canvas.drawRect(Rect.fromLTWH(rail, menu, 1, full.height - menu), seam);
      for (var i = 0; i < 6; i++) {
        canvas.drawRect(
            Rect.fromLTWH(3.5, menu + 4 + i * 8.0, 3, 3), i == 0 ? lit : mark);
      }
      left = rail + 1;
    } else if (lantern) {
      const pill = 9.0;
      canvas.drawRRect(
          RRect.fromRectAndRadius(Rect.fromLTWH(3, menu + 2, 56, pill),
              const Radius.circular(pill)),
          pane);
      canvas.drawRRect(
          RRect.fromRectAndRadius(
              Rect.fromLTWH(full.width - 45, menu + 2, 42, pill),
              const Radius.circular(pill)),
          pane);
      for (var i = 0; i < 6; i++) {
        canvas.drawCircle(
            Offset(9 + i * 8.0, menu + 2 + pill / 2), 1.7, i == 0 ? lit : mark);
      }
      top = menu + 2 + pill;
    } else {
      const strip = 9.0;
      canvas.drawRect(Rect.fromLTWH(0, menu, full.width, strip),
          Paint()..color = t.surface2);
      canvas.drawRect(Rect.fromLTWH(0, menu + strip, full.width, 1), seam);
      for (var i = 0; i < 6; i++) {
        canvas.drawRect(
            Rect.fromLTWH(4 + i * 8.0, menu + 3, 3, 3), i == 0 ? lit : mark);
      }
      top = menu + strip + 1;
    }

    final gap = lantern ? 3.0 : 1.0;
    final edge = lantern ? 3.0 : 0.0;
    final area = Rect.fromLTRB(
        left + edge, top + edge, full.width - edge, full.height - edge);
    if (!lantern) canvas.drawRect(area, seam);
    _lay(presetLayout(preset), area, gap, (panel, rect) {
      canvas.drawRRect(
          RRect.fromRectAndRadius(rect, Radius.circular(lantern ? 4 : 0)),
          pane);
      _content(canvas, panel, rect, lantern);
    });
    canvas.restore();
    canvas.drawRRect(
      window.deflate(0.5),
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = t.hairlineStrong,
    );
  }

  /// Hand each pane of [node] its rectangle in [r], tabs as their front pane.
  void _lay(DockNode node, Rect r, double gap,
      void Function(Panel panel, Rect rect) pane) {
    switch (node) {
      case DockPane(:final panel):
        pane(panel, r);
      case DockTabs(:final children, :final active):
        pane(children[active.clamp(0, children.length - 1)].panel, r);
      case DockSplit(:final axis, :final children, :final shares):
        final across = axis == DockAxis.horizontal;
        final room =
            (across ? r.width : r.height) - gap * (children.length - 1);
        var at = across ? r.left : r.top;
        for (var i = 0; i < children.length; i++) {
          final length = room * shares[i];
          _lay(
              children[i],
              across
                  ? Rect.fromLTWH(at, r.top, length, r.height)
                  : Rect.fromLTWH(r.left, at, r.width, length),
              gap,
              pane);
          at += length + gap;
        }
    }
  }

  /// What a pane holds, drawn small.
  void _content(Canvas canvas, Panel panel, Rect r, bool lantern) {
    final t = theme;
    final mark = Paint()..color = t.textMuted;
    final lit = Paint()..color = t.accent;
    final clip = Paint()..color = t.surface4;
    final seam = Paint()..color = t.hairlineStrong;
    final round = Radius.circular(lantern ? 1.5 : 0);
    switch (panel) {
      case Panel.viewer:
        canvas.drawRRect(
            RRect.fromRectAndRadius(
                r.deflate(lantern ? 3 : 4), Radius.circular(lantern ? 3 : 0)),
            Paint()..color = t.surface0);
      case Panel.timeline:
        // Layer names, and a bar each; the first one lit.
        for (var i = 0; i < 6; i++) {
          final y = r.top + 5 + i * 5.5;
          if (y + 3 > r.bottom - 2) break;
          canvas.drawRect(Rect.fromLTWH(r.left + 3, y + 0.5, 18, 1.5), mark);
          canvas.drawRRect(
              RRect.fromRectAndRadius(
                  Rect.fromLTWH(
                      r.left + 28 + i * 8.0, y, r.width - 50 - i * 12.0, 3),
                  round),
              i == 0 ? lit : seam);
        }
      case Panel.cutTimeline:
        // Two rows of picture over two of sound, cut into clips, with one
        // picture clip and its sound lit together.
        final row = (r.height - 6) / 4;
        final lanes =
            Rect.fromLTRB(r.left + 14, r.top + 3, r.right - 3, r.bottom - 3);
        const spans = [
          [(0.0, 0.3), (0.32, 0.62), (0.64, 0.95)],
          [(0.1, 0.45), (0.47, 0.8)],
          [(0.1, 0.45), (0.47, 0.8)],
          [(0.0, 0.62), (0.64, 1.0)],
        ];
        for (var i = 0; i < 4; i++) {
          final y = lanes.top + row * i;
          canvas.drawRect(
              Rect.fromLTWH(r.left + 3, y + row / 2 - 0.75, 8, 1.5), mark);
          for (final (j, (from, to)) in spans[i].indexed) {
            final box = RRect.fromRectAndRadius(
                Rect.fromLTRB(lanes.left + lanes.width * from, y + 0.75,
                    lanes.left + lanes.width * to, y + row - 0.75),
                round);
            final pair = (i == 1 || i == 2) && j == 0;
            canvas.drawRRect(
                box,
                pair
                    ? (Paint()..color = t.accent.withValues(alpha: 0.35))
                    : clip);
            if (i >= 2) {
              // A sound clip's wave, flat at this size.
              canvas.drawRect(
                  Rect.fromLTRB(box.left + 2, box.outerRect.center.dy - 0.4,
                      box.right - 2, box.outerRect.center.dy + 0.4),
                  pair ? lit : mark);
            }
          }
        }
      case Panel.graph:
        // Three nodes wired into a fourth, which is lit.
        final w = 16.0, h = 7.0;
        final inputs = [
          Offset(r.left + r.width * 0.16, r.top + r.height * 0.22),
          Offset(r.left + r.width * 0.16, r.top + r.height * 0.5),
          Offset(r.left + r.width * 0.42, r.top + r.height * 0.78),
        ];
        final out = Offset(r.left + r.width * 0.7, r.top + r.height * 0.45);
        final wire = Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1
          ..color = t.textMuted;
        for (final from in inputs) {
          final a = Offset(from.dx + w / 2, from.dy);
          final b = Offset(out.dx - w / 2, out.dy);
          final reach = (b.dx - a.dx).abs() * 0.5;
          canvas.drawPath(
              Path()
                ..moveTo(a.dx, a.dy)
                ..cubicTo(a.dx + reach, a.dy, b.dx - reach, b.dy, b.dx, b.dy),
              wire);
        }
        for (final at in inputs) {
          canvas.drawRRect(
              RRect.fromRectAndRadius(
                  Rect.fromCenter(center: at, width: w, height: h),
                  const Radius.circular(1.5)),
              clip);
        }
        canvas.drawRRect(
            RRect.fromRectAndRadius(
                Rect.fromCenter(center: out, width: w, height: h),
                const Radius.circular(1.5)),
            lit);
      case Panel.mixer:
        // Two faders.
        for (var i = 0; i < 2; i++) {
          final x = r.left + r.width * (i + 1) / 3;
          canvas.drawRect(
              Rect.fromLTRB(x - 0.5, r.top + 4, x + 0.5, r.bottom - 4), mark);
          canvas.drawRect(
              Rect.fromCenter(
                  center: Offset(x, r.top + r.height * (i == 0 ? 0.4 : 0.6)),
                  width: 5,
                  height: 2.5),
              i == 0 ? lit : mark);
        }
      default:
        for (var i = 0; i < 6; i++) {
          final y = r.top + 5 + i * 5.0;
          if (y + 2 > r.bottom - 2) break;
          canvas.drawRect(
              Rect.fromLTWH(
                  r.left + 3, y, (r.width - 8) * (i.isEven ? 1 : 0.7), 1.5),
              mark);
        }
    }
  }

  @override
  bool shouldRepaint(_WorkspacePainter old) =>
      old.preset != preset || old.theme != theme || old.corner != corner;
}

/// A short set of choices side by side, the one in force lit. The
/// [recommended] one says so after its name, in the label's own colour but
/// quieter, so it reads in every state the button can be in.
class _Chips<T extends Enum> extends StatelessWidget {
  final String id;
  final List<T> options;
  final T value;
  final T? recommended;
  final String Function(T) label;
  final ValueChanged<T> onChanged;

  const _Chips({
    required this.id,
    required this.options,
    required this.value,
    this.recommended,
    required this.label,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) => Wrap(
        spacing: 4,
        runSpacing: 4,
        children: [
          for (final option in options)
            SizedBox(
              height: _rowControl,
              child: HouseButton(
                key: ValueKey<String>('$id-${option.name}'),
                small: true,
                active: option == value,
                padding: const EdgeInsets.symmetric(horizontal: 8),
                onPressed: () => onChanged(option),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(label(option)),
                    if (option == recommended) ...[
                      const SizedBox(width: 6),
                      Builder(builder: (context) {
                        final style = DefaultTextStyle.of(context).style;
                        return Text(
                          l10n.firstRunRecommended,
                          style: style.copyWith(
                              color: style.color?.withValues(alpha: 0.6)),
                        );
                      }),
                    ],
                  ],
                ),
              ),
            ),
        ],
      );
}
