// The easing editor: a unit box you shape an ease in, and a button that stamps
// it onto the selected keyframes (docs/07 §5.3 and §5.4).
//
// The graph editor shapes one span at a time, in the units that span happens
// to use. This is the same shape drawn once, in the abstract: the travel runs
// left to right across the box, from nothing done to all done, and the two
// handles bend it. Because the box is unitless the shape is reusable, and
// Apply puts it on every span the selection covers, whatever those spans move
// by (`applyEasingToSelection`).
//
// **One editor, shown two ways**. By default it is the Easing panel
// (`easing_panel_frb.dart`), which stays on screen while the selection changes
// underneath it. Settings ▸ Interface ▸ Editing turns it back into a popup that
// opens from the graph's own bottom bar. [EasingEditor] is the body both show:
// the panel gives it a height to fill, so the box and its buttons stay put and
// only the library scrolls, and the popup gives it none.
//
// **Shapes of your own.** Save keeps the drawn curve under a name, and it then
// stands in the library after the ones that ship, applying by the same road.
// Those belong to the person rather than to the project, so they live beside
// the settings file (`state/custom_easings.dart`).

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart' show kPrimaryButton;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/custom_easings.dart';
import '../state/file_dialogs.dart'
    show pickEasingsSaveLocation, pickEasingsToOpen;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'easing_curve.dart';
import 'easing_painters.dart';
import 'graph_maths.dart' show minTangentReach;
import 'timeline_extras_frb.dart' show showMenuAt;

/// A preset's shown name. Kept beside the editor rather than in
/// `easing_curve.dart` so the maths stays free of the locale.
String easingPresetName(String id) => switch (id) {
      'easy' => l10n.easeEasy,
      'slowStart' => l10n.easingSlowStart,
      'slowFinish' => l10n.easingSlowFinish,
      'heavy' => l10n.easingHeavy,
      'snap' => l10n.easingSnap,
      'sineIn' => l10n.easingSineIn,
      'sineOut' => l10n.easingSineOut,
      'sineInOut' => l10n.easingSineInOut,
      'quadIn' => l10n.easingQuadIn,
      'quadOut' => l10n.easingQuadOut,
      'quadInOut' => l10n.easingQuadInOut,
      'cubicIn' => l10n.easingCubicIn,
      'cubicOut' => l10n.easingCubicOut,
      'cubicInOut' => l10n.easingCubicInOut,
      'quartIn' => l10n.easingQuartIn,
      'quartOut' => l10n.easingQuartOut,
      'quartInOut' => l10n.easingQuartInOut,
      'expoIn' => l10n.easingExpoIn,
      'expoOut' => l10n.easingExpoOut,
      'expoInOut' => l10n.easingExpoInOut,
      'backIn' => l10n.easingBackIn,
      'backOut' => l10n.easingBackOut,
      _ => l10n.easingBackInOut,
    };

/// The house shapes, one row along the top of the library.
const _basics = ['easy', 'slowStart', 'slowFinish', 'heavy', 'snap'];

/// The families, one row each, with their in, out and in out shapes across.
const _families = ['sine', 'quad', 'cubic', 'quart', 'expo', 'back'];

String _familyName(String id) => switch (id) {
      'sine' => l10n.easingFamilySine,
      'quad' => l10n.easingFamilyQuad,
      'cubic' => l10n.easingFamilyCubic,
      'quart' => l10n.easingFamilyQuart,
      'expo' => l10n.easingFamilyExpo,
      _ => l10n.easingFamilyBack,
    };

EasingCurve _presetCurve(String id) =>
    easingPresets.firstWhere((p) => p.id == id).curve;

/// Side room: enough for a knob at x = 0 or x = 1 to be drawn whole.
const double _marginX = 14;

/// How near the pointer must come to a handle to take hold of it.
const double _grabRadius = 14;

/// The unit box is wider than it is tall, as a graph is. Its height is this
/// share of its width, unless the panel is too short to give it that.
const double _boxAspect = 0.55;
const double _boxMinHeight = 56;

/// The popup's content width.
const double _popupWidth = 236;

/// The strip under the box that plays the shape.
const double _previewHeight = 16;

/// A panel shorter than this scrolls as one, rather than squeezing the
/// library to nothing under a box that will not shrink.
const double _pinnedMinHeight = 380;

/// Wide enough, and wide rather than tall: the library goes beside the box.
const double _sideBySideMinWidth = 520;

const double _gap = 6;

/// Where the unit box sits inside the drawing. The drawing is the box plus
/// room above and below for overshoot, sized from [easingHandleReach] so a
/// handle at the very limit still draws, and can still be taken hold of.
class _Geometry {
  final double width;
  final Rect box;

  const _Geometry._(this.width, this.box);

  factory _Geometry.fit(double width, double maxHeight) {
    final across = width - 2 * _marginX;
    final tallest = maxHeight / (1 + 2 * easingHandleReach);
    final h = (across * _boxAspect)
        .clamp(_boxMinHeight, tallest < _boxMinHeight ? _boxMinHeight : tallest)
        .toDouble();
    return _Geometry._(
      width,
      Rect.fromLTWH(_marginX, h * easingHandleReach, across, h),
    );
  }

  double get height => box.height * (1 + 2 * easingHandleReach);
}

/// Open the easing editor at [position] as a popup, and call [onApply] with
/// the shape each time Apply is pressed. It stays up across an Apply, and
/// closes on Close or a click outside it.
Future<void> showEasingPopup({
  required BuildContext context,
  required Offset position,
  required ApplyEasing onApply,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => Builder(builder: (context) {
        final t = ThemeScope.of(context).theme;
        return Container(
          decoration: BoxDecoration(
            color: t.surface2,
            borderRadius: BorderRadius.circular(t.tokens.floatRadius),
            border: Border.all(color: t.hairlineStrong),
          ),
          child: SizedBox(
            width: _popupWidth + 16,
            child: EasingEditor(
              onApply: onApply,
              onClose: () => close(null),
            ),
          ),
        );
      }),
    );

/// The editor body: the box, its numbers and buttons, and the library.
///
/// [onClose] null means there is no Close button. [onApply] null means Apply
/// is there but greyed, with [whyNot] as its tip. [span] is the shape of the
/// selected keys, for the Read button; null leaves the button out.
/// [footer] stands under the buttons, where the panel puts the selected key.
class EasingEditor extends StatefulWidget {
  final ApplyEasing? onApply;
  final VoidCallback? onClose;
  final String? whyNot;
  final ValueListenable<EasingCurve?>? span;
  final Widget? footer;

  const EasingEditor({
    super.key,
    required this.onApply,
    this.onClose,
    this.whyNot,
    this.span,
    this.footer,
  });

  @override
  State<EasingEditor> createState() => _EasingEditorState();
}

class _EasingEditorState extends State<EasingEditor>
    with SingleTickerProviderStateMixin {
  /// It opens on the gentlest preset, and the shape survives a selection
  /// change: the panel keeps this State for its whole life.
  EasingCurve _curve = easingPresets.first.curve;

  /// Which handle the pointer has hold of, or is over: 1, 2 or null.
  int? _dragging;
  int? _hovering;

  /// Apply as you shape: a handle let go, a number typed or Reverse applies
  /// at once, as a preset does.
  bool _live = false;

  /// Which library tab is open: null until one is picked, which opens on
  /// your own shapes when there are any.
  bool? _yours;

  /// What the name field is for: null when it is not there, the empty string
  /// for saving the drawn shape, a saved shape's name for renaming that one.
  String? _naming;

  final TextEditingController _nameField = TextEditingController();

  /// The preview: one pass of the shape, 0 to 1 in time.
  late final AnimationController _play = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 900),
  );

  _Geometry _geom = _Geometry.fit(_popupWidth, 200);

  @override
  void dispose() {
    _play.dispose();
    _nameField.dispose();
    super.dispose();
  }

  /// A new shape in the box, played once so it can be seen moving.
  void _show(EasingCurve curve) {
    setState(() => _curve = curve);
    if (ThemeScope.of(context).animationLevel != AnimationLevel.none) {
      _play.forward(from: 0);
    }
  }

  /// The box's shape sent to the keys, when Apply as you shape is on.
  void _applyIfLive() {
    if (_live) widget.onApply?.call(_curve, EaseSides.both);
  }

  void _startNaming(String forName) => setState(() {
        _naming = forName;
        _nameField.text = forName;
      });

  /// Enter, or a click away: keep what was typed. A blank name keeps nothing.
  void _commitName() {
    final job = _naming;
    if (job == null) return;
    setState(() {
      if (job.isEmpty) {
        if (CustomEasings.add(_nameField.text, _curve) != null) _yours = true;
      } else {
        CustomEasings.rename(job, _nameField.text);
      }
      _naming = null;
    });
  }

  /// A control point in the drawing's coordinates. y is flipped: the curve
  /// rises as the value completes, and screen y grows downward.
  Offset _pointOf(int handle) {
    final b = _geom.box;
    final x = handle == 1 ? _curve.x1 : _curve.x2;
    final y = handle == 1 ? _curve.y1 : _curve.y2;
    return Offset(b.left + x * b.width, b.bottom - y * b.height);
  }

  int? _handleAt(Offset local) {
    final d1 = (local - _pointOf(1)).distance;
    final d2 = (local - _pointOf(2)).distance;
    if (d1 > _grabRadius && d2 > _grabRadius) return null;
    return d1 <= d2 ? 1 : 2;
  }

  void _grab(Offset local) {
    final handle = _handleAt(local);
    if (handle == null) return;
    setState(() => _dragging = handle);
    _drag(local);
  }

  /// Shift snaps to twentieths, Alt mirrors the other handle round the
  /// middle, which keeps an in out shape even.
  void _drag(Offset local) {
    final handle = _dragging;
    if (handle == null) return;
    final b = _geom.box;
    var x = (local.dx - b.left) / b.width;
    var y = (b.bottom - local.dy) / b.height;
    final keys = HardwareKeyboard.instance;
    if (keys.isShiftPressed) {
      x = (x * 20).round() / 20;
      y = (y * 20).round() / 20;
    }
    var c = _curve.withHandle(first: handle == 1, x: x, y: y);
    if (keys.isAltPressed) {
      c = handle == 1
          ? EasingCurve(c.x1, c.y1, 1 - c.x1, 1 - c.y1)
          : EasingCurve(1 - c.x2, 1 - c.y2, c.x2, c.y2);
    }
    setState(() => _curve = c);
  }

  void _release() {
    if (_dragging == null) return;
    setState(() => _dragging = null);
    _show(_curve);
    _applyIfLive();
  }

  /// The four numbers as CSS writes them, which is what most tools paste.
  String get _css => 'cubic-bezier(${[
        _curve.x1,
        _curve.y1,
        _curve.x2,
        _curve.y2
      ].map((v) => v.toStringAsFixed(2)).join(', ')})';

  /// Any four numbers on the clipboard, in CSS or bare.
  Future<void> _paste() async {
    final text = (await Clipboard.getData(Clipboard.kTextPlain))?.text ?? '';
    final n = RegExp(r'-?\d*\.?\d+')
        .allMatches(text)
        .map((m) => double.parse(m[0]!))
        .toList();
    if (n.length != 4 || !mounted) return;
    _show(EasingCurve(n[0], n[1], n[2], n[3]));
    _applyIfLive();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LayoutBuilder(builder: (context, c) {
      // The popup: no height to fill, so a fixed width and a short library.
      if (!c.hasBoundedHeight) {
        return Padding(
          padding: const EdgeInsets.all(8),
          child: SizedBox(
            width: _popupWidth,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                ..._editor(t, _popupWidth, 210),
                const SizedBox(height: _gap),
                ConstrainedBox(
                  constraints: const BoxConstraints(maxHeight: 220),
                  child: SingleChildScrollView(child: _library(t, _popupWidth)),
                ),
              ],
            ),
          ),
        );
      }
      final w = c.maxWidth - 16;
      final h = c.maxHeight - 16;
      final footer = widget.footer;
      if (w >= _sideBySideMinWidth && w >= h * 1.25) {
        final left = (w * 0.42).clamp(240.0, 380.0).toDouble();
        return Padding(
          padding: const EdgeInsets.all(8),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(
                width: left,
                child: SingleChildScrollView(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      ..._editor(t, left, h - 76),
                      if (footer != null) footer,
                    ],
                  ),
                ),
              ),
              const SizedBox(width: 10),
              Container(width: 1, color: t.hairline),
              const SizedBox(width: 10),
              Expanded(
                child: SingleChildScrollView(
                  child: Align(
                    alignment: Alignment.topLeft,
                    child: LayoutBuilder(
                        builder: (context, lc) => _library(t, lc.maxWidth)),
                  ),
                ),
              ),
            ],
          ),
        );
      }
      if (h < _pinnedMinHeight) {
        return SingleChildScrollView(
          child: Padding(
            padding: const EdgeInsets.all(8),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                ..._editor(t, w, 220),
                if (footer != null) footer,
                const SizedBox(height: _gap),
                _library(t, w),
              ],
            ),
          ),
        );
      }
      // Tall: the box and its buttons stay put, and the selected key and the
      // library scroll under them.
      return Padding(
        padding: const EdgeInsets.fromLTRB(8, 8, 8, 0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ..._editor(t, w, h * 0.38),
            Expanded(
              child: SingleChildScrollView(
                child: Padding(
                  padding: const EdgeInsets.only(bottom: 8),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      if (footer != null) footer,
                      _library(t, w),
                    ],
                  ),
                ),
              ),
            ),
          ],
        ),
      );
    });
  }

  /// The box, the preview under it, the four numbers and the buttons.
  List<Widget> _editor(LumitTheme t, double width, double maxHeight) {
    final g = _geom = _Geometry.fit(width, maxHeight);
    final apply = widget.onApply;
    return [
      // The empty drag handlers put the box in the gesture arena, where it
      // beats a scroll view round it: a handle drag must shape the curve, not
      // scroll it away. The Listener does the work, from the first pointer-down.
      HouseContextMenu(
        itemBuilder: (close) => [
          MenuRow(
            onPressed: () {
              close();
              Clipboard.setData(ClipboardData(text: _css));
            },
            child: Text(l10n.easingCopyCss, style: t.small),
          ),
          MenuRow(
            onPressed: () {
              close();
              _paste();
            },
            child: Text(l10n.menuPaste, style: t.small),
          ),
        ],
        child: MouseRegion(
          cursor: _dragging != null || _hovering != null
              ? SystemMouseCursors.grab
              : MouseCursor.defer,
          onHover: (e) {
            final over = _handleAt(e.localPosition);
            if (over != _hovering) setState(() => _hovering = over);
          },
          onExit: (_) {
            if (_hovering != null) setState(() => _hovering = null);
          },
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onVerticalDragUpdate: (_) {},
            onHorizontalDragUpdate: (_) {},
            child: Listener(
              key: const ValueKey('easing-box'),
              onPointerDown: (e) {
                if (e.buttons == kPrimaryButton) _grab(e.localPosition);
              },
              onPointerMove: (e) => _drag(e.localPosition),
              onPointerUp: (_) => _release(),
              onPointerCancel: (_) => _release(),
              child: AnimatedBuilder(
                animation: _play,
                builder: (context, _) => CustomPaint(
                  size: Size(g.width, g.height),
                  painter: EasingBoxPainter(
                    curve: _curve,
                    box: g.box,
                    theme: t,
                    lit: _dragging ?? _hovering,
                    playhead: _play.isAnimating ? _play.value : null,
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
      // The shape played: the accent dot eases, the faint one keeps time.
      // Hovering it plays it again.
      MouseRegion(
        onEnter: (_) => _play.forward(from: 0),
        child: AnimatedBuilder(
          animation: _play,
          builder: (context, _) => CustomPaint(
            key: const ValueKey('easing-preview'),
            size: Size(g.width, _previewHeight),
            painter: EasingPreviewPainter(
              curve: _curve,
              box: g.box,
              theme: t,
              time: _play.isAnimating ? _play.value : 0,
            ),
          ),
        ),
      ),
      const SizedBox(height: _gap),
      _numbers(t),
      const SizedBox(height: _gap),
      Row(
        children: [
          if (widget.span != null)
            ValueListenableBuilder<EasingCurve?>(
              valueListenable: widget.span!,
              builder: (context, span, _) => _iconButton(
                t,
                key: 'easing-read',
                glyph: LumitIcons.eyedropper,
                tip: l10n.easingRead,
                onPressed: span == null ? null : () => _show(span),
              ),
            ),
          _iconButton(
            t,
            key: 'easing-reverse',
            glyph: LumitIcons.loop,
            tip: l10n.easingReverse,
            onPressed: () {
              _show(_curve.reversed);
              _applyIfLive();
            },
          ),
          _iconButton(
            t,
            key: 'easing-live',
            glyph: LumitIcons.link,
            tip: l10n.easingLiveApply,
            active: _live,
            onPressed: () => setState(() => _live = !_live),
          ),
          _iconButton(
            t,
            key: 'easing-save',
            glyph: LumitIcons.star,
            tip: l10n.easingSaveEllipsis,
            onPressed: () => _startNaming(''),
          ),
          Builder(
            builder: (button) => _iconButton(
              t,
              key: 'easing-more',
              glyph: LumitIcons.more,
              tip: l10n.easingMore,
              onPressed: () => _showMore(t, button),
            ),
          ),
          const Spacer(),
          if (widget.onClose != null) ...[
            HouseButton(
              small: true,
              onPressed: widget.onClose,
              child: Text(l10n.close, style: t.small),
            ),
            const SizedBox(width: _gap),
          ],
          _maybeTip(
            apply == null ? widget.whyNot : null,
            HouseButton(
              key: const ValueKey('easing-apply'),
              small: true,
              primary: true,
              onPressed:
                  apply == null ? null : () => apply(_curve, EaseSides.both),
              child: Text(l10n.apply,
                  style: t.small.copyWith(
                      color: apply == null ? t.textDisabled : t.textPrimary)),
            ),
          ),
        ],
      ),
      if (_naming != null) ...[
        const SizedBox(height: _gap),
        HouseTextField(
          key: const ValueKey('easing-name'),
          controller: _nameField,
          width: width,
          autofocus: true,
          hint: l10n.easingNameYours,
          onSubmitted: (_) => _commitName(),
          onTapOutside: _commitName,
          onCancelled: () => setState(() => _naming = null),
        ),
      ],
      const SizedBox(height: _gap),
    ];
  }

  /// Import and export: a collection of shapes is a file to pass round.
  void _showMore(LumitTheme t, BuildContext button) {
    final box = button.findRenderObject();
    if (box is! RenderBox) return;
    Widget row(String label, Future<void> Function() run, VoidCallback close) =>
        MenuRow(
          onPressed: () {
            close();
            run();
          },
          child: Text(label, style: t.small),
        );
    showMenuAt<void>(
      context: context,
      position: box.localToGlobal(Offset(0, box.size.height)),
      rows: (close) => [
        row(l10n.easingImport, () async {
          final path = await pickEasingsToOpen();
          if (path != null && CustomEasings.importFrom(path) > 0 && mounted) {
            setState(() => _yours = true);
          }
        }, () => close(null)),
        if (CustomEasings.all.isNotEmpty)
          row(l10n.easingExport, () async {
            final path = await pickEasingsSaveLocation();
            if (path != null) CustomEasings.exportTo(path);
          }, () => close(null)),
      ],
    );
  }

  Widget _maybeTip(String? tip, Widget child) =>
      tip == null ? child : LumitTooltip(message: tip, child: child);

  Widget _iconButton(
    LumitTheme t, {
    required String key,
    required String glyph,
    required String tip,
    required VoidCallback? onPressed,
    bool active = false,
  }) =>
      LumitTooltip(
        message: tip,
        child: HouseButton(
          key: ValueKey(key),
          small: true,
          frameless: true,
          active: active,
          padding: const EdgeInsets.all(3),
          onPressed: onPressed,
          child: LumitIcon(
            glyph,
            colour: onPressed == null
                ? t.textDisabled
                : active
                    ? t.accent
                    : t.textSecondary,
          ),
        ),
      );

  /// Each handle's point in the box, to read off or type: x is how far it
  /// reaches along the time, y how high it lifts.
  Widget _numbers(LumitTheme t) {
    Widget field(String key, double value, double min, double max,
            EasingCurve Function(double v) next) =>
        Expanded(
          child: DragValueField(
            key: ValueKey(key),
            value: value,
            min: min,
            max: max,
            decimals: 2,
            speed: 0.01,
            onChanged: (v) {
              _show(next(v.toDouble()));
              _applyIfLive();
            },
          ),
        );
    Widget label(String text) => Padding(
          padding: const EdgeInsets.symmetric(horizontal: 4),
          child: Text(t.kickerCase(text), style: t.kicker),
        );
    final c = _curve;
    const reach = easingHandleReach;
    return Row(
      children: [
        label(l10n.graphEaseOut),
        field('easing-x1', c.x1, minTangentReach, 1,
            (v) => EasingCurve(v, c.y1, c.x2, c.y2)),
        const SizedBox(width: 2),
        field('easing-y1', c.y1, -reach, 1 + reach,
            (v) => EasingCurve(c.x1, v, c.x2, c.y2)),
        const SizedBox(width: _gap),
        label(l10n.graphEaseIn),
        field('easing-x2', c.x2, 0, 1 - minTangentReach,
            (v) => EasingCurve(c.x1, c.y1, v, c.y2)),
        const SizedBox(width: 2),
        field('easing-y2', c.y2, -reach, 1 + reach,
            (v) => EasingCurve(c.x1, c.y1, c.x2, v)),
      ],
    );
  }

  /// The library, in two tabs: the shapes that ship and the person's own. The
  /// tabs only show once something is saved, and open on your own when there
  /// is any, so someone who only uses theirs never scrolls past the rest.
  ///
  /// **A click is an apply.** A tile loads its shape into the box and presses
  /// the same road Apply does, so easing a selection is one click. Shift or
  /// Ctrl eases one side only ([EaseSides]). While there is nowhere to send a
  /// shape a click only loads.
  Widget _library(LumitTheme t, double available) {
    // Past this a wider panel only stretches the thumbnails.
    final width = available.clamp(0.0, 480.0).toDouble();
    final saved = CustomEasings.all;
    final yours = saved.isNotEmpty && (_yours ?? true);
    Widget tab(String key, String label, bool on) => HouseButton(
          key: ValueKey(key),
          small: true,
          active: on,
          onPressed: () => setState(() => _yours = key == 'easing-tab-yours'),
          child: Text(label,
              style: t.small.copyWith(color: on ? t.textPrimary : null)),
        );
    return SizedBox(
      width: width,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (saved.isNotEmpty) ...[
            Row(
              children: [
                tab('easing-tab-built-in', l10n.easingBuiltIn, !yours),
                const SizedBox(width: 3),
                tab('easing-tab-yours', l10n.easingYours, yours),
              ],
            ),
            const SizedBox(height: _gap),
          ],
          yours ? _saved(t, width, saved) : _builtIn(t, width),
        ],
      ),
    );
  }

  /// The house shapes along the top, then each family as a row with its in,
  /// out and in out shapes across.
  Widget _builtIn(LumitTheme t, double width) {
    const labelWidth = 44.0;
    final cell = (width - labelWidth - 2 * 3) / 3;
    final thumb = (cell * 0.42).clamp(22.0, 40.0).toDouble();
    final basic = (width - 4 * 3) / 5;
    Widget head(String text) => Expanded(
          child: Text(text,
              textAlign: TextAlign.center,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: t.caption.copyWith(color: t.textMuted)),
        );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            for (final id in _basics) ...[
              if (id != _basics.first) const SizedBox(width: 3),
              Expanded(
                child: _tile(t, 'easing-preset-$id', easingPresetName(id),
                    _presetCurve(id),
                    height: thumb, caption: basic >= 60),
              ),
            ],
          ],
        ),
        const SizedBox(height: 10),
        Row(
          children: [
            const SizedBox(width: labelWidth + 3),
            head(l10n.easingColumnIn),
            const SizedBox(width: 3),
            head(l10n.easingColumnOut),
            const SizedBox(width: 3),
            head(l10n.easingColumnInOut),
          ],
        ),
        const SizedBox(height: 3),
        for (final family in _families)
          Padding(
            padding: const EdgeInsets.only(bottom: 3),
            child: Row(
              children: [
                SizedBox(
                  width: labelWidth,
                  child: Text(_familyName(family),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: t.small.copyWith(color: t.textSecondary)),
                ),
                for (final variant in const ['In', 'Out', 'InOut']) ...[
                  const SizedBox(width: 3),
                  Expanded(
                    child: _tile(
                        t,
                        'easing-preset-$family$variant',
                        easingPresetName('$family$variant'),
                        _presetCurve('$family$variant'),
                        height: thumb),
                  ),
                ],
              ],
            ),
          ),
      ],
    );
  }

  /// The person's own shapes, named, with Rename and Delete on a right-click.
  Widget _saved(LumitTheme t, double width, List<SavedEasing> saved) {
    final cell = (width - 2 * 3) / 3;
    final thumb = (cell * 0.42).clamp(22.0, 40.0).toDouble();
    return Wrap(
      spacing: 3,
      runSpacing: 3,
      children: [
        for (final s in saved)
          SizedBox(
            width: cell,
            child: HouseContextMenu(
              itemBuilder: (close) => [
                MenuRow(
                  onPressed: () {
                    close();
                    _startNaming(s.name);
                  },
                  child: Text(l10n.rename, style: t.small),
                ),
                MenuRow(
                  onPressed: () {
                    close();
                    setState(() => CustomEasings.delete(s.name));
                  },
                  child: Text(l10n.delete, style: t.small),
                ),
              ],
              child: _tile(t, 'easing-saved-${s.name}', s.name, s.curve,
                  height: thumb, caption: true),
            ),
          ),
      ],
    );
  }

  Widget _tile(LumitTheme t, String key, String name, EasingCurve curve,
      {required double height, bool caption = false}) {
    final current = _curve == curve;
    final tile = HouseButton(
      key: ValueKey(key),
      small: true,
      frameless: true,
      active: current,
      padding: const EdgeInsets.all(2),
      onPressed: () {
        _show(curve);
        final keys = HardwareKeyboard.instance;
        widget.onApply?.call(
            curve,
            keys.isShiftPressed
                ? EaseSides.into
                : keys.isControlPressed
                    ? EaseSides.outOf
                    : EaseSides.both);
      },
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          ClipRect(
            child: CustomPaint(
              size: Size.fromHeight(height),
              painter:
                  EasingTilePainter(curve: curve, theme: t, current: current),
            ),
          ),
          if (caption)
            Padding(
              padding: const EdgeInsets.only(top: 1),
              child: Text(
                name,
                textAlign: TextAlign.center,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style:
                    t.caption.copyWith(color: current ? t.accent : t.textMuted),
              ),
            ),
        ],
      ),
    );
    return caption ? tile : LumitTooltip(message: name, child: tile);
  }
}
