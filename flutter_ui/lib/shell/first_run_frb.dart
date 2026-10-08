// The first-run screen: a few questions, asked once (docs/07 §13.1).
//
// On the very first launch — a machine with no settings file — Lumit asks how
// the user edits and what the interface should look like: the style, the
// colour scheme, where the toolbar stands and how much the chrome moves. The
// After Effects answer loads the After Effects shortcuts with it. One page,
// no steps. Every setting it writes is an ordinary row in Settings
// afterwards — the editing pair under Interface ▸ Editing, the look under
// Appearance and Interface, the update tick under General ▸ Updates — so
// nothing here is a decision anybody is stuck with.
//
// The look is applied as it is chosen, so the window behind the screen and the
// screen itself are the preview. Skipping puts the look back to what it was.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';

import '../l10n/strings.dart';
import '../state/keymap.dart';
import '../state/settings.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/theme_swatches.dart';

/// What the screen comes back with: which editor, and whether Lumit should
/// keep an eye out for new versions. The tick is on the screen rather than
/// only in Settings because it is a decision about how Lumit behaves from now
/// on, which is exactly what this screen is for. A null editor is Skip.
/// [sequenceLayers] is false when the tick on the Vegas card asks for the
/// Retime graph alone, and means nothing with the other answer.
typedef FirstRunAnswer = ({bool? vegas, bool sequenceLayers, bool autoUpdate});

/// Show the screen if this machine has never answered it, and record the
/// answer. Does nothing at all on any later launch, so callers can call it
/// unconditionally at start-up.
///
/// [keymap] is what the After Effects answer loads its shortcuts into.
Future<void> maybeShowFirstRunFrb(BuildContext context, Workspace workspace,
    {KeymapState? keymap}) async {
  if (workspace.firstRunDone) return;
  // The look as it stood, which is what Skip goes back to.
  final shape = workspace.themeShape;
  final scheme = workspace.themeChoice;
  final toolBar = workspace.interface.toolBarPosition;
  final motion = workspace.animationLevel;
  final answer = await showLumitModal<FirstRunAnswer>(
    context: context,
    initialSize: const Size(620, 548),
    minSize: const Size(520, 380),
    builder: (close) => _FirstRun(workspace: workspace, onChoose: close),
  );
  // A null answer is a click on the scrim, which is the same as Skip: the
  // questions have been put, so they are not put again, and the defaults
  // stand — the After Effects shape, with update checks on.
  workspace.setAutoUpdate(answer?.autoUpdate ?? true);
  final vegas = answer?.vegas;
  if (vegas == null) {
    workspace.setShape(shape);
    workspace.choose(scheme);
    workspace.interface.toolBarPosition = toolBar;
    workspace.setAnimationLevel(motion);
    workspace.skipFirstRun();
  } else {
    workspace.setEditingStyle(
        vegas: vegas, sequenceLayers: answer!.sequenceLayers);
    // The card says After Effects, so its keys come too: the preset the
    // After Effects button on Settings > Shortcuts loads.
    if (!vegas) await keymap?.loadPreset(BridgeKeymapPreset.afterEffects);
  }
}

class _FirstRun extends StatefulWidget {
  final Workspace workspace;
  final ValueChanged<FirstRunAnswer?> onChoose;
  const _FirstRun({required this.workspace, required this.onChoose});

  @override
  State<_FirstRun> createState() => _FirstRunState();
}

class _FirstRunState extends State<_FirstRun> {
  /// After Effects to begin with, which is what Lumit is with nothing set.
  bool _vegas = false;

  /// The tick on the Vegas card: the Retime graph is all that changes, and
  /// video goes on arriving as a layer. Clear to begin with, since the two
  /// together are what the card describes.
  bool _retimeOnly = false;

  /// Ticked to begin with. Nothing is downloaded either way — this is
  /// permission to look, not permission to fetch.
  bool _autoUpdate = true;

  /// Answer with both halves at once: the editor, or null for Skip, and the
  /// update tick as it stands.
  void _answer(bool? vegas) => widget.onChoose((
        vegas: vegas,
        sequenceLayers: !_retimeOnly,
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
                padding: const EdgeInsets.fromLTRB(14, 12, 14, 4),
                child: Text(l10n.firstRunSetup,
                    style: t.bodyStrong.copyWith(color: t.textPrimary)),
              ),
              Padding(
                padding: const EdgeInsets.fromLTRB(14, 0, 14, 10),
                child: Text(l10n.firstRunSetupBlurb, style: t.body),
              ),
              // Scrolls when the window is too short for all of it, so no
              // question is ever out of reach.
              Expanded(
                child: SingleChildScrollView(
                  padding: const EdgeInsets.symmetric(horizontal: 14),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      _heading(t, l10n.firstRunTitle),
                      IntrinsicHeight(
                        child: Row(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            Expanded(
                              child: _Choice(
                                id: 'first-run-ae',
                                vegas: false,
                                title: l10n.keymapAfterEffects,
                                blurb: l10n.firstRunAfterEffects,
                                note: l10n.firstRunAfterEffectsKeys,
                                chosen: !_vegas,
                                onTap: () => setState(() => _vegas = false),
                              ),
                            ),
                            const SizedBox(width: 10),
                            Expanded(
                              child: _Choice(
                                id: 'first-run-vegas',
                                vegas: true,
                                title: l10n.firstRunVegasName,
                                blurb: l10n.firstRunVegas,
                                chosen: _vegas,
                                onTap: () => setState(() => _vegas = true),
                                // Ticking or clearing it is choosing Vegas:
                                // it is a question about that answer.
                                option: l10n.firstRunVegasRetimeOnly,
                                optionOn: _retimeOnly,
                                onOption: (on) => setState(() {
                                  _retimeOnly = on;
                                  _vegas = true;
                                }),
                              ),
                            ),
                          ],
                        ),
                      ),
                      _heading(t, l10n.firstRunStyle),
                      Row(
                        children: [
                          for (final shape in ThemeShape.values) ...[
                            if (shape != ThemeShape.values.first)
                              const SizedBox(width: 10),
                            Expanded(
                              child: _StyleChoice(
                                shape: shape,
                                // Drawn in the colours in force, so the three
                                // differ by their shape and nothing else.
                                theme: workspace.theme.copyWith(
                                  shape: shape,
                                  tokens: ShapeTokens.of(shape),
                                ),
                                chosen: workspace.themeShape == shape,
                                onTap: () => workspace.setShape(shape),
                              ),
                            ),
                          ],
                        ],
                      ),
                      const SizedBox(height: 14),
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
                              ),
                            ),
                            const SizedBox(width: 8),
                            ThemeSwatchStrip(theme: workspace.theme),
                          ],
                        ),
                      ),
                      _row(
                        t,
                        l10n.settingsToolBarPosition,
                        _Chips<ToolBarPosition>(
                          id: 'first-run-toolbar',
                          options: ToolBarPosition.values,
                          value: workspace.interface.toolBarPosition,
                          label: (position) => switch (position) {
                            ToolBarPosition.auto => l10n.styleChoice,
                            ToolBarPosition.top => l10n.toolBarTop,
                            ToolBarPosition.left => l10n.toolBarLeft,
                          },
                          onChanged: (position) {
                            workspace.interface.toolBarPosition = position;
                            workspace.settingsChanged();
                          },
                        ),
                      ),
                      _row(
                        t,
                        l10n.settingsMotion,
                        _Chips<AnimationLevel>(
                          id: 'first-run-motion',
                          options: AnimationLevel.values,
                          value: workspace.animationLevel,
                          label: (level) => switch (level) {
                            AnimationLevel.all => l10n.motionFull,
                            AnimationLevel.minimal => l10n.motionMinimal,
                            AnimationLevel.none => l10n.none,
                          },
                          onChanged: workspace.setAnimationLevel,
                        ),
                      ),
                    ],
                  ),
                ),
              ),
              Padding(
                padding: const EdgeInsets.fromLTRB(14, 10, 14, 12),
                child: Row(
                  children: [
                    // The update tick sits in the footer rather than in a
                    // section of its own: it is a much smaller question, and
                    // giving it a heading would suggest it is the others'
                    // equal.
                    HouseCheckbox(
                      key: const ValueKey('first-run-auto-update'),
                      value: _autoUpdate,
                      onChanged: (on) => setState(() => _autoUpdate = on),
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        l10n.firstRunAutoUpdate,
                        style: t.small.copyWith(color: t.textMuted),
                      ),
                    ),
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
                      onPressed: () => _answer(_vegas),
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

  /// The word over a question.
  Widget _heading(LumitTheme t, String text) => Padding(
        padding: const EdgeInsets.only(top: 4, bottom: 6),
        child: Text(t.kickerCase(text), style: t.kicker),
      );

  /// A question that is one line: its name, then its control.
  Widget _row(LumitTheme t, String label, Widget control) => Padding(
        padding: const EdgeInsets.only(bottom: 8),
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

/// How tall the editing drawings stand. Each is as wide as its card.
const double _editingPicture = 66;

/// The style drawings' size, the proportions of a window.
const Size _silhouette = Size(132, 84);

/// A big button: one of the answers that has more to show than a word.
///
/// Its own surface and not the house button's, because that is an action's
/// face and a stadium under Lantern, which is the wrong shape for something
/// holding a picture or a paragraph. A card on a page wears the section's
/// corner, as the welcome screen's do. The one in force takes the accent as
/// an outline and a tint in every style.
class _Card extends StatefulWidget {
  final String id;
  final bool chosen;
  final VoidCallback onTap;
  final EdgeInsets padding;
  final Widget child;

  const _Card({
    required this.id,
    required this.chosen,
    required this.onTap,
    required this.padding,
    required this.child,
  });

  @override
  State<_Card> createState() => _CardState();
}

class _CardState extends State<_Card> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final step = _hover ? scope.motion.hoverIn : scope.motion.hoverOut;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        key: ValueKey<String>(widget.id),
        behavior: HitTestBehavior.opaque,
        onTap: widget.onTap,
        child: AnimatedContainer(
          duration: step.duration,
          curve: step.curve,
          padding: widget.padding,
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
    );
  }
}

/// One answer to how you edit: a tall card, because the blurb is as much a
/// part of the choice as the name over it, with a drawing of the Timeline as
/// that editor has it across the top.
class _Choice extends StatelessWidget {
  final String id;

  /// Which of the two the drawing is of.
  final bool vegas;
  final String title;
  final String blurb;

  /// One more line under the blurb, for something else the answer does.
  final String? note;
  final bool chosen;
  final VoidCallback onTap;

  /// A tick at the foot of the card, for an answer that comes in two
  /// strengths: its words, whether it is on, and what changes it.
  final String? option;
  final bool optionOn;
  final ValueChanged<bool>? onOption;

  const _Choice({
    required this.id,
    required this.vegas,
    required this.title,
    required this.blurb,
    this.note,
    required this.chosen,
    required this.onTap,
    this.option,
    this.optionOn = false,
    this.onOption,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return _Card(
      id: id,
      chosen: chosen,
      onTap: onTap,
      padding: const EdgeInsets.all(10),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: double.infinity,
            height: _editingPicture,
            child: CustomPaint(painter: _EditingPainter(vegas, t)),
          ),
          const SizedBox(height: 8),
          Text(title, style: t.bodyPrimary),
          const SizedBox(height: 4),
          Text(blurb, style: t.small.copyWith(color: t.textSecondary)),
          if (note case final note?) ...[
            const SizedBox(height: 8),
            Text(note, style: t.small.copyWith(color: t.textSecondary)),
          ],
          if (option case final option?) ...[
            const SizedBox(height: 8),
            Row(
              children: [
                HouseCheckbox(
                  key: ValueKey<String>('$id-option'),
                  value: optionOn,
                  onChanged: onOption,
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(option,
                      style: t.small.copyWith(color: t.textPrimary)),
                ),
              ],
            ),
          ],
        ],
      ),
    );
  }
}

/// The Timeline in outline, as one way of editing has it.
///
/// What differs between the two is how a layer is retimed, so that is what is
/// drawn. After Effects: the graph open in place of the bars, with one curve
/// rising through it from one key to the next, which is the moment of the
/// source against time. Vegas: three tracks of clips, one of them cut in two,
/// and one carrying its speed as a line along the clip that dips and comes
/// back up between its points.
class _EditingPainter extends CustomPainter {
  final bool vegas;
  final LumitTheme theme;

  const _EditingPainter(this.vegas, this.theme);

  @override
  void paint(Canvas canvas, Size size) {
    final t = theme;
    final desk = t.shape == ThemeShape.desk;
    final lantern = t.shape == ThemeShape.lantern;
    final window = RRect.fromRectAndRadius(
        Offset.zero & size, Radius.circular(desk ? 0 : (lantern ? 5 : 3)));
    canvas.save();
    canvas.clipRRect(window);
    canvas.drawRect(Offset.zero & size, Paint()..color = t.surface1);

    final seam = Paint()..color = t.hairlineStrong;
    final mark = Paint()..color = t.textMuted;

    // The names down the left and the ruler along the top, as both have them.
    const ruler = 8.0;
    final names = (size.width * (vegas ? 0.2 : 0.27)).roundToDouble();
    final lanes = Rect.fromLTRB(names + 1, ruler + 1, size.width, size.height);
    canvas.drawRect(
        Rect.fromLTWH(0, 0, size.width, ruler), Paint()..color = t.surface2);
    for (var x = lanes.left + 9; x < size.width; x += 18) {
      canvas.drawRect(Rect.fromLTWH(x, ruler - 3, 1, 3), mark);
    }
    canvas.drawRect(Rect.fromLTWH(0, ruler, size.width, 1), seam);
    canvas.drawRect(Rect.fromLTWH(names, 0, 1, size.height), seam);

    if (vegas) {
      _tracks(canvas, t, lanes, names, lantern ? 2 : (desk ? 0 : 1));
    } else {
      _graph(canvas, t, lanes, names);
    }
    canvas.restore();
    canvas.drawRRect(
      window.deflate(0.5),
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = t.hairlineStrong,
    );
  }

  /// A layer with its property twirled open, and the graph where the bars
  /// would be.
  void _graph(Canvas canvas, LumitTheme t, Rect lanes, double names) {
    final lit = Paint()..color = t.accent;
    final mark = Paint()..color = t.textMuted;
    final faint = Paint()..color = t.hairline;
    final row = lanes.height / 3;
    for (var i = 0; i < 3; i++) {
      final y = lanes.top + row * i + row / 2 - 1;
      // The middle row is the property the graph is drawing, set in under
      // its layer.
      final inset = i == 1 ? 13.0 : 6.0;
      canvas.drawRect(
          Rect.fromLTWH(inset, y, math.max(4, names - inset - 8 - i * 5), 2),
          i == 1 ? lit : mark);
    }
    // The graph's ground: a few lines each way.
    for (var i = 1; i < 4; i++) {
      canvas.drawRect(
          Rect.fromLTWH(
              lanes.left, lanes.top + lanes.height * i / 4, lanes.width, 1),
          faint);
    }
    for (var i = 1; i < 5; i++) {
      canvas.drawRect(
          Rect.fromLTWH(
              lanes.left + lanes.width * i / 5, lanes.top, 1, lanes.height),
          faint);
    }
    // One curve from a key low on the left to a key high on the right,
    // leaving and arriving level. Its handles lie flat, so it never turns
    // back on itself.
    final from = Offset(lanes.left + 12, lanes.bottom - 9);
    final to = Offset(lanes.right - 12, lanes.top + 8);
    final reach = (to.dx - from.dx) * 0.42;
    final handle = Paint()
      ..color = t.textMuted
      ..strokeWidth = 1;
    final leaving = Offset(from.dx + reach * 0.6, from.dy);
    final arriving = Offset(to.dx - reach * 0.6, to.dy);
    canvas.drawLine(from, leaving, handle);
    canvas.drawLine(to, arriving, handle);
    canvas.drawCircle(leaving, 1.6, mark);
    canvas.drawCircle(arriving, 1.6, mark);
    canvas.drawPath(
      Path()
        ..moveTo(from.dx, from.dy)
        ..cubicTo(from.dx + reach, from.dy, to.dx - reach, to.dy, to.dx, to.dy),
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.75
        ..strokeCap = StrokeCap.round
        ..color = t.accent,
    );
    for (final key in [from, to]) {
      canvas.drawRect(Rect.fromCenter(center: key, width: 5, height: 5), lit);
    }
  }

  /// Three tracks of clips. The middle one stands taller and wears its speed.
  void _tracks(
      Canvas canvas, LumitTheme t, Rect lanes, double names, double corner) {
    final seam = Paint()..color = t.hairlineStrong;
    final mark = Paint()..color = t.textMuted;
    final clip = Paint()..color = t.surface4;
    final radius = Radius.circular(corner);
    final unit = lanes.height / 3.8;
    final heights = [unit, unit * 1.8, unit];
    var top = lanes.top;
    for (var i = 0; i < 3; i++) {
      final track = Rect.fromLTWH(lanes.left, top, lanes.width, heights[i]);
      top = track.bottom;
      if (i > 0) {
        canvas.drawRect(Rect.fromLTWH(0, track.top, lanes.right, 1), seam);
      }
      // The track's name and its two switches.
      canvas.drawRect(
          Rect.fromLTWH(5, track.center.dy - 1, math.max(4, names - 24), 2),
          mark);
      for (var b = 0; b < 2; b++) {
        canvas.drawRect(
            Rect.fromLTWH(names - 14 + b * 6.0, track.center.dy - 2, 3.5, 3.5),
            mark);
      }
      final body = track.deflate(2.5);
      RRect piece(double from, double to) => RRect.fromRectAndRadius(
          Rect.fromLTRB(body.left + body.width * from, body.top,
              body.left + body.width * to, body.bottom),
          radius);
      switch (i) {
        case 0:
          // One piece of footage, cut in two.
          canvas.drawRRect(piece(0.03, 0.44), clip);
          canvas.drawRRect(piece(0.45, 0.8), clip);
        case 1:
          final speed = piece(0.1, 0.97);
          canvas.drawRRect(
              speed, Paint()..color = t.accent.withValues(alpha: 0.2));
          _speed(canvas, t, speed.outerRect.deflate(4));
        default:
          canvas.drawRRect(piece(0.03, 0.62), clip);
      }
    }
  }

  /// A clip's speed along it: from each point the line dips and comes back up
  /// to the next, so the whole of it reads as a run of bowls joined at their
  /// rims.
  void _speed(Canvas canvas, LumitTheme t, Rect room) {
    // As many bowls as stand about twice as wide as they are deep.
    final count = math.max(2, (room.width / (room.height * 2.4)).round());
    final width = room.width / count;
    // A cubic with both handles at one depth reaches three quarters of the
    // way to it, so the handles go a third deeper than the bowl.
    final deep = room.top + room.height / 0.75;
    final line = Path()..moveTo(room.left, room.top);
    for (var i = 0; i < count; i++) {
      final from = room.left + width * i;
      line.cubicTo(from + width * 0.14, deep, from + width * 0.86, deep,
          from + width, room.top);
    }
    canvas.drawPath(
      line,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.5
        ..strokeJoin = StrokeJoin.round
        ..strokeCap = StrokeCap.round
        ..color = t.accent,
    );
    final point = Paint()..color = t.accent;
    for (var i = 0; i <= count; i++) {
      canvas.drawCircle(Offset(room.left + width * i, room.top), 2, point);
    }
  }

  @override
  bool shouldRepaint(_EditingPainter old) =>
      old.vegas != vegas || old.theme != theme;
}

/// One style: a drawing of the window as that style lays it out, with its
/// name under it, and the whole of it the button.
class _StyleChoice extends StatelessWidget {
  final ThemeShape shape;

  /// The theme the drawing is made in: this style's shape, the colours in
  /// force.
  final LumitTheme theme;
  final bool chosen;
  final VoidCallback onTap;

  const _StyleChoice({
    required this.shape,
    required this.theme,
    required this.chosen,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return _Card(
      id: 'first-run-style-${shape.name}',
      chosen: chosen,
      onTap: onTap,
      padding: const EdgeInsets.fromLTRB(10, 10, 10, 8),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          FittedBox(
            fit: BoxFit.scaleDown,
            child: CustomPaint(
              size: _silhouette,
              painter: _SilhouettePainter(shape, theme),
            ),
          ),
          const SizedBox(height: 8),
          Text(
            switch (shape) {
              ThemeShape.studio => l10n.shapeStudio,
              ThemeShape.desk => l10n.shapeDesk,
              ThemeShape.lantern => l10n.shapeLantern,
            },
            style: t.bodyPrimary,
          ),
        ],
      ),
    );
  }
}

/// The window in outline, as one style arranges it.
///
/// Three things tell the styles apart at this size, and they are what is
/// drawn. Studio: a tool strip across the top and panes meeting flush with a
/// hairline between them. Desk: the same flush panes, square, with the tools
/// on a rail down the left. Lantern: the panes as rounded cards standing apart
/// in a room, with the tools in a pill.
class _SilhouettePainter extends CustomPainter {
  final ThemeShape shape;
  final LumitTheme theme;

  const _SilhouettePainter(this.shape, this.theme);

  @override
  void paint(Canvas canvas, Size size) {
    final t = theme;
    final lantern = shape == ThemeShape.lantern;
    final desk = shape == ThemeShape.desk;
    final window = RRect.fromRectAndRadius(
        Offset.zero & size, Radius.circular(desk ? 0 : 3));
    canvas.save();
    canvas.clipRRect(window);
    canvas.drawRect(
        Offset.zero & size, Paint()..color = lantern ? t.room : t.surface0);

    final pane = Paint()..color = t.surface1;
    final seam = Paint()..color = t.hairlineStrong;
    final mark = Paint()..color = t.textMuted;
    final lit = Paint()..color = t.accent;

    // The menu line, the same in all three.
    const menu = 7.0;
    if (!lantern) {
      canvas.drawRect(Rect.fromLTWH(0, 0, size.width, menu), pane);
    }
    for (var i = 0; i < 4; i++) {
      canvas.drawRect(Rect.fromLTWH(4 + i * 9.0, 3, 6, 1.5), mark);
    }

    // The tools: a strip, a rail, or a pill.
    var top = menu;
    var left = 0.0;
    if (desk) {
      const rail = 10.0;
      canvas.drawRect(Rect.fromLTWH(0, menu, rail, size.height - menu), pane);
      canvas.drawRect(Rect.fromLTWH(rail, menu, 1, size.height - menu), seam);
      canvas.drawRect(const Rect.fromLTWH(0, menu + 3, 1.5, 5), lit);
      for (var i = 0; i < 6; i++) {
        canvas.drawRect(
            Rect.fromLTWH(3.5, menu + 4 + i * 8.0, 3, 3), i == 0 ? lit : mark);
      }
      left = rail + 1;
    } else if (lantern) {
      const pill = 9.0;
      final tools = RRect.fromRectAndRadius(
          Rect.fromLTWH(3, menu + 2, 56, pill), const Radius.circular(pill));
      final spaces = RRect.fromRectAndRadius(
          Rect.fromLTWH(size.width - 45, menu + 2, 42, pill),
          const Radius.circular(pill));
      canvas.drawRRect(tools, pane);
      canvas.drawRRect(spaces, pane);
      for (var i = 0; i < 6; i++) {
        canvas.drawCircle(
            Offset(9 + i * 8.0, menu + 2 + pill / 2), 1.7, i == 0 ? lit : mark);
      }
      top = menu + 2 + pill;
    } else {
      const strip = 9.0;
      canvas.drawRect(
          Rect.fromLTWH(0, menu, size.width, strip), Paint()..color = t.surface2);
      canvas.drawRect(Rect.fromLTWH(0, menu + strip, size.width, 1), seam);
      for (var i = 0; i < 6; i++) {
        canvas.drawRect(
            Rect.fromLTWH(4 + i * 8.0, menu + 3, 3, 3), i == 0 ? lit : mark);
      }
      top = menu + strip + 1;
    }

    // The panes: three across, and the Timeline under them.
    final gap = lantern ? 3.0 : 1.0;
    final edge = lantern ? 3.0 : 0.0;
    final corner = Radius.circular(lantern ? 4 : 0);
    final area = Rect.fromLTRB(
        left + edge, top + edge, size.width - edge, size.height - edge);
    final upper = (area.height - gap) * 0.62;
    final side = (area.width - 2 * gap) * 0.22;
    final project = Rect.fromLTWH(area.left, area.top, side, upper);
    final viewer = Rect.fromLTWH(
        project.right + gap, area.top, area.width - 2 * (side + gap), upper);
    final effects = Rect.fromLTWH(viewer.right + gap, area.top, side, upper);
    final timeline = Rect.fromLTRB(
        area.left, area.top + upper + gap, area.right, area.bottom);
    if (!lantern) canvas.drawRect(area, seam);
    for (final rect in [project, viewer, effects, timeline]) {
      canvas.drawRRect(RRect.fromRectAndRadius(rect, corner), pane);
    }
    // The picture in the Viewer, a few rows in the side panes, and the
    // Timeline's bars.
    canvas.drawRRect(
        RRect.fromRectAndRadius(viewer.deflate(lantern ? 3 : 4), corner),
        Paint()..color = t.surface0);
    for (final rect in [project, effects]) {
      for (var i = 0; i < 4; i++) {
        canvas.drawRect(
            Rect.fromLTWH(
                rect.left + 3, rect.top + 5 + i * 5.0, rect.width - 8, 1.5),
            mark);
      }
    }
    for (var i = 0; i < 3; i++) {
      final y = timeline.top + 5 + i * 5.5;
      if (y + 3 > timeline.bottom - 2) break;
      canvas.drawRect(Rect.fromLTWH(timeline.left + 3, y + 0.5, 22, 1.5), mark);
      canvas.drawRRect(
          RRect.fromRectAndRadius(
              Rect.fromLTWH(timeline.left + 34 + i * 9.0, y,
                  timeline.width - 60 - i * 14.0, 3),
              Radius.circular(lantern ? 1.5 : 0)),
          i == 0 ? lit : seam);
    }
    canvas.restore();
    canvas.drawRRect(
      window.deflate(0.5),
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = t.hairlineStrong,
    );
  }

  @override
  bool shouldRepaint(_SilhouettePainter old) =>
      old.shape != shape || old.theme != theme;
}

/// A short set of choices side by side, the one in force lit.
class _Chips<T extends Enum> extends StatelessWidget {
  final String id;
  final List<T> options;
  final T value;
  final String Function(T) label;
  final ValueChanged<T> onChanged;

  const _Chips({
    required this.id,
    required this.options,
    required this.value,
    required this.label,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) => Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final option in options) ...[
            if (option != options.first) const SizedBox(width: 4),
            SizedBox(
              height: _rowControl,
              child: HouseButton(
                key: ValueKey<String>('$id-${option.name}'),
                small: true,
                active: option == value,
                padding: const EdgeInsets.symmetric(horizontal: 8),
                onPressed: () => onChanged(option),
                child: Text(label(option)),
              ),
            ),
          ],
        ],
      );
}
