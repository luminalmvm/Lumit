// The Scopes panel, on the flutter_rust_bridge API.
//
// One, two or four instruments of the frame at the playhead, each slot with
// its own: waveform, parade, vectorscope, histogram or CIE xy. The binning runs
// on the GPU and only the finished 256x256 pictures cross, which is why a
// scope costs a fraction of what reading the frame back would.
//
// **Why it asks for its own render.** The trace needs CPU pixels, and the
// zero-copy Viewer paths never bring any back — so a scope cannot borrow the
// picture on screen and has to render its own. That is real work per trace, so
// this asks once per new picture rather than once per rebuild, every slot's
// trace in the one request, and nothing happens at all while the panel is not
// on screen.

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
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'placeholder.dart';
import 'scopes_graticule.dart';

/// The engine's fixed trace size.
const int _traceEdge = 256;

/// How many instruments the panel shows side by side.
enum ScopeLayout {
  one,
  two,
  four;

  int get slots => switch (this) {
        ScopeLayout.one => 1,
        ScopeLayout.two => 2,
        ScopeLayout.four => 4,
      };
}

/// One slot's instrument and its options.
class ScopeSlot {
  final ScopeKind kind;

  /// Whether the waveform and the histogram read each channel rather than
  /// the brightness.
  final bool rgb;

  /// Whether the vectorscope is drawn at twice the size.
  final bool zoom;

  const ScopeSlot(this.kind, {this.rgb = false, this.zoom = false});

  ScopeSlot copyWith({ScopeKind? kind, bool? rgb, bool? zoom}) =>
      ScopeSlot(kind ?? this.kind,
          rgb: rgb ?? this.rgb, zoom: zoom ?? this.zoom);

  /// The engine's code for this slot's trace: 0 luma waveform, 1 RGB
  /// waveform, 2 vectorscope, 3 RGB histogram, 4 parade, 5 luma histogram, 6
  /// CIE xy, 7 vectorscope at 2x.
  int get engineKind => switch (kind) {
        ScopeKind.waveform => rgb ? 1 : 0,
        ScopeKind.parade => 4,
        ScopeKind.vectorscope => zoom ? 7 : 2,
        ScopeKind.histogram => rgb ? 3 : 5,
        ScopeKind.chromaticity => 6,
      };

  /// Whether this instrument has a luma or RGB reading to choose between.
  /// The parade is RGB by nature and the plots of colour read chroma alone.
  bool get hasChannelPick =>
      kind == ScopeKind.waveform || kind == ScopeKind.histogram;

  Map<String, dynamic> toJson() =>
      {'kind': kind.name, 'rgb': rgb, 'zoom': zoom};

  static ScopeSlot? fromJson(Object? j) {
    if (j is! Map) return null;
    final kind = ScopeKind.values.asNameMap()[j['kind']];
    if (kind == null) return null;
    return ScopeSlot(kind, rgb: j['rgb'] == true, zoom: j['zoom'] == true);
  }
}

/// How one Scopes panel is set up. Kept in the workspace settings by the
/// panel's instance number, so a second Scopes panel keeps its own.
class ScopesSetup extends ChangeNotifier {
  ScopeLayout layout = ScopeLayout.one;

  /// Always four, so a slot keeps its instrument while the layout hides it.
  final List<ScopeSlot> slots = [
    const ScopeSlot(ScopeKind.waveform),
    const ScopeSlot(ScopeKind.vectorscope),
    const ScopeSlot(ScopeKind.parade),
    const ScopeSlot(ScopeKind.histogram),
  ];

  /// How bright the traces are drawn, 1 as the engine makes them.
  double gain = 1;
  bool graticule = true;
  ScopeScale scale = ScopeScale.percent;

  /// The slots the layout shows.
  Iterable<ScopeSlot> get shown => slots.take(layout.slots);

  /// The engine kinds the shown slots need, each once.
  List<int> get kinds => {for (final s in shown) s.engineKind}.toList();

  /// Whether the last change is one to keep. A brightness drag is not until
  /// it is let go, so the settings file is not written on every tick.
  bool kept = true;

  void change(void Function(ScopesSetup s) edit, {bool keep = true}) {
    edit(this);
    kept = keep;
    notifyListeners();
  }

  Map<String, dynamic> toJson() => {
        'layout': layout.name,
        'slots': [for (final s in slots) s.toJson()],
        'gain': gain,
        'graticule': graticule,
        'scale': scale.name,
      };

  void readJson(Map<String, dynamic> j) {
    layout = ScopeLayout.values.asNameMap()[j['layout']] ?? layout;
    if (j['slots'] case final List<dynamic> saved) {
      for (var i = 0; i < math.min(saved.length, slots.length); i++) {
        slots[i] = ScopeSlot.fromJson(saved[i]) ?? slots[i];
      }
    }
    if (j['gain'] case final num g) gain = g.toDouble().clamp(0.25, 4.0);
    graticule = j['graticule'] != false;
    scale = ScopeScale.values.asNameMap()[j['scale']] ?? scale;
  }
}

/// The room a trace leaves round itself for the scale: levels down the left
/// of a waveform, code values under a histogram. Wide enough that "100" in
/// the scale's type clears the panel's edge, and "1023" on the 10-bit scale.
const double _gutter = 26;
const double _gutterTenBit = 32;
const double _margin = 6;

class ScopesPanelFrb extends StatefulWidget {
  /// Which Scopes panel this is, when there is more than one: each keeps its
  /// own setup.
  final int instance;

  const ScopesPanelFrb({super.key, this.instance = 0});

  @override
  State<ScopesPanelFrb> createState() => _ScopesPanelFrbState();
}

class _ScopesPanelFrbState extends State<ScopesPanelFrb> {
  final ScopesSetup _setup = ScopesSetup();

  /// The newest trace of each engine kind asked for. Slots showing the same
  /// kind share one picture.
  final Map<int, ui.Image> _traces = {};

  /// The kinds the last request asked for. A trace of any other kind (the
  /// Levels effect asks for histograms on the same stream) is never shown.
  List<int> _asked = const [];
  StreamSubscription<WorkerResponse>? _responses;

  int _lastFrame = -1;

  /// The value of [LumitUiState.frameArrived] the last request was made at.
  /// Part of the memo with [_lastFrame], because a new *picture* of the frame
  /// already traced is exactly as worth tracing as a new frame number — a
  /// value drag holds the playhead still and changes the picture under it.
  int _lastArrival = -1;

  late final LumitUiState _ui;

  @override
  void initState() {
    super.initState();
    final state = Provider.of<LumitState>(context, listen: false);
    _responses = state.onWorkerResponse.listen(_onResponse);
    // The two facts worth a trace, listened to rather than read in `build`:
    // the playhead moving, and a picture reaching the Viewer. Nothing a
    // rebuild does on its own asks the engine for anything.
    _ui = Provider.of<LumitUiState>(context, listen: false);
    final saved = _ui.workspace.scopeSetups[widget.instance];
    if (saved != null) _setup.readJson(saved);
    _setup.addListener(_changed);
    _ui.playheadFrame.addListener(_request);
    _ui.frameArrived.addListener(_request);
    WidgetsBinding.instance.addPostFrameCallback((_) => _request());
  }

  @override
  void dispose() {
    _ui.playheadFrame.removeListener(_request);
    _ui.frameArrived.removeListener(_request);
    _setup.removeListener(_changed);
    _setup.dispose();
    _responses?.cancel();
    for (final image in _traces.values) {
      image.dispose();
    }
    super.dispose();
  }

  /// A setting changed: keep it, redraw, and ask for whatever new kind of
  /// trace it needs. Neither the frame nor the picture of it has moved, so
  /// nothing else here would ask.
  void _changed() {
    if (_setup.kept) {
      _ui.workspace.setScopeSetup(widget.instance, _setup.toJson());
    }
    setState(() {});
    if (!_sameKinds(_setup.kinds, _asked)) _request(force: true);
  }

  static bool _sameKinds(List<int> a, List<int> b) =>
      a.length == b.length && a.toSet().containsAll(b);

  /// Scope traces ride the same worker stream as the frames, so this ignores
  /// everything that is not one, and every trace that is not this panel's.
  /// The Levels effect's own histogram asks on the same stream, and before
  /// this test it landed here as a white bell over the waveform.
  void _onResponse(WorkerResponse response) {
    if (response is! WorkerResponse_Scope) return;
    final kind = response.field0.kind;
    if (!_asked.contains(kind)) return;
    ui.decodeImageFromPixels(
      Uint8List.fromList(response.field0.rgba),
      _traceEdge,
      _traceEdge,
      ui.PixelFormat.rgba8888,
      (image) {
        if (!mounted || !_asked.contains(kind)) {
          image.dispose();
          return;
        }
        setState(() {
          _traces[kind]?.dispose();
          _traces[kind] = image;
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
    final themed = ui_.workspace.themedScopes;
    Widget slot(int i) => _slot(t, i, themed: themed);
    Widget across(List<Widget> children) => Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (var i = 0; i < children.length; i++) ...[
              if (i > 0) Container(width: 1, color: t.hairline),
              Expanded(child: children[i]),
            ],
          ],
        );
    return switch (_setup.layout) {
      ScopeLayout.one => slot(0),
      ScopeLayout.two => across([slot(0), slot(1)]),
      ScopeLayout.four => Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(child: across([slot(0), slot(1)])),
            Container(height: 1, color: t.hairline),
            Expanded(child: across([slot(2), slot(3)])),
          ],
        ),
    };
  }

  /// One instrument: its header over its plot, and its settings on a
  /// right-click anywhere on either.
  Widget _slot(LumitTheme t, int i, {required bool themed}) => _ScopeMenu(
        setup: _setup,
        slot: i,
        workspace: _ui.workspace,
        onThemed: (on) {
          _ui.workspace.setThemedScopes(on);
          // The same kinds in other colours: worth a request of its own.
          _request(force: true);
        },
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _header(t, i),
            Expanded(child: _body(t, i, themed: themed)),
          ],
        ),
      );

  /// The suffix that tells one slot's keys from another's. None on the
  /// first, so a one-up panel reads as it always has.
  static String _key(String name, int i) => i == 0 ? name : '$name-$i';

  Widget _header(LumitTheme t, int i) {
    final slot = _setup.slots[i];
    void set(ScopeSlot s) => _setup.change((x) => x.slots[i] = s);
    // The layout picker sits in the panel's top-right corner, whichever slot
    // that is.
    final corner = i == math.min(1, _setup.layout.slots - 1);
    return Container(
      height: 26,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Row(
        children: [
          Expanded(
            // Scrolls sideways when docked narrow, the same answer the
            // Timeline toolbar gives, as an overflow stripe is a layout fault.
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: [
                  SizedBox(
                    width: 110,
                    child: BareDropdown<ScopeKind>(
                      key: ValueKey(_key('scope-kind', i)),
                      value: slot.kind,
                      options: ScopeKind.values,
                      label: scopeKindLabel,
                      onChanged: (k) => set(slot.copyWith(kind: k)),
                    ),
                  ),
                  if (slot.hasChannelPick) ...[
                    const SizedBox(width: 8),
                    _pill(l10n.scopeLuma, _key('scope-luma', i),
                        on: !slot.rgb,
                        onPressed: () => set(slot.copyWith(rgb: false))),
                    const SizedBox(width: 2),
                    _pill(l10n.scopeRgb, _key('scope-rgb', i),
                        on: slot.rgb,
                        onPressed: () => set(slot.copyWith(rgb: true))),
                  ],
                  if (slot.kind == ScopeKind.vectorscope) ...[
                    const SizedBox(width: 8),
                    _pill(l10n.scopeZoom, _key('scope-zoom', i),
                        on: slot.zoom,
                        onPressed: () => set(slot.copyWith(zoom: !slot.zoom))),
                  ],
                ],
              ),
            ),
          ),
          if (corner)
            BareDropdown<ScopeLayout>(
              key: const ValueKey('scope-layout'),
              value: _setup.layout,
              options: ScopeLayout.values,
              label: scopeLayoutLabel,
              face: CustomPaint(
                size: const Size(14, 10),
                painter: _LayoutGlyph(_setup.layout, t.textMuted),
              ),
              onChanged: (l) => _setup.change((x) => x.layout = l),
            ),
        ],
      ),
    );
  }

  /// The word takes the button's own colour: on Lantern's filled pill the
  /// panel's small style vanished into the accent.
  Widget _pill(String word, String key,
          {required bool on, required VoidCallback onPressed}) =>
      HouseButton(
        key: ValueKey<String>(key),
        small: true,
        frameless: !on,
        active: on,
        onPressed: onPressed,
        child: Text(word),
      );

  Widget _body(LumitTheme t, int i, {required bool themed}) {
    final slot = _setup.slots[i];
    final colours = themed ? null : ScopeColours.standard;
    final plotFill = colours?.bg ?? t.surface0;
    final line = colours?.graticule ?? t.hairlineStrong;
    final trace = _traces[slot.engineKind];
    return Container(
      color: t.surface0,
      child: LayoutBuilder(
        builder: (context, box) {
          final plot = _plotRect(slot.kind, box.biggest);
          return Stack(
            fit: StackFit.expand,
            children: [
              Positioned.fromRect(
                rect: plot,
                child: ColoredBox(
                  color: plotFill,
                  child: trace == null
                      ? null
                      : _brightened(
                          plotFill,
                          RawImage(
                            key: ValueKey(_key('scope-trace', i)),
                            image: trace,
                            fit: BoxFit.fill,
                            // A 256-column trace stretched across a wide
                            // panel: bilinear gives a phosphor's glow,
                            // nearest gives bars.
                            filterQuality: FilterQuality.low,
                          ),
                        ),
                ),
              ),
              if (_setup.graticule)
                CustomPaint(
                  key: ValueKey(_key('scope-graticule', i)),
                  painter: ScopeGraticulePainter(
                    kind: slot.kind,
                    plot: plot,
                    line: line,
                    label: t.mono.copyWith(fontSize: 9, color: t.textMuted),
                    scale: _setup.scale,
                    zoom: slot.zoom,
                  ),
                ),
            ],
          );
        },
      ),
    );
  }

  /// The trace at the panel's brightness. The engine adds each trace over the
  /// backdrop, so scaling what stands above the backdrop by the gain is
  /// scaling the trace and nothing else, whatever colour the backdrop is.
  Widget _brightened(Color backdrop, Widget trace) {
    final g = _setup.gain;
    if (g == 1) return trace;
    double off(double c) => (1 - g) * c * 255;
    return ColorFiltered(
      colorFilter: ColorFilter.matrix([
        g, 0, 0, 0, off(backdrop.r), //
        0, g, 0, 0, off(backdrop.g), //
        0, 0, g, 0, off(backdrop.b), //
        0, 0, 0, 1, 0, //
      ]),
      child: trace,
    );
  }

  /// Where the trace is drawn: the slot less the scale's gutters, and for
  /// the two plots of colour the largest square that fits, centred.
  Rect _plotRect(ScopeKind kind, Size size) {
    final square =
        kind == ScopeKind.vectorscope || kind == ScopeKind.chromaticity;
    final left = square
        ? _margin
        : _setup.scale == ScopeScale.tenBit
            ? _gutterTenBit
            : _gutter;
    final bottom = kind == ScopeKind.histogram ? 14.0 : _margin;
    final inner = Rect.fromLTRB(
        left,
        _margin,
        math.max(left, size.width - _margin),
        math.max(_margin, size.height - bottom));
    if (!square) return inner;
    final side = math.max(0.0, inner.shortestSide);
    return Rect.fromCenter(center: inner.center, width: side, height: side);
  }

  /// Ask for traces when the frame has moved, or when a new picture of it has
  /// reached the Viewer. The engine traces the picture the Viewer is showing,
  /// a drag's preview included, so the trace follows the grade in hand.
  ///
  /// Never from `build`: the playhead and the arrival count call it, and the
  /// replies arrive on the worker stream, one per kind. Those two numbers are
  /// the whole memo: the same frame and the same picture of it is nothing to
  /// ask for, so a hover, a resize or a trace landing costs no render. [force]
  /// is how a new instrument asks for a new kind of the same picture.
  void _request({bool force = false}) {
    if (!mounted) return;
    final comp = _ui.selectedComp;
    if (comp == null) return;
    final frame = _ui.playheadFrame.value;
    final arrived = _ui.frameArrived.value;
    if (!force && frame == _lastFrame && arrived == _lastArrival) return;

    _lastFrame = frame;
    _lastArrival = arrived;
    final kinds = _setup.kinds;
    // A trace of a kind no longer shown is dropped, so showing it again
    // later starts from an empty plot rather than an old picture.
    for (final gone in _traces.keys.where((k) => !kinds.contains(k)).toList()) {
      _traces.remove(gone)?.dispose();
    }
    _asked = kinds;
    final t = ThemeScope.of(context).theme;
    try {
      comp.renderScope(
        frame: frame,
        scale: _ui.measuredScale,
        kinds: kinds,
        colours: scopeColoursFor(t, themed: _ui.workspace.themedScopes),
        view: _ui.measuredViewId,
      );
    } catch (_) {
      // A comp that went away under the panel: the plots stay as they were.
    }
  }
}

/// What the picker and the menu call each instrument.
String scopeKindLabel(ScopeKind kind) => switch (kind) {
      ScopeKind.waveform => l10n.foldWaveform,
      ScopeKind.parade => l10n.scopeParade,
      ScopeKind.vectorscope => l10n.scopeVectorscope,
      ScopeKind.histogram => l10n.scopeHistogram,
      ScopeKind.chromaticity => l10n.scopeChromaticity,
    };

/// What the layout picker and the menu call each layout.
String scopeLayoutLabel(ScopeLayout layout) => switch (layout) {
      ScopeLayout.one => l10n.scopeLayoutOne,
      ScopeLayout.two => l10n.scopeLayoutTwo,
      ScopeLayout.four => l10n.scopeLayoutFour,
    };

/// The layout picker's face: one, two or four boxes, as the panel will be.
class _LayoutGlyph extends CustomPainter {
  final ScopeLayout layout;
  final Color colour;

  const _LayoutGlyph(this.layout, this.colour);

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    final box = (Offset.zero & size).deflate(0.5);
    canvas.drawRect(box, paint);
    if (layout != ScopeLayout.one) {
      canvas.drawLine(box.topCenter, box.bottomCenter, paint);
    }
    if (layout == ScopeLayout.four) {
      canvas.drawLine(box.centerLeft, box.centerRight, paint);
    }
  }

  @override
  bool shouldRepaint(_LayoutGlyph old) =>
      old.layout != layout || old.colour != colour;
}

/// A slot's right-click menu: its instrument and that instrument's options,
/// then what the whole panel shares: the layout, the scale and the
/// brightness. It follows the setup while it is open, so a row ticks as it is
/// picked.
class _ScopeMenu extends StatelessWidget {
  final ScopesSetup setup;
  final int slot;
  final Workspace workspace;
  final ValueChanged<bool> onThemed;
  final Widget child;

  const _ScopeMenu({
    required this.setup,
    required this.slot,
    required this.workspace,
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
    showLumitPopup<void>(
      context: context,
      position: at,
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: ListenableBuilder(
            listenable: Listenable.merge([setup, workspace]),
            builder: (context, _) => _rows(),
          ),
        ),
      ),
    );
  }

  Widget _rows() {
    final s = setup.slots[slot];
    void set(ScopeSlot next) => setup.change((x) => x.slots[slot] = next);
    Widget option(String key, String label, bool on, VoidCallback press,
            {bool enabled = true}) =>
        MenuRow.option(
          key: ValueKey<String>('scope-menu-$key'),
          enabled: enabled,
          selected: enabled && on,
          onPressed: press,
          child: MenuLabel(label, ticked: enabled && on, enabled: enabled),
        );
    final picks = s.hasChannelPick;
    final zooms = s.kind == ScopeKind.vectorscope;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final k in ScopeKind.values)
          option(k.name, scopeKindLabel(k), k == s.kind,
              () => set(s.copyWith(kind: k))),
        const MenuDivider(),
        // Greyed where the instrument has no such reading, so the rows keep
        // their place and the eye learns where to find them.
        option(
            'luma', l10n.scopeLuma, !s.rgb, () => set(s.copyWith(rgb: false)),
            enabled: picks),
        option('rgb', l10n.scopeRgb, s.rgb, () => set(s.copyWith(rgb: true)),
            enabled: picks),
        option('zoom', l10n.scopeZoom, s.zoom,
            () => set(s.copyWith(zoom: !s.zoom)),
            enabled: zooms),
        const MenuDivider(),
        for (final l in ScopeLayout.values)
          option('layout-${l.name}', scopeLayoutLabel(l), l == setup.layout,
              () => setup.change((x) => x.layout = l)),
        const MenuDivider(),
        option('graticule', l10n.scopeGraticule, setup.graticule,
            () => setup.change((x) => x.graticule = !x.graticule)),
        option(
            'percent',
            l10n.scopeScalePercent,
            setup.scale == ScopeScale.percent,
            () => setup.change((x) => x.scale = ScopeScale.percent)),
        option(
            'ten-bit',
            l10n.scopeScaleTenBit,
            setup.scale == ScopeScale.tenBit,
            () => setup.change((x) => x.scale = ScopeScale.tenBit)),
        option('themed', l10n.scopeThemeColours, workspace.themedScopes,
            () => onThemed(!workspace.themedScopes)),
        const MenuDivider(),
        _BrightnessRow(setup: setup),
      ],
    );
  }
}

/// The trace brightness, as a slider in the menu: it shows live while it is
/// dragged and is kept when it is let go.
class _BrightnessRow extends StatelessWidget {
  final ScopesSetup setup;

  const _BrightnessRow({required this.setup});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
      child: Row(
        children: [
          // The tick column, so the word lines up with the rows above.
          menuTick(false),
          Text(l10n.scopeBrightness, style: t.bodyPrimary),
          const SizedBox(width: 12),
          HouseSlider(
            key: const ValueKey('scope-menu-brightness'),
            value: setup.gain * 100,
            min: 25,
            max: 400,
            step: 5,
            decimals: 0,
            suffix: '%',
            width: 110,
            commitOnRelease: true,
            onChangeLive: (v) =>
                setup.change((x) => x.gain = v / 100, keep: false),
            onChanged: (v) => setup.change((x) => x.gain = v / 100),
          ),
        ],
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
