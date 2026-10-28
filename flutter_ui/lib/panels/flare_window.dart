// The Flare window: the Sprite flare's design, a list of elements drawn in
// order, with a preview, the presets strip and the selected element's
// controls. Edits stay in the window until Use flare puts the design on the
// effect, or Save to your flares writes it as a .flare file.

library;

import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:uuid/uuid.dart';

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/flare.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/dialog_frame.dart';
import '../shell/theme_name_dialog.dart';
import '../state/file_dialogs.dart';
import '../state/preview_throttle.dart';
import '../theme/theme.dart';
import '../widgets/angle_dial.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import 'shader_editor.dart' show InstanceHome;

part 'flare_window_elements.dart';
part 'flare_window_inspector.dart';
part 'flare_window_presets.dart';

typedef Json = Map<String, dynamic>;

const Size flareWindowSize = Size(1180, 760);
const Size _minSize = Size(900, 600);
const double _listWidth = 220;
const double _inspectorWidth = 300;
const double _rowHeight = 24;
const double _labelWidth = 120;

/// The widest a preset tile grows; the grid fits as many as the width takes.
const double _tileMaxWidth = 160;

/// The comp the preview stands in for when the window has no comp to ask: a
/// node graph's Fx box with no layer behind it.
const Size _fallbackComp = Size(1920, 1080);

/// The longest edge of the Frame background. One still, asked for once.
const int _stillEdge = 960;

/// The Frame background's last choice, kept for the next opening.
bool _frameLast = false;

/// A still of the comp at the playhead with the flare left out, its longest
/// edge [maxEdge] pixels. Null where nothing could be drawn.
typedef FlareStill = Future<BridgeRenderedFrame?> Function(int maxEdge);

/// Press a Sprite flare's Edit flare… row. Answers whether [param] was it.
/// [comp] and [frame] let the preview take the comp's size and show its
/// picture at the playhead; without them it stands in for a 1080p frame.
bool pressFlareButton({
  required BuildContext context,
  required InstanceHome home,
  required UuidValue effect,
  required String param,
  required VoidCallback onApplied,
  CompositionReference? comp,
  BigInt? frame,
}) {
  if (param != 'edit_flare') return false;
  unawaited(() async {
    if (await showFlareWindow(
            context: context,
            home: home,
            effect: effect,
            comp: comp,
            frame: frame) &&
        context.mounted) {
      onApplied();
    }
  }());
  return true;
}

/// Open the window on one effect and commit what it returns. Answers whether
/// anything changed. While it is open the Viewer follows its edits through
/// [InstanceHome.draw], and Cancel puts the Viewer back.
Future<bool> showFlareWindow({
  required BuildContext context,
  required InstanceHome home,
  required UuidValue effect,
  CompositionReference? comp,
  BigInt? frame,
}) async {
  String? design;
  double? lightX, lightY;
  for (final instance in home.read()) {
    if (instance.id() != effect) continue;
    design = instance.flareDesign();
    // A still Light is where the preview's light starts, a keyed one isn't.
    double? still(String id) => switch (instance.getValue(id: id)) {
          BridgeEffectValue_Float(field0: BridgeScalar_Static(:final field0)) =>
            field0,
          _ => null,
        };
    lightX = still('light_x');
    lightY = still('light_y');
  }
  if (design == null || !context.mounted) return false;
  final opened = design;
  Size? compSize;
  try {
    final s = comp?.getSettings();
    if (s != null) compSize = Size(s.width.toDouble(), s.height.toDouble());
  } catch (_) {}
  final draw = home.draw;
  final chosen = await showLumitModal<String>(
    context: context,
    id: 'flare-window',
    initialSize: flareWindowSize,
    minSize: _minSize,
    builder: (close) => FlareWindow(
      design: opened,
      compSize: compSize,
      light: lightX == null || lightY == null
          ? null
          : Offset(lightX / (compSize ?? _fallbackComp).width,
              lightY / (compSize ?? _fallbackComp).height),
      still: comp == null || frame == null
          ? null
          : (maxEdge) => comp.stillWithoutEffect(
              frame: frame, maxEdge: maxEdge, effect: effect),
      draw: draw == null
          ? null
          : (json) {
              final staged = home.read();
              for (final instance in staged) {
                if (instance.id() != effect) continue;
                try {
                  instance.setFlareDesign(json: json);
                } catch (_) {
                  return;
                }
                draw(staged);
                return;
              }
            },
      onUse: close,
      onCancel: () => close(null),
    ),
  );
  if (chosen == null) {
    // The Viewer showed the window's edits; the document's own design again.
    draw?.call(home.read());
    return false;
  }
  final stack = home.read();
  for (final instance in stack) {
    if (instance.id() != effect) continue;
    instance.setFlareDesign(json: chosen);
    try {
      home.commit(stack);
    } catch (_) {
      return false;
    }
    return true;
  }
  return false;
}

/// The window's body, given the design as JSON. Public so it can be put in
/// a window of any size.
class FlareWindow extends StatefulWidget {
  final String design;

  /// The comp the flare is in, so px@comp sizes read as they will there.
  final Size? compSize;

  /// Where the light starts, as fractions of the comp.
  final Offset? light;

  /// The comp's picture at the playhead without this flare, for the Frame
  /// background. Null hides the choice.
  final FlareStill? still;

  /// Shows the design as it stands in the Viewer, latest wins.
  final ValueChanged<String>? draw;
  final ValueChanged<String> onUse;
  final VoidCallback onCancel;

  const FlareWindow({
    super.key,
    required this.design,
    this.compSize,
    this.light,
    this.still,
    this.draw,
    required this.onUse,
    required this.onCancel,
  });

  @override
  State<FlareWindow> createState() => _FlareWindowState();
}

class _FlareWindowState extends State<FlareWindow>
    with SingleTickerProviderStateMixin {
  late Json _design = jsonDecode(widget.design) as Json;
  late final List<BridgeFlareKind> _kinds = flareKinds();
  late final List<BridgeFlareOption> _modes = flareColourModes();
  final Map<String, List<BridgeFlareControl>> _controls = {};
  List<BridgeFlarePreset> _presets = flarePresets();

  int _selected = 0;

  /// The one element the preview shows alone, from an Alt-click on its eye.
  int? _solo;

  /// Snapshots of the design and the selection, for Ctrl+Z.
  final List<(String, int)> _undo = [];
  final List<(String, int)> _redo = [];
  String? _coalesce;
  DateTime _coalesceAt = DateTime.now();

  ui.Image? _image;
  Size _previewSize = Size.zero;
  bool _busy = false;
  bool _dirty = false;

  /// The Frame background: whether it is wanted, and the still once it has
  /// been drawn. Asked for once, when it is switched on.
  bool _frame = _frameLast;
  ui.Image? _still;
  bool _stillAsked = false;

  /// The Viewer follows at a drag's pace, the newest edit winning.
  final PreviewThrottle _viewer = PreviewThrottle();

  /// The light, as fractions of the preview. May sit off it.
  late Offset _light = widget.light ?? const Offset(0.3, 0.35);
  bool _moving = false;
  bool _fit = true;
  late final Ticker _ticker = createTicker(_tick);

  /// Which presets tab is open, or null for the default.
  bool? _yours;
  String _group = '';
  String? _said;

  final _name = TextEditingController();

  /// The last row clicked and when, so a second click can rename it.
  int? _lastTap;
  DateTime _lastTapAt = DateTime.now();

  /// The list row being renamed in place, and its field.
  int? _renaming;
  final _listName = TextEditingController();

  /// A grip drag: the row it started on, where it sits now, and how far the
  /// pointer has come.
  int? _dragFrom;
  int? _dragAt;
  double _dragDy = 0;

  final _addKey = GlobalKey();
  final _shareKey = GlobalKey();
  final Set<String> _closed = {'Response', 'Ignore'};

  @override
  void initState() {
    super.initState();
    _syncName();
    if (_frame) unawaited(_askStill());
  }

  @override
  void dispose() {
    _ticker.dispose();
    _name.dispose();
    _listName.dispose();
    _image?.dispose();
    _still?.dispose();
    super.dispose();
  }

  Size get _comp => widget.compSize ?? _fallbackComp;

  List<Json> get _elements => (_design['elements'] as List).cast<Json>();

  Json? get _element =>
      _selected < _elements.length ? _elements[_selected] : null;

  String get _designName => (_design['name'] as String?) ?? '';

  void _syncName() => _name.text = (_element?['name'] as String?) ?? '';

  /// The parts files rebuild through this, as setState is the state's own.
  void _redraw(VoidCallback fn) => setState(fn);

  /// Change the design through [fn], with a snapshot first so Ctrl+Z can
  /// take it back. Edits that share a [coalesce] key within a second are one
  /// step, so a slider drag undoes in one.
  void _edit(void Function() fn, {String? coalesce}) {
    final now = DateTime.now();
    if (coalesce == null ||
        coalesce != _coalesce ||
        now.difference(_coalesceAt) > const Duration(seconds: 1)) {
      _undo.add((jsonEncode(_design), _selected));
      _redo.clear();
      if (_undo.length > 200) _undo.removeAt(0);
    }
    _coalesce = coalesce;
    _coalesceAt = now;
    setState(fn);
    _requestPreview();
    _drawViewer();
  }

  /// The Viewer shows the design as it stands, the window's solo left out.
  void _drawViewer() {
    final draw = widget.draw;
    if (draw == null) return;
    _viewer.request(() => draw(jsonEncode(_design)));
  }

  void _undoStep() {
    if (_undo.isEmpty) return;
    _redo.add((jsonEncode(_design), _selected));
    final (json, selected) = _undo.removeLast();
    _restore(json, selected);
  }

  void _redoStep() {
    if (_redo.isEmpty) return;
    _undo.add((jsonEncode(_design), _selected));
    final (json, selected) = _redo.removeLast();
    _restore(json, selected);
  }

  void _restore(String json, int selected) {
    _coalesce = null;
    setState(() {
      _design = jsonDecode(json) as Json;
      _selected = selected.clamp(0, math.max(0, _elements.length - 1));
      _solo = null;
      _syncName();
    });
    _requestPreview();
    _drawViewer();
  }

  KeyEventResult _key(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    if (FocusManager.instance.primaryFocus?.context?.widget is EditableText) {
      return KeyEventResult.ignored;
    }
    final keys = HardwareKeyboard.instance;
    final ctrl = keys.isControlPressed || keys.isMetaPressed;
    final k = event.logicalKey;
    if (ctrl && k == LogicalKeyboardKey.keyZ) {
      keys.isShiftPressed ? _redoStep() : _undoStep();
      return KeyEventResult.handled;
    }
    if (ctrl && k == LogicalKeyboardKey.keyY) {
      _redoStep();
      return KeyEventResult.handled;
    }
    if ((k == LogicalKeyboardKey.delete || k == LogicalKeyboardKey.backspace) &&
        _elements.isNotEmpty) {
      _deleteElement(_selected);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  // The preview. Latest wins: a request while one is drawing marks it
  // dirty, and the next draw starts when that one lands.

  String _previewJson() {
    if (_solo == null) return jsonEncode(_design);
    final copy = jsonDecode(jsonEncode(_design)) as Json;
    final elements = copy['elements'] as List;
    for (var i = 0; i < elements.length; i++) {
      (elements[i] as Json)['on'] = i == _solo;
    }
    return jsonEncode(copy);
  }

  void _requestPreview() {
    _dirty = true;
    unawaited(_pumpPreview());
  }

  Future<void> _pumpPreview() async {
    if (_busy || !_dirty || _previewSize.isEmpty) return;
    _busy = true;
    _dirty = false;
    final w = _previewSize.width.round();
    final h = _previewSize.height.round();
    try {
      final bytes = await flarePreview(
        designJson: _previewJson(),
        width: w,
        height: h,
        lightX: _light.dx * w,
        lightY: _light.dy * h,
        pxScale: _fit ? w / _comp.width : 1,
      );
      final image = bytes.isEmpty ? null : await _decode(bytes, w, h);
      if (mounted) {
        setState(() {
          _image?.dispose();
          _image = image;
        });
      }
    } catch (_) {}
    _busy = false;
    if (mounted && _dirty) unawaited(_pumpPreview());
  }

  void _setFrame(bool on) {
    setState(() => _frame = on);
    _frameLast = on;
    if (on) unawaited(_askStill());
  }

  /// The Frame background, drawn once: the comp at the playhead without this
  /// flare. A second switch-on shows the still already here.
  Future<void> _askStill() async {
    final still = widget.still;
    if (still == null || _stillAsked) return;
    _stillAsked = true;
    try {
      final got = await still(_stillEdge);
      if (got == null || !mounted) return;
      final image = await _decode(got.rgba, got.width, got.height);
      if (!mounted) return;
      setState(() {
        _still?.dispose();
        _still = image;
      });
    } catch (_) {}
  }

  void _tick(Duration elapsed) {
    final a = elapsed.inMilliseconds / 1000 * 0.35;
    setState(() => _light = Offset(
        0.5 + 0.38 * math.cos(a), 0.5 + 0.3 * math.sin(0.7 * a)));
    _requestPreview();
  }

  void _setMoving(bool on) {
    setState(() => _moving = on);
    if (on) {
      if (!_ticker.isActive) _ticker.start();
    } else {
      _ticker.stop();
    }
  }

  void _dragLight(Offset local) {
    if (_moving) _setMoving(false);
    setState(() => _light = Offset(
          (local.dx / _previewSize.width).clamp(-0.25, 1.25),
          (local.dy / _previewSize.height).clamp(-0.25, 1.25),
        ));
    _requestPreview();
  }

  // The elements.

  void _select(int i) {
    if (!mounted) return;
    setState(() {
      _selected = i;
      _syncName();
    });
  }

  void _addKind(String kind) {
    final element = jsonDecode(flareNewElement(kind: kind)) as Json;
    _edit(() {
      _elements.add(element);
      _selected = _elements.length - 1;
      _syncName();
    });
  }

  void _duplicateElement(int i) {
    if (i >= _elements.length) return;
    final copy = jsonDecode(jsonEncode(_elements[i])) as Json;
    _edit(() {
      _elements.insert(i + 1, copy);
      _selected = i + 1;
      _solo = null;
      _syncName();
    });
  }

  void _deleteElement(int i) {
    if (i >= _elements.length) return;
    _edit(() {
      _elements.removeAt(i);
      _selected = _selected.clamp(0, math.max(0, _elements.length - 1));
      _solo = null;
      _syncName();
    });
  }

  void _moveElement(int from, int to) {
    if (from == to) return;
    final solo = _solo == null ? null : _elements[_solo!];
    final selected = _element;
    final element = _elements.removeAt(from);
    _elements.insert(to, element);
    _selected = selected == null ? 0 : _elements.indexOf(selected);
    _solo = solo == null ? null : _elements.indexOf(solo);
  }

  void _renameSelected(String text) {
    final e = _element;
    final name = text.trim();
    if (e == null || name.isEmpty || e['name'] == name) {
      _syncName();
      return;
    }
    _edit(() => e['name'] = name);
  }

  // Files and the clipboard.

  Future<void> _import() async {
    final path = await pickEffectInputFile(['flare'], l10n.flareFileType);
    if (path == null || !mounted) return;
    try {
      final json = await importFlare(path: path);
      if (!mounted) return;
      _edit(() {
        _design = jsonDecode(json) as Json;
        _selected = 0;
        _solo = null;
        _said = null;
        _syncName();
      });
    } catch (_) {
      setState(() => _said = l10n.flareNotAFlare);
    }
  }

  Future<void> _share() async {
    final box = _shareKey.currentContext?.findRenderObject();
    if (box is! RenderBox) return;
    final what = await showLumitPopup<String>(
      context: context,
      position: box.localToGlobal(Offset(0, -4)),
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              MenuRow(
                key: const ValueKey('flare-share-export'),
                onPressed: () => close('export'),
                child: Text(l10n.flareExportFile),
              ),
              MenuRow(
                key: const ValueKey('flare-share-copy'),
                onPressed: () => close('copy'),
                child: Text(l10n.flareCopyJson),
              ),
            ],
          ),
        ),
      ),
    );
    if (!mounted) return;
    switch (what) {
      case 'copy':
        await Clipboard.setData(ClipboardData(text: jsonEncode(_design)));
        if (mounted) setState(() => _said = l10n.flareCopied);
      case 'export':
        final path = await pickFlareSaveLocation(
            _designName.isEmpty ? l10n.flareFileType : _designName);
        if (path == null || !mounted) return;
        try {
          await exportFlare(json: jsonEncode(_design), path: path);
        } catch (_) {
          if (mounted) setState(() => _said = l10n.flareNotSaved);
        }
    }
  }

  Future<void> _save() async {
    final name = await askThemeName(context,
        title: l10n.flareNameTitle,
        suggested: _designName,
        confirm: l10n.flareSave);
    if (name == null || !mounted) return;
    _design['name'] = name;
    try {
      final saved = await saveFlare(json: jsonEncode(_design));
      if (!mounted) return;
      setState(() {
        _design['name'] = saved;
        _presets = flarePresets();
        _yours = true;
        _said = null;
      });
    } catch (_) {
      if (mounted) setState(() => _said = l10n.flareNotSaved);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Focus(
      autofocus: true,
      onKeyEvent: _key,
      child: DialogFrame(
        // No width of its own: the window has the size, so the grip can
        // change it.
        children: [
          dialogTitleBar(t,
              title: l10n.flareWindowTitle,
              subject: _designName,
              onClose: widget.onCancel,
              keyPrefix: 'flare'),
          Expanded(
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SizedBox(width: _listWidth, child: _elementsPane(t)),
                Container(width: 1, color: t.hairline),
                Expanded(child: _middle(t)),
                Container(width: 1, color: t.hairline),
                SizedBox(width: _inspectorWidth, child: _inspector(t)),
              ],
            ),
          ),
          dialogFooter(
            t,
            keyPrefix: 'flare',
            leading: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                dialogPlainAction(t,
                    key: const ValueKey('flare-import'),
                    label: l10n.flareImport,
                    onPressed: _import),
                const SizedBox(width: dialogFooterGap),
                KeyedSubtree(
                  key: _shareKey,
                  child: dialogPlainAction(t,
                      key: const ValueKey('flare-share'),
                      label: l10n.flareShare,
                      onPressed: _share),
                ),
                if (_said case final said?) ...[
                  const SizedBox(width: dialogFooterGap),
                  Text(said,
                      key: const ValueKey('flare-summary'),
                      style: dialogMono(t)),
                ],
              ],
            ),
            actions: [
              dialogPlainAction(t,
                  key: const ValueKey('flare-cancel'),
                  label: l10n.cancel,
                  onPressed: widget.onCancel),
              dialogPlainAction(t,
                  key: const ValueKey('flare-save'),
                  label: l10n.flareSave,
                  onPressed: _save),
              dialogFilledAction(t,
                  key: const ValueKey('flare-use'),
                  label: l10n.flareUse,
                  onPressed: () => widget.onUse(jsonEncode(_design))),
            ],
          ),
        ],
      ),
    );
  }

  /// The least height the presets grid keeps under the preview: about two
  /// rows of tiles.
  static const double _gridMin = 150;

  Widget _middle(LumitTheme t) => Padding(
        padding: const EdgeInsets.all(10),
        child: LayoutBuilder(builder: (context, c) {
          final aspect = _comp.width / math.max(1, _comp.height);
          // The toolbar, the presets header and the gaps between them.
          const fixed = 24.0 + 8 + 10 + 22 + 6;
          final previewHeight = math
              .min(c.maxWidth / aspect, c.maxHeight - fixed - _gridMin)
              .clamp(80.0, c.maxHeight)
              .floorToDouble();
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              SizedBox(
                height: 24,
                child: Row(
                  children: [
                    if (widget.still != null) ...[
                      _segmented(
                        t,
                        keyPrefix: 'flare-ground',
                        items: [
                          ('black', l10n.flareBlack),
                          ('frame', l10n.flareFrame),
                        ],
                        current: _frame ? 'frame' : 'black',
                        onPick: (k) => _setFrame(k == 'frame'),
                      ),
                      const SizedBox(width: 6),
                    ],
                    _segmented(
                      t,
                      keyPrefix: 'flare-light',
                      items: [
                        ('moving', l10n.flareMovingLight),
                        ('still', l10n.flareStillLight),
                      ],
                      current: _moving ? 'moving' : 'still',
                      onPick: (k) => _setMoving(k == 'moving'),
                    ),
                    const Spacer(),
                    _segmented(
                      t,
                      keyPrefix: 'flare-zoom',
                      items: [
                        ('fit', l10n.flareFit),
                        ('actual', l10n.flareActualSize),
                      ],
                      current: _fit ? 'fit' : 'actual',
                      onPick: (k) {
                        setState(() => _fit = k == 'fit');
                        _requestPreview();
                      },
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 8),
              SizedBox(
                height: previewHeight,
                child: Center(
                  child: AspectRatio(aspectRatio: aspect, child: _previewBox(t)),
                ),
              ),
              const SizedBox(height: 10),
              Expanded(child: _presetsStrip(t)),
            ],
          );
        }),
      );

  /// A row of small buttons where one is in force.
  Widget _segmented(
    LumitTheme t, {
    required String keyPrefix,
    required List<(String, String)> items,
    required String current,
    required ValueChanged<String> onPick,
  }) =>
      Container(
        decoration: BoxDecoration(
          color: t.surface0,
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        ),
        padding: const EdgeInsets.all(2),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final (key, label) in items)
              HouseButton(
                key: ValueKey('$keyPrefix-$key'),
                small: true,
                frameless: key != current,
                active: key == current,
                padding: const EdgeInsets.symmetric(horizontal: 8),
                onPressed: () => onPick(key),
                child: Text(label,
                    style: t.small.copyWith(
                        color: key == current ? t.textPrimary : null)),
              ),
          ],
        ),
      );

  Widget _previewBox(LumitTheme t) => LayoutBuilder(builder: (context, c) {
        final size = Size(c.maxWidth.floorToDouble(), c.maxHeight.floorToDouble());
        if (size != _previewSize && !size.isEmpty) {
          _previewSize = size;
          scheduleMicrotask(_requestPreview);
        }
        return ClipRRect(
          borderRadius: BorderRadius.circular(t.tokens.wellRadius),
          child: GestureDetector(
            key: const ValueKey('flare-preview'),
            behavior: HitTestBehavior.opaque,
            onPanDown: (d) => _dragLight(d.localPosition),
            onPanUpdate: (d) => _dragLight(d.localPosition),
            child: CustomPaint(
              painter: _PreviewPaint(
                ground: t.surface0,
                still: _frame ? _still : null,
                fit: _fit,
                comp: _comp,
                flare: _image,
                light: _light,
                ink: t.textPrimary,
              ),
            ),
          ),
        );
      });
}

Future<ui.Image> _decode(Uint8List bytes, int w, int h) {
  final done = Completer<ui.Image>();
  ui.decodeImageFromPixels(bytes, w, h, ui.PixelFormat.rgba8888, done.complete);
  return done.future;
}

/// The preview: the ground or the Frame still, the flare added over it, and
/// the light's handle and the centre mark on top.
class _PreviewPaint extends CustomPainter {
  final Color ground;
  final ui.Image? still;
  final ui.Image? flare;

  /// Fit draws the still over the whole box; 100% draws it at comp size
  /// about the middle, as the flare is drawn then.
  final bool fit;
  final Size comp;
  final Offset light;
  final Color ink;

  const _PreviewPaint({
    required this.ground,
    required this.still,
    required this.fit,
    required this.comp,
    required this.flare,
    required this.light,
    required this.ink,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final box = Offset.zero & size;
    canvas.drawRect(box, Paint()..color = ground);
    if (still case final still?) {
      final src = Rect.fromLTWH(
          0, 0, still.width.toDouble(), still.height.toDouble());
      final dst = fit
          ? box
          : Rect.fromCenter(
              center: box.center, width: comp.width, height: comp.height);
      canvas.drawImageRect(
          still, src, dst, Paint()..filterQuality = FilterQuality.medium);
    }
    if (flare case final flare?) {
      final src = Rect.fromLTWH(
          0, 0, flare.width.toDouble(), flare.height.toDouble());
      // Light adds: the flare is drawn on black, so black adds nothing.
      canvas.drawImageRect(flare, src, box, Paint()..blendMode = BlendMode.plus);
    }
    final centre = size.center(Offset.zero);
    final at = Offset(light.dx * size.width, light.dy * size.height);
    final dark = Paint()
      ..color = ground.withValues(alpha: 0.7)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 3;
    final line = Paint()
      ..color = ink.withValues(alpha: 0.8)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    void cross(Offset o, double r, Paint p) {
      canvas.drawLine(o - Offset(r, 0), o + Offset(r, 0), p);
      canvas.drawLine(o - Offset(0, r), o + Offset(0, r), p);
    }

    cross(centre, 5, dark);
    cross(centre, 5, line);
    canvas.drawCircle(at, 7, dark);
    canvas.drawCircle(at, 7, line);
    cross(at, 11, dark);
    cross(at, 11, line);
  }

  @override
  bool shouldRepaint(_PreviewPaint old) =>
      old.light != light ||
      old.ink != ink ||
      old.ground != ground ||
      old.still != still ||
      old.flare != flare ||
      old.fit != fit ||
      old.comp != comp;
}
