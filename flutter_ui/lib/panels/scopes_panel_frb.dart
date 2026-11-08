// The Scopes panel, on the flutter_rust_bridge API.
//
// One of the four instruments, waveform, parade, vectorscope or histogram, of
// the frame at the playhead. The binning runs on the GPU and only the finished
// 256x256 picture crosses, which is why a scope costs a fraction of what
// reading the frame back would.
//
// **Why it asks for its own render.** The trace needs CPU pixels, and the
// zero-copy Viewer paths never bring any back — so a scope cannot borrow the
// picture on screen and has to render its own. That is real work per trace, so
// this asks once per new picture rather than once per rebuild, and nothing
// happens at all while the panel is not on screen.

import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'placeholder.dart';

/// The four instruments the picker offers.
enum ScopeKind { waveform, parade, vectorscope, histogram }

/// The engine's fixed trace size.
const int _traceEdge = 256;

/// The engine's codes: 0 luma waveform, 1 RGB waveform, 2 vectorscope, 3 RGB
/// histogram, 4 parade, 5 luma histogram.
int _engineKind(ScopeKind kind, {required bool rgb}) => switch (kind) {
      ScopeKind.waveform => rgb ? 1 : 0,
      ScopeKind.parade => 4,
      ScopeKind.vectorscope => 2,
      ScopeKind.histogram => rgb ? 3 : 5,
    };

/// Which kinds have a luma or RGB reading to choose between. The parade is
/// RGB by nature and the vectorscope reads chroma alone.
bool _hasChannelPick(ScopeKind kind) =>
    kind == ScopeKind.waveform || kind == ScopeKind.histogram;

/// The room a trace leaves round itself for the scale: levels down the left
/// of a waveform, code values under a histogram. Wide enough that "100" in
/// the scale's type clears the panel's edge.
const double _gutter = 26;
const double _margin = 6;

class ScopesPanelFrb extends StatefulWidget {
  const ScopesPanelFrb({super.key});

  @override
  State<ScopesPanelFrb> createState() => _ScopesPanelFrbState();
}

class _ScopesPanelFrbState extends State<ScopesPanelFrb> {
  ScopeKind _kind = ScopeKind.waveform;

  /// Whether the waveform and the histogram read each channel rather than
  /// the brightness.
  bool _rgb = false;
  bool _graticule = true;
  ui.Image? _trace;

  /// The engine kind the trace on screen was made for, so a trace of another
  /// kind (the Levels effect asks for histograms on the same stream) is
  /// never shown here.
  int _traceKind = -1;
  StreamSubscription<WorkerResponse>? _responses;

  int _lastFrame = -1;

  /// The value of [LumitUiState.frameArrived] the last request was made at.
  /// Part of the memo with [_lastFrame], because a new *picture* of the frame
  /// already traced is exactly as worth tracing as a new frame number — a
  /// value drag holds the playhead still and changes the picture under it.
  int _lastArrival = -1;

  late final LumitUiState _ui;

  int get _asked => _engineKind(_kind, rgb: _rgb);

  @override
  void initState() {
    super.initState();
    final state = Provider.of<LumitState>(context, listen: false);
    _responses = state.onWorkerResponse.listen(_onResponse);
    // The two facts worth a trace, listened to rather than read in `build`:
    // the playhead moving, and a picture reaching the Viewer. Nothing a
    // rebuild does on its own asks the engine for anything.
    _ui = Provider.of<LumitUiState>(context, listen: false);
    _ui.playheadFrame.addListener(_request);
    _ui.frameArrived.addListener(_request);
    WidgetsBinding.instance.addPostFrameCallback((_) => _request());
  }

  @override
  void dispose() {
    _ui.playheadFrame.removeListener(_request);
    _ui.frameArrived.removeListener(_request);
    _responses?.cancel();
    _trace?.dispose();
    super.dispose();
  }

  /// Scope traces ride the same worker stream as the frames, so this ignores
  /// everything that is not one, and every trace that is not this panel's.
  /// The Levels effect's own histogram asks on the same stream, and before
  /// this test it landed here as a white bell over the waveform.
  void _onResponse(WorkerResponse response) {
    if (response is! WorkerResponse_Scope) return;
    if (response.field0.kind != _asked) return;
    ui.decodeImageFromPixels(
      Uint8List.fromList(response.field0.rgba),
      _traceEdge,
      _traceEdge,
      ui.PixelFormat.rgba8888,
      (image) {
        if (!mounted) {
          image.dispose();
          return;
        }
        setState(() {
          _trace?.dispose();
          _trace = image;
          _traceKind = response.field0.kind;
        });
      },
    );
  }

  void _pick({ScopeKind? kind, bool? rgb}) {
    setState(() {
      _kind = kind ?? _kind;
      _rgb = rgb ?? _rgb;
    });
    // A new trace kind is worth a request of its own: neither the frame nor
    // the picture of it has moved, so nothing else here would ask.
    _request(force: true);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui_ = Provider.of<LumitUiState>(context);
    final comp = ui_.selectedComp;
    if (comp == null) {
      return PlaceholderPanel(
        icon: LumitIcon.graphCurve,
        title: l10n.panelScopes,
        hint: l10n.selectACompositionFirst,
      );
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _header(t),
        Expanded(child: _body(t, ui_)),
      ],
    );
  }

  Widget _header(LumitTheme t) => Container(
        height: 26,
        color: t.surface1,
        padding: const EdgeInsets.symmetric(horizontal: 6),
        // Scrolls sideways when docked narrow, the same answer the Timeline
        // toolbar gives, as an overflow stripe is a layout fault.
        child: SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: Row(
            children: [
              SizedBox(
                width: 110,
                child: BareDropdown<ScopeKind>(
                  key: const ValueKey('scope-kind'),
                  value: _kind,
                  options: ScopeKind.values,
                  label: scopeKindLabel,
                  onChanged: (k) => _pick(kind: k),
                ),
              ),
              if (_hasChannelPick(_kind)) ...[
                const SizedBox(width: 8),
                _channelButton(l10n.scopeLuma, rgb: false),
                const SizedBox(width: 2),
                _channelButton(l10n.scopeRgb, rgb: true),
              ],
            ],
          ),
        ),
      );

  /// The word takes the button's own colour: on Lantern's filled pill the
  /// panel's small style vanished into the accent.
  Widget _channelButton(String word, {required bool rgb}) => HouseButton(
        key: ValueKey<String>('scope-${rgb ? 'rgb' : 'luma'}'),
        small: true,
        frameless: _rgb != rgb,
        active: _rgb == rgb,
        onPressed: () => _pick(rgb: rgb),
        child: Text(word),
      );

  Widget _body(LumitTheme t, LumitUiState state) {
    final themed = state.workspace.themedScopes;
    final colours = themed ? null : ScopeColours.standard;
    final plotFill = colours?.bg ?? t.surface0;
    final line = colours?.graticule ?? t.hairlineStrong;
    // The trace on screen only while it is the one asked for, so a change of
    // instrument shows an empty plot for the moment, never the old picture
    // under the new scale.
    final trace = _traceKind == _asked ? _trace : null;
    return _ScopeMenu(
      kind: _kind,
      rgb: _rgb,
      graticule: _graticule,
      themed: themed,
      onKind: (k) => _pick(kind: k),
      onRgb: (on) => _pick(rgb: on),
      onGraticule: (on) => setState(() => _graticule = on),
      onThemed: (on) {
        state.workspace.setThemedScopes(on);
        _pick();
      },
      child: Container(
        color: t.surface0,
        child: LayoutBuilder(
          builder: (context, box) {
            final plot = _plotRect(box.biggest);
            return Stack(
              fit: StackFit.expand,
              children: [
                Positioned.fromRect(
                  rect: plot,
                  child: ColoredBox(
                    color: plotFill,
                    child: trace == null
                        ? null
                        : RawImage(
                            key: const ValueKey('scope-trace'),
                            image: trace,
                            fit: BoxFit.fill,
                            // A 256-column trace stretched across a wide
                            // panel: bilinear gives a phosphor's glow,
                            // nearest gives bars.
                            filterQuality: FilterQuality.low,
                          ),
                  ),
                ),
                if (_graticule)
                  CustomPaint(
                    key: const ValueKey('scope-graticule'),
                    painter: _GraticulePainter(
                      kind: _kind,
                      plot: plot,
                      line: line,
                      label: t.mono.copyWith(fontSize: 9, color: t.textMuted),
                    ),
                  ),
              ],
            );
          },
        ),
      ),
    );
  }

  /// Where the trace is drawn: the panel less the scale's gutters, and for
  /// the vectorscope the largest square that fits, centred.
  Rect _plotRect(Size size) {
    final left = _kind == ScopeKind.vectorscope ? _margin : _gutter;
    final bottom = _kind == ScopeKind.histogram ? 14.0 : _margin;
    final inner = Rect.fromLTRB(
        left, _margin, size.width - _margin, size.height - bottom);
    if (_kind != ScopeKind.vectorscope) return inner;
    final side = math.max(0.0, inner.shortestSide);
    return Rect.fromCenter(center: inner.center, width: side, height: side);
  }

  /// Ask for a trace when the frame has moved, or when a new picture of it has
  /// reached the Viewer. The engine traces the picture the Viewer is showing,
  /// a drag's preview included, so the trace follows the grade in hand.
  ///
  /// Never from `build`: the playhead and the arrival count call it, and the
  /// reply arrives on the worker stream. Those two numbers are the whole memo:
  /// the same frame and the same picture of it is nothing to ask for, so a
  /// hover, a resize or the trace landing costs no render. [force] is how
  /// the pickers ask for a new kind of the same picture.
  void _request({bool force = false}) {
    if (!mounted) return;
    final comp = _ui.selectedComp;
    if (comp == null) return;
    final frame = _ui.playheadFrame.value;
    final arrived = _ui.frameArrived.value;
    if (!force && frame == _lastFrame && arrived == _lastArrival) return;

    _lastFrame = frame;
    _lastArrival = arrived;
    final t = ThemeScope.of(context).theme;
    try {
      comp.renderScope(
        frame: frame,
        scale: _ui.measuredScale,
        kind: _asked,
        colours: scopeColoursFor(t, themed: _ui.workspace.themedScopes),
        view: _ui.measuredViewId,
      );
    } catch (_) {
      // A comp that went away under the panel: the plot stays as it was.
    }
  }
}

/// What the picker and the menu call each instrument.
String scopeKindLabel(ScopeKind kind) => switch (kind) {
      ScopeKind.waveform => l10n.foldWaveform,
      ScopeKind.parade => l10n.scopeParade,
      ScopeKind.vectorscope => l10n.scopeVectorscope,
      ScopeKind.histogram => l10n.scopeHistogram,
    };

/// The panel's right-click menu: the instrument, its reading, and what it is
/// read against.
class _ScopeMenu extends StatelessWidget {
  final ScopeKind kind;
  final bool rgb;
  final bool graticule;
  final bool themed;
  final ValueChanged<ScopeKind> onKind;
  final ValueChanged<bool> onRgb;
  final ValueChanged<bool> onGraticule;
  final ValueChanged<bool> onThemed;
  final Widget child;

  const _ScopeMenu({
    required this.kind,
    required this.rgb,
    required this.graticule,
    required this.themed,
    required this.onKind,
    required this.onRgb,
    required this.onGraticule,
    required this.onThemed,
    required this.child,
  });

  @override
  Widget build(BuildContext context) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onSecondaryTapDown: (d) => _open(context, d.globalPosition),
        child: child,
      );

  void _open(BuildContext context, Offset at) {
    final picks = _hasChannelPick(kind);
    showLumitPopup<void>(
      context: context,
      position: at,
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final k in ScopeKind.values)
                MenuRow.option(
                  key: ValueKey<String>('scope-menu-${k.name}'),
                  selected: k == kind,
                  onPressed: () => onKind(k),
                  child: MenuLabel(scopeKindLabel(k), ticked: k == kind),
                ),
              const MenuDivider(),
              // Greyed where the instrument has one reading, so the rows keep
              // their place and the eye learns where to find them.
              MenuRow.option(
                key: const ValueKey('scope-menu-luma'),
                enabled: picks,
                selected: picks && !rgb,
                onPressed: () => onRgb(false),
                child: MenuLabel(l10n.scopeLuma,
                    ticked: picks && !rgb, enabled: picks),
              ),
              MenuRow.option(
                key: const ValueKey('scope-menu-rgb'),
                enabled: picks,
                selected: picks && rgb,
                onPressed: () => onRgb(true),
                child: MenuLabel(l10n.scopeRgb,
                    ticked: picks && rgb, enabled: picks),
              ),
              const MenuDivider(),
              MenuRow.option(
                key: const ValueKey('scope-menu-graticule'),
                onPressed: () => onGraticule(!graticule),
                child: MenuLabel(l10n.scopeGraticule, ticked: graticule),
              ),
              MenuRow.option(
                key: const ValueKey('scope-menu-themed'),
                onPressed: () => onThemed(!themed),
                child: MenuLabel(l10n.scopeThemeColours, ticked: themed),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Background, trace, then the R, G and B tints — the five triples the engine
/// takes.
///
/// **Standard by default**. A waveform or vectorscope is a measuring
/// instrument, and it is read on a near-black graticule with a bright trace
/// whatever the chrome around it is doing — the same grading-accuracy
/// reasoning that keeps the Viewer's surround neutral (docs/15-DESIGN §8,
/// §2.1). `ScopeColours.standard` is that fixed set.
///
/// With [themed] on, the scope takes the theme's own colours instead —
/// off-spec, opt-in, and squarely a matter of taste rather than of reading a
/// signal accurately.
List<Uint8List> scopeColoursFor(LumitTheme t, {bool themed = false}) {
  Uint8List rgb(Color c) => Uint8List.fromList([
        (c.r * 255).round(),
        (c.g * 255).round(),
        (c.b * 255).round(),
      ]);
  if (!themed) {
    const s = ScopeColours.standard;
    return [rgb(s.bg), rgb(s.trace), rgb(s.red), rgb(s.green), rgb(s.blue)];
  }
  return [
    rgb(t.surface0),
    rgb(t.textPrimary),
    rgb(t.layer.footage),
    rgb(t.layer.solid),
    rgb(t.accent),
  ];
}

/// The scale a trace is read against: IRE for the waveform and parade, the
/// colour targets and skin-tone line for the vectorscope, and the code range
/// for the histogram.
///
/// The engine sends a picture of the measurement, and this draws the ruler
/// over it. Without one a waveform is a shape with no idea what counts as
/// black or white, and a vectorscope is a blob with no idea which way is red.
/// The numbers sit in the gutter beside the plot, never over the trace.
class _GraticulePainter extends CustomPainter {
  final ScopeKind kind;
  final Rect plot;
  final Color line;
  final TextStyle label;

  const _GraticulePainter({
    required this.kind,
    required this.plot,
    required this.line,
    required this.label,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = line
      ..strokeWidth = 1
      ..style = PaintingStyle.stroke;
    switch (kind) {
      // Both read as levels up the side, so both take the same IRE scale.
      // 0 and 100 are the ones that matter — black and white — and they are
      // the two the trace is clipped against.
      case ScopeKind.waveform:
      case ScopeKind.parade:
        for (final ire in const [0, 25, 50, 75, 100]) {
          final y = plot.top + plot.height * (1 - ire / 100);
          canvas.drawLine(Offset(plot.left, y), Offset(plot.right, y), paint);
          _text(canvas, '$ire', Offset(plot.left - 3, y), align: _Align.right);
        }
        // The parade's three cells, so it reads as R, G and B rather than one
        // wide trace. No captions over them: each channel fills its own third
        // of the width and is drawn in its own colour.
        if (kind == ScopeKind.parade) {
          for (var i = 1; i < 3; i++) {
            final x = plot.left + plot.width * i / 3;
            canvas.drawLine(Offset(x, plot.top), Offset(x, plot.bottom), paint);
          }
        }

      // Hue round, saturation out from the middle: the circle is the frame of
      // reference, and without it the trace has no centre and no scale.
      case ScopeKind.vectorscope:
        final centre = plot.center;
        final radius = plot.shortestSide / 2;
        for (final r in const [0.25, 0.5, 0.75, 1.0]) {
          canvas.drawCircle(centre, radius * r, paint);
        }
        canvas.drawLine(
            Offset(centre.dx, plot.top), Offset(centre.dx, plot.bottom), paint);
        canvas.drawLine(
            Offset(plot.left, centre.dy), Offset(plot.right, centre.dy), paint);
        // The six primary and secondary targets at their standard angles,
        // measured anticlockwise from the +x axis, and the skin-tone line at
        // 123 degrees — the one every colourist actually looks for.
        for (final (deg, name) in const [
          (103.0, 'R'),
          (241.0, 'G'),
          (347.0, 'B'),
          (61.0, 'Mg'),
          (167.0, 'Yl'),
          (283.0, 'Cy'),
        ]) {
          final a = deg * math.pi / 180;
          final p = centre +
              Offset(radius * 0.75 * math.cos(a), -radius * 0.75 * math.sin(a));
          canvas.drawCircle(p, 3, paint);
          // The name sits just outside its target, away from the centre.
          final out = p + Offset(10 * math.cos(a), -10 * math.sin(a));
          _text(canvas, name, out, align: _Align.centre);
        }
        final skin = 123.0 * math.pi / 180;
        canvas.drawLine(
          centre,
          centre + Offset(radius * math.cos(skin), -radius * math.sin(skin)),
          paint,
        );

      // Code value across, count up: only the horizontal axis carries meaning,
      // because the vertical is scaled to whatever the tallest bin happens to
      // be and a number on it would be a number about nothing.
      case ScopeKind.histogram:
        for (final (frac, name) in const [
          (0.0, '0'),
          (0.25, '64'),
          (0.5, '128'),
          (0.75, '192'),
          (1.0, '255'),
        ]) {
          final x = plot.left + plot.width * frac;
          canvas.drawLine(Offset(x, plot.top), Offset(x, plot.bottom), paint);
          _text(canvas, name, Offset(x, plot.bottom + 2),
              align: frac == 0
                  ? _Align.left
                  : frac == 1
                      ? _Align.right
                      : _Align.centre,
              below: true);
        }
    }
  }

  /// A label anchored at [at]: centred on its y, and set left, right or
  /// centred on its x; [below] hangs it under the point instead.
  void _text(Canvas canvas, String s, Offset at,
      {required _Align align, bool below = false}) {
    final p = TextPainter(
      text: TextSpan(text: s, style: label),
      textDirection: TextDirection.ltr,
    )..layout();
    final x = switch (align) {
      _Align.left => at.dx,
      _Align.right => at.dx - p.width,
      _Align.centre => at.dx - p.width / 2,
    };
    final y = below ? at.dy : at.dy - p.height / 2;
    p.paint(canvas, Offset(x, y));
  }

  @override
  bool shouldRepaint(_GraticulePainter old) =>
      old.kind != kind || old.line != line || old.plot != plot;

  /// A scale, not a control: clicks fall through to whatever is beneath.
  @override
  bool? hitTest(Offset position) => false;
}

enum _Align { left, right, centre }
