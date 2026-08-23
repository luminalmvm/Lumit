// The Scopes panel, on the flutter_rust_bridge API.
//
// One of the four traces — waveform, parade, vectorscope, histogram — of the
// frame at the playhead. The binning runs on the GPU and only the finished
// 256x256 picture crosses, which is why a scope costs a fraction of what
// reading the frame back would.
//
// **Why it asks for its own render.** The trace needs CPU pixels, and the
// zero-copy Viewer paths never bring any back — so a scope cannot borrow the
// picture on screen and has to render its own. That is real work per trace, so
// this throttles rather than tracing every frame the playhead touches, and
// nothing happens at all while the panel is not on screen.

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

/// The trace codes the engine reads.
enum ScopeKind { waveform, parade, vectorscope, histogram }

/// The engine's fixed trace size.
const int _traceEdge = 256;

class ScopesPanelFrb extends StatefulWidget {
  const ScopesPanelFrb({super.key});

  @override
  State<ScopesPanelFrb> createState() => _ScopesPanelFrbState();
}

class _ScopesPanelFrbState extends State<ScopesPanelFrb> {
  ScopeKind _kind = ScopeKind.waveform;
  ui.Image? _trace;
  StreamSubscription<WorkerResponse>? _responses;

  int _lastFrame = -1;

  /// The value of [LumitUiState.frameArrived] the last request was made at.
  /// Part of the memo with [_lastFrame], because a new *picture* of the frame
  /// already traced is exactly as worth tracing as a new frame number — a
  /// value drag holds the playhead still and changes the picture under it.
  int _lastArrival = -1;

  @override
  void initState() {
    super.initState();
    final state = Provider.of<LumitState>(context, listen: false);
    _responses = state.onWorkerResponse.listen(_onResponse);
  }

  @override
  void dispose() {
    _responses?.cancel();
    _trace?.dispose();
    super.dispose();
  }

  /// Scope traces ride the same worker stream as the frames, so this ignores
  /// everything that is not one.
  void _onResponse(WorkerResponse response) {
    if (response is! WorkerResponse_Scope) return;
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
        });
      },
    );
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

    return ValueListenableBuilder<int>(
      valueListenable: ui_.playheadFrame,
      builder: (context, frame, _) => ValueListenableBuilder<int>(
        // A rendered frame reaching the Viewer is the other reason to trace
        // again: an edit at a stationary playhead moves neither the playhead
        // nor this panel's own state, and without this the trace kept showing
        // the picture from before the edit.
        valueListenable: ui_.frameArrived,
        builder: (context, arrived, __) {
          _requestIfDue(ui_, frame, arrived);
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Container(
                height: 26,
                color: t.surface1,
                padding: const EdgeInsets.symmetric(horizontal: 6),
                // Scrolls sideways when docked narrow, the same answer the
                // Timeline toolbar gives — an overflow stripe is a layout fault.
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
                          label: _label,
                          onChanged: (k) => setState(() {
                            _kind = k;
                            // A new trace kind is worth a request of its own:
                            // neither the frame nor the picture of it has
                            // moved, so nothing else here would ask.
                            _lastFrame = -1;
                          }),
                        ),
                      ),
                      // No frame readout here: the playhead's position is the
                      // Timeline's and the Viewer's to state, and repeating it in
                      // the scope's own toolbar only competes with the trace.
                    ],
                  ),
                ),
              ),
              Expanded(
                child: Container(
                  color: t.surface0,
                  child: Center(
                    child: _trace == null
                        ? Text(l10n.scopesWaiting, style: t.small)
                        : AspectRatio(
                            aspectRatio: 1,
                            child: Stack(
                              fit: StackFit.expand,
                              children: [
                                RawImage(
                                  key: const ValueKey('scope-trace'),
                                  image: _trace,
                                  fit: BoxFit.contain,
                                  filterQuality: FilterQuality.none,
                                ),
                                // The graticule: what the trace is measured
                                // against. Drawn here rather than baked into the
                                // engine's picture so it stays crisp at any panel
                                // size — the trace is a fixed 256x256 and would
                                // carry its labels up scaled and soft.
                                CustomPaint(
                                  key: const ValueKey('scope-graticule'),
                                  painter: _GraticulePainter(
                                    kind: _kind,
                                    line: t.hairlineStrong,
                                    label: t.small.copyWith(color: t.textMuted),
                                  ),
                                ),
                              ],
                            ),
                          ),
                  ),
                ),
              ),
            ],
          );
        },
      ),
    );
  }

  /// Ask for a trace when the frame has moved, or when a new picture of it has
  /// reached the Viewer.
  ///
  /// Called from `build`, so it must never call `setState` — it only sends, and
  /// the reply arrives on the worker stream.
  ///
  /// [arrived] is [LumitUiState.frameArrived]'s count. Together with [frame] it
  /// is the whole memo: the same frame and the same picture of it is nothing to
  /// ask for, and every other rebuild — a hover, a resize, the trace landing —
  /// therefore costs no render. Every rebuild used to ask once a throttle had
  /// elapsed, which was a trace the engine had to make each time; then the
  /// throttle went and the same-frame test alone swallowed a value drag, which
  /// changes the picture without moving the playhead. Once per arrival is the
  /// rule, never once per rebuild. `_lastFrame = -1` is how the kind picker
  /// forces one through.
  void _requestIfDue(LumitUiState state, int frame, int arrived) {
    final comp = state.selectedComp;
    if (comp == null) return;
    if (frame == _lastFrame && arrived == _lastArrival) return;

    _lastFrame = frame;
    _lastArrival = arrived;
    final t = ThemeScope.of(context).theme;
    comp.renderScope(
      frame: BigInt.from(frame),
      scale: state.viewerScale,
      kind: _kind.index,
      colours: scopeColoursFor(t, themed: state.workspace.themedScopes),
    );
  }

  static String _label(ScopeKind kind) => switch (kind) {
        ScopeKind.waveform => l10n.foldWaveform,
        ScopeKind.parade => l10n.scopeParade,
        ScopeKind.vectorscope => l10n.scopeVectorscope,
        ScopeKind.histogram => l10n.scopeHistogram,
      };
}

/// Background, trace, then the R, G and B tints — the five triples the engine
/// takes.
///
/// **Standard by default** (K-202). A waveform or vectorscope is a measuring
/// instrument, and it is read on a near-black graticule with a bright trace
/// whatever the chrome around it is doing — the same grading-accuracy
/// reasoning that keeps the Viewer's surround neutral (docs/15-DESIGN §8,
/// §2.1). `ScopeColours.standard` is that fixed set, and it has been in this
/// file all along; the panel simply never asked for it.
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
/// In plain terms: the engine sends a picture of the measurement, and this
/// draws the ruler over it. Without one a waveform is a shape with no idea
/// what counts as black or white, and a vectorscope is a blob with no idea
/// which way is red — which is what made the scopes pretty rather than useful.
class _GraticulePainter extends CustomPainter {
  final ScopeKind kind;
  final Color line;
  final TextStyle label;

  const _GraticulePainter({
    required this.kind,
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
          final y = size.height * (1 - ire / 100);
          canvas.drawLine(Offset(0, y), Offset(size.width, y), paint);
          _text(canvas, '$ire', Offset(2, y + 1), size);
        }
        // The parade's three cells, so it reads as R, G and B rather than one
        // wide trace. No captions over them: each channel fills its own third
        // of the width and is drawn in its own colour, so an R above the left
        // cell tells you what the cell already says.
        if (kind == ScopeKind.parade) {
          for (var i = 1; i < 3; i++) {
            final x = size.width * i / 3;
            canvas.drawLine(Offset(x, 0), Offset(x, size.height), paint);
          }
        }

      // Hue round, saturation out from the middle: the circle is the frame of
      // reference, and without it the trace has no centre and no scale.
      case ScopeKind.vectorscope:
        final centre = Offset(size.width / 2, size.height / 2);
        final radius = size.shortestSide / 2;
        for (final r in const [0.25, 0.5, 0.75, 1.0]) {
          canvas.drawCircle(centre, radius * r, paint);
        }
        canvas.drawLine(
            Offset(centre.dx, 0), Offset(centre.dx, size.height), paint);
        canvas.drawLine(
            Offset(0, centre.dy), Offset(size.width, centre.dy), paint);
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
          _text(canvas, name, p + const Offset(4, -6), size);
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
          final x = size.width * frac;
          canvas.drawLine(Offset(x, 0), Offset(x, size.height), paint);
          _text(canvas, name, Offset(x + 2, size.height - 12).translate(0, 0),
              size);
        }
    }
  }

  void _text(Canvas canvas, String s, Offset at, Size size) {
    final p = TextPainter(
      text: TextSpan(text: s, style: label),
      textDirection: TextDirection.ltr,
    )..layout();
    p.paint(
      canvas,
      Offset(
        at.dx.clamp(0, (size.width - p.width).clamp(0, double.infinity)),
        at.dy.clamp(0, (size.height - p.height).clamp(0, double.infinity)),
      ),
    );
  }

  @override
  bool shouldRepaint(_GraticulePainter old) =>
      old.kind != kind || old.line != line;

  /// A scale, not a control: clicks fall through to whatever is beneath.
  @override
  bool? hitTest(Offset position) => false;
}
