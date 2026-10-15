// The Interpret footage window.
//
// One window over a draft: what the picked items are read as is fetched once
// on the way in, edited here, and written as one undo step on OK. The numbers
// in the footer are the engine's own answer to "what would this make of the
// file", asked again after each edit and never worked out here.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/colour.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/interpret.dart';

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import 'comp_settings_frb.dart' show formatRate, parseRate;
import 'dialog_frame.dart';

const double _width = 480;
const double _labelColumn = 130;
const double _well = 80;

/// The rates the list beside the field offers, as their exact pairs.
const List<(String, int, int)> _rates = [
  ('23.976', 24000, 1001),
  ('24', 24, 1),
  ('25', 25, 1),
  ('29.97', 30000, 1001),
  ('30', 30, 1),
  ('50', 50, 1),
  ('59.94', 60000, 1001),
  ('60', 60, 1),
];

/// The pixel shapes with names, as their exact fractions. Format names, so
/// they are the same in every language.
const List<(String, int, int)> _shapes = [
  ('D1/DV NTSC', 10, 11),
  ('D1/DV NTSC Widescreen', 40, 33),
  ('D1/DV PAL', 59, 54),
  ('D1/DV PAL Widescreen', 118, 81),
  ('HDV 1080/DVCPRO HD 720', 4, 3),
  ('DVCPRO HD 1080', 3, 2),
  ('Anamorphic 2:1', 2, 1),
];

/// The cadences by where the file's first frame sits in them, whole (W) and
/// split (S) frames counted from there.
const List<String> _threeTwo = ['WWSSW', 'WSSWW', 'SSWWW', 'SWWWS', 'WWWSS'];
const List<String> _advanced = ['WWSWW', 'WSWWW', 'SWWWW', 'WWWWS', 'WWWSW'];

/// What Remember interpretation last took, for Apply interpretation to give.
/// Kept for the run of the app, as a clipboard is.
BridgeInterpretation? rememberedInterpretation;

/// Take [item]'s interpretation, to apply to others.
Future<void> rememberInterpretation(FootageReference item) async {
  try {
    rememberedInterpretation = (await item.interpretation()).now;
  } catch (_) {
    // The item went as the menu closed.
  }
}

/// Read every one of [items] the way the remembered one is. One undo step.
void applyRememberedInterpretation(List<FootageReference> items) {
  final remembered = rememberedInterpretation;
  if (remembered == null || items.isEmpty) return;
  try {
    setInterpretation(items: items, to: remembered);
  } catch (_) {
    // A rate the engine refuses changes nothing.
  }
}

/// Open the window on [items], which are all read alike on OK. [subject] is
/// what the title names them as, and [colour] the colour configuration, whose
/// spaces are offered. True when something was applied.
Future<bool> showInterpretFootageFrb({
  required BuildContext context,
  required List<FootageReference> items,
  required String subject,
  BridgeColourSummary? colour,
}) async {
  if (items.isEmpty) return false;
  final BridgeFootageReading reading;
  try {
    reading = await items.first.interpretation();
  } catch (_) {
    return false;
  }
  if (!context.mounted) return false;
  final applied = await showLumitModal<bool>(
    context: context,
    id: 'interpret-footage',
    builder: (close) => _InterpretBody(
      items: items,
      subject: subject,
      colour: colour,
      reading: reading,
      onDone: () => close(true),
      onCancel: () => close(false),
    ),
  );
  return applied ?? false;
}

enum _Page { main, colour }

class _InterpretBody extends StatefulWidget {
  final List<FootageReference> items;
  final String subject;
  final BridgeColourSummary? colour;
  final BridgeFootageReading reading;
  final VoidCallback onDone;
  final VoidCallback onCancel;

  const _InterpretBody({
    required this.items,
    required this.subject,
    this.colour,
    required this.reading,
    required this.onDone,
    required this.onCancel,
  });

  @override
  State<_InterpretBody> createState() => _InterpretBodyState();
}

class _InterpretBodyState extends State<_InterpretBody> {
  _Page _page = _Page.main;

  late BridgeAlphaMode _alpha;
  late List<int> _matte;
  late bool _invert;
  late final TextEditingController _fps;
  late final TextEditingController _timecode;
  late BridgeFieldOrder _fields;
  late bool _preserveEdges;
  late BridgePulldown _pulldown;
  late int _phase;
  late (int, int) _shape;
  late int _loops;
  late String? _colourSpace;
  late BridgeLinearLight _linear;
  late bool _preserveRgb;
  late String? _pass;

  /// What the draft would make of the file, from the engine.
  late BridgeInterpreted _made;

  /// A guess that came back with nothing, said beside its button.
  bool _noAlphaGuess = false;
  bool _noPulldownGuess = false;
  bool _guessing = false;

  BridgeFootageFacts get _file => widget.reading.file;

  /// Whether there are frames that run: a still has no rate, fields or loop.
  bool get _runs => _file.hasPicture && !_file.isStill;

  @override
  void initState() {
    super.initState();
    final now = widget.reading.now;
    _alpha = now.alpha;
    _matte = [now.matteR, now.matteG, now.matteB];
    _invert = now.invertAlpha;
    _fps = TextEditingController(
        text: now.fpsNum == 0 ? '' : formatRate(now.fpsNum, now.fpsDen));
    _timecode = TextEditingController(text: now.timecode ?? '');
    _fields = now.fields;
    _preserveEdges = now.preserveEdges;
    _pulldown = now.pulldown;
    _phase = now.pulldownPhase;
    _shape = (now.pixelAspectNum, now.pixelAspectDen);
    _loops = now.loops < 1 ? 1 : now.loops;
    _colourSpace = now.colourSpace;
    _linear = now.linearLight;
    _preserveRgb = now.preserveRgb;
    _pass = now.renderPass;
    _made = interpreted(file: _file, to: _draft());
    _fps.addListener(_edited);
    _timecode.addListener(_edited);
  }

  @override
  void dispose() {
    _fps.dispose();
    _timecode.dispose();
    super.dispose();
  }

  BridgeInterpretation _draft() {
    final rate = parseRate(_fps.text);
    final timecode = _timecode.text.trim();
    return BridgeInterpretation(
      alpha: _alpha,
      matteR: _matte[0],
      matteG: _matte[1],
      matteB: _matte[2],
      invertAlpha: _invert,
      fpsNum: rate?.$1 ?? 0,
      fpsDen: rate?.$2 ?? 1,
      timecode: timecode.isEmpty ? null : timecode,
      fields: _fields,
      preserveEdges: _preserveEdges,
      pulldown: _pulldown,
      pulldownPhase: _phase,
      pixelAspectNum: _shape.$1,
      pixelAspectDen: _shape.$2,
      loops: _loops,
      colourSpace: _colourSpace,
      linearLight: _linear,
      preserveRgb: _preserveRgb,
      renderPass: _pass,
    );
  }

  /// A typed field changed: ask the engine again what the draft makes.
  void _edited() {
    if (!mounted) return;
    setState(() => _made = interpreted(file: _file, to: _draft()));
  }

  void _set(VoidCallback change) {
    change();
    _edited();
  }

  void _confirm() {
    // Typed and not a rate: left for the user to put right.
    if (_fps.text.trim().isNotEmpty && parseRate(_fps.text) == null) return;
    try {
      setInterpretation(items: widget.items, to: _draft());
    } catch (_) {
      // A rate the engine refuses leaves the window open on it.
      return;
    }
    widget.onDone();
  }

  Future<void> _guessAlpha() async {
    setState(() => _guessing = true);
    BridgeAlphaGuess? guess;
    try {
      guess = await widget.items.first.guessAlpha();
    } catch (_) {}
    if (!mounted) return;
    _guessing = false;
    _noAlphaGuess = guess == null;
    if (guess case final found?) {
      _alpha = found.alpha;
      _matte = [found.matteR, found.matteG, found.matteB];
    }
    _edited();
  }

  Future<void> _guessPulldown() async {
    setState(() => _guessing = true);
    BridgePulldownGuess? guess;
    try {
      guess = await widget.items.first.guessPulldown();
    } catch (_) {}
    if (!mounted) return;
    _guessing = false;
    _noPulldownGuess = guess == null;
    if (guess case final found?) {
      _pulldown = found.pulldown;
      _phase = found.phase;
    }
    _edited();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: _width,
      children: [
        dialogTitleBar(
          t,
          title: l10n.interpretFootage,
          subject: widget.subject,
          onClose: widget.onCancel,
          keyPrefix: 'interpret',
        ),
        dialogTabs<_Page>(
          t,
          tabs: [
            (_Page.main, l10n.interpretTabMain),
            (_Page.colour, l10n.interpretTabColour),
          ],
          current: _page,
          onPick: (page) => setState(() => _page = page),
          keyPrefix: 'interpret',
        ),
        if (_page == _Page.main) ...[
          // Only a file with passes to choose between: a multi-pass EXR.
          if (_file.renderPasses.length > 1)
            _section(t, '', enabled: true, [
              _row(t, l10n.interpretRenderPass, _passRow(t)),
            ]),
          _section(t, l10n.interpretAlpha, enabled: _file.hasPicture, [
            _row(t, l10n.interpretReadAs, _alphaRow(t)),
            _tick(t, 'interpret-invert', l10n.interpretInvertAlpha, _invert,
                (on) => _set(() => _invert = on)),
          ]),
          _section(t, l10n.interpretTime, enabled: _runs, [
            _row(t, l10n.frameRate, _rateRow(t)),
            _row(t, l10n.interpretStartTimecode, _timecodeRow(t)),
          ]),
          _section(t, l10n.interpretFieldsGroup, enabled: _runs, [
            _row(t, l10n.interpretSeparateFields, _fieldsRow(t)),
            _tick(
                t,
                'interpret-preserve-edges',
                l10n.interpretPreserveEdges,
                _preserveEdges,
                _fields == BridgeFieldOrder.off
                    ? null
                    : (on) => _set(() => _preserveEdges = on)),
            _row(t, l10n.interpretRemovePulldown, _pulldownRow(t)),
          ]),
          _section(t, l10n.interpretOther, enabled: _file.hasPicture, [
            _row(t, l10n.interpretPixelShape, _shapeRow(t)),
            _row(t, l10n.interpretLoop, _loopRow(t)),
          ]),
        ] else
          _section(t, '', enabled: _file.hasPicture, [
            _row(t, l10n.colourSpace, _spaceRow(t)),
            _row(t, l10n.interpretLinearLight, _linearRow(t)),
            _tick(t, 'interpret-preserve-rgb', l10n.interpretPreserveRgb,
                _preserveRgb, (on) => _set(() => _preserveRgb = on)),
          ]),
        dialogFooter(
          t,
          keyPrefix: 'interpret',
          summary: _summary(),
          actions: [
            HouseButton(
              key: const ValueKey('interpret-cancel'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: widget.onCancel,
              child: Text(l10n.cancel),
            ),
            HouseButton(
              key: const ValueKey('interpret-apply'),
              primary: true,
              autofocus: true,
              padding: const EdgeInsets.symmetric(horizontal: 16),
              onPressed: _confirm,
              child: Text(l10n.apply),
            ),
          ],
        ),
      ],
    );
  }

  /// The file as the draft reads it: size, rate and length.
  String _summary() {
    if (!_file.hasPicture) return '';
    final size = '${_made.width} × ${_made.height}';
    if (_file.isStill || _made.fpsNum == 0) return size;
    final rate = formatRate(_made.fpsNum, _made.fpsDen);
    final length =
        timecodeOfRate(_made.frames.toInt(), _made.fpsNum, _made.fpsDen);
    return '$size · $rate ${l10n.unitFps} · $length';
  }

  // ---- the rows ------------------------------------------------------------

  Widget _alphaRow(LumitTheme t) => Row(
        children: [
          dialogDropdown<BridgeAlphaMode>(
            t,
            id: 'interpret-alpha',
            value: _alpha,
            options: BridgeAlphaMode.values,
            label: (mode) => switch (mode) {
              BridgeAlphaMode.straight => l10n.interpretAlphaStraight,
              BridgeAlphaMode.ignore => l10n.interpretAlphaIgnore,
              BridgeAlphaMode.premultiplied => l10n.interpretAlphaPremultiplied,
            },
            onChanged: (mode) => _set(() => _alpha = mode),
            width: 140,
          ),
          if (_alpha == BridgeAlphaMode.premultiplied) ...[
            const SizedBox(width: 8),
            LumitTooltip(
              message: l10n.tipInterpretMatte,
              child: GestureDetector(
                key: const ValueKey('interpret-matte'),
                behavior: HitTestBehavior.opaque,
                onTapDown: (details) => _pickMatte(details.globalPosition),
                child: Container(
                  width: 20,
                  height: 16,
                  decoration: BoxDecoration(
                    color: _matteColour,
                    border: Border.all(color: t.hairline),
                    borderRadius: BorderRadius.circular(t.tokens.sectionRadius),
                  ),
                ),
              ),
            ),
          ],
          const Spacer(),
          if (_noAlphaGuess) ...[
            Text(l10n.interpretNoGuess, style: dialogMono(t)),
            const SizedBox(width: 8),
          ],
          HouseButton(
            key: const ValueKey('interpret-guess-alpha'),
            small: true,
            onPressed: _guessing ? null : _guessAlpha,
            child: Text(l10n.interpretGuess, style: t.small),
          ),
        ],
      );

  Color get _matteColour =>
      Color.fromARGB(255, _matte[0], _matte[1], _matte[2]);

  void _pickMatte(Offset position) {
    final t = ThemeScope.of(context).theme;
    showColourPicker(
      context: context,
      position: position,
      initial: PickedColour.of(_matteColour),
      presets: t.backgroundPresets,
      onCommit: (picked) => _set(() => _matte = [
            for (final v in [picked.r, picked.g, picked.b])
              (v.clamp(0.0, 1.0) * 255).round(),
          ]),
    );
  }

  Widget _rateRow(LumitTheme t) {
    final own = formatRate(_file.fpsNum, _file.fpsDen);
    final typed = _fps.text.trim();
    return Row(
      children: [
        SizedBox(
          width: _well,
          height: dialogControlHeight,
          child: HouseTextField(
            key: const ValueKey('interpret-fps'),
            controller: _fps,
            width: double.infinity,
            padding: const EdgeInsets.symmetric(horizontal: 8),
            fill: t.surface0,
            textAlign: TextAlign.right,
            // Empty is the file's own rate, which the hint shows.
            hint: _file.isSequence ? null : own,
            onSubmitted: (_) => _confirm(),
          ),
        ),
        const SizedBox(width: 6),
        Text(l10n.unitFps, style: dialogMono(t)),
        const Spacer(),
        dialogDropdown<String>(
          t,
          id: 'interpret-fps-presets',
          value: typed.isEmpty
              ? l10n.interpretFromFile(own)
              : (_rates.any((r) => r.$1 == typed) ? typed : l10n.custom),
          options: [
            if (!_file.isSequence) l10n.interpretFromFile(own),
            ..._rates.map((r) => r.$1),
          ],
          label: (s) => s,
          onChanged: (picked) =>
              _fps.text = _rates.any((r) => r.$1 == picked) ? picked : '',
          width: 150,
        ),
      ],
    );
  }

  Widget _timecodeRow(LumitTheme t) => Row(
        children: [
          SizedBox(
            width: 120,
            height: dialogControlHeight,
            child: HouseTextField(
              key: const ValueKey('interpret-timecode'),
              controller: _timecode,
              width: double.infinity,
              padding: const EdgeInsets.symmetric(horizontal: 8),
              fill: t.surface0,
              // Empty is the file's own, which the hint shows.
              hint: _file.timecode ?? '00:00:00:00',
              onSubmitted: (_) => _confirm(),
            ),
          ),
        ],
      );

  Widget _fieldsRow(LumitTheme t) => Row(
        children: [
          dialogDropdown<BridgeFieldOrder>(
            t,
            id: 'interpret-fields',
            value: _fields,
            options: BridgeFieldOrder.values,
            label: _fieldsLabel,
            onChanged: (order) => _set(() => _fields = order),
            width: 170,
          ),
          const Spacer(),
          // What the file itself declares, when it declares anything.
          if (_file.fields != BridgeFieldOrder.off)
            Flexible(
              child: Text(
                l10n.interpretFromFile(_fieldsLabel(_file.fields)),
                style: dialogMono(t),
                overflow: TextOverflow.ellipsis,
              ),
            ),
        ],
      );

  String _fieldsLabel(BridgeFieldOrder order) => switch (order) {
        BridgeFieldOrder.off => l10n.off,
        BridgeFieldOrder.upper => l10n.interpretUpperFirst,
        BridgeFieldOrder.lower => l10n.interpretLowerFirst,
      };

  Widget _pulldownRow(LumitTheme t) {
    final options = <(BridgePulldown, int)>[
      (BridgePulldown.off, 0),
      for (var i = 0; i < 5; i++) (BridgePulldown.threeTwo, i),
      for (var i = 0; i < 5; i++) (BridgePulldown.advanced, i),
    ];
    final separated = _fields != BridgeFieldOrder.off;
    return Row(
      children: [
        dialogDropdown<(BridgePulldown, int)>(
          t,
          id: 'interpret-pulldown',
          value: separated ? (_pulldown, _phase) : (BridgePulldown.off, 0),
          options: options,
          label: (o) => switch (o.$1) {
            BridgePulldown.off => l10n.off,
            BridgePulldown.threeTwo => _threeTwo[o.$2 % 5],
            BridgePulldown.advanced =>
              l10n.interpretAdvancedPulldown(_advanced[o.$2 % 5]),
          },
          // A cadence is made of fields, so there is none to pick without.
          onChanged: separated
              ? (o) => _set(() {
                    _pulldown = o.$1;
                    _phase = o.$2;
                  })
              : null,
          width: 170,
        ),
        const Spacer(),
        if (_noPulldownGuess) ...[
          Text(l10n.interpretNoGuess, style: dialogMono(t)),
          const SizedBox(width: 8),
        ],
        HouseButton(
          key: const ValueKey('interpret-guess-pulldown'),
          small: true,
          onPressed: _guessing || !separated ? null : _guessPulldown,
          child: Text(l10n.interpretGuess, style: t.small),
        ),
      ],
    );
  }

  Widget _shapeRow(LumitTheme t) {
    final fromFile = (_file.pixelAspectNum, _file.pixelAspectDen);
    bool same((int, int) a, (int, int) b) => a.$1 * b.$2 == b.$1 * a.$2;
    final named = <(String, int, int)>[
      (l10n.interpretSquarePixels, 1, 1),
      ..._shapes,
      if (![(1, 1), ..._shapes.map((s) => (s.$2, s.$3))]
          .any((s) => same(s, fromFile)))
        (
          l10n.interpretFromFile(_ratio(fromFile)),
          fromFile.$1,
          fromFile.$2
        ),
    ];
    final current = named.where((s) => same((s.$2, s.$3), _shape)).firstOrNull;
    return Row(
      children: [
        Expanded(
          child: dialogDropdown<(String, int, int)?>(
            t,
            id: 'interpret-shape',
            value: current,
            options: [...named, if (current == null) null],
            label: (s) => s == null
                ? l10n.custom
                : (s.$2 == s.$3 ? s.$1 : '${s.$1} (${_ratio((s.$2, s.$3))})'),
            onChanged: (s) {
              if (s != null) _set(() => _shape = (s.$2, s.$3));
            },
          ),
        ),
        const SizedBox(width: 8),
        SizedBox(
          width: 64,
          height: dialogControlHeight,
          child: DragValueField(
            key: const ValueKey('interpret-shape-ratio'),
            value: _shape.$2 == 0 ? 1.0 : _shape.$1 / _shape.$2,
            min: 0.1,
            max: 10,
            speed: 0.01,
            decimals: 3,
            fill: t.surface0,
            onChanged: (v) =>
                _set(() => _shape = ((v.toDouble() * 1000).round(), 1000)),
          ),
        ),
      ],
    );
  }

  String _ratio((int, int) shape) =>
      shape.$2 == 0 ? '1' : (shape.$1 / shape.$2).toStringAsFixed(2);

  Widget _loopRow(LumitTheme t) => Row(
        children: [
          SizedBox(
            width: _well,
            height: dialogControlHeight,
            child: DragValueField(
              key: const ValueKey('interpret-loops'),
              value: _loops,
              min: 1,
              max: 9999,
              fill: t.surface0,
              onChanged: _runs ? (v) => _set(() => _loops = v.toInt()) : (_) {},
            ),
          ),
          const SizedBox(width: 6),
          Text(l10n.interpretLoopTimes, style: dialogMono(t)),
        ],
      );

  Widget _passRow(LumitTheme t) {
    final passes = _file.renderPasses;
    return dialogDropdown<String?>(
      t,
      id: 'interpret-render-pass',
      value: _pass,
      options: [
        null,
        ...passes,
        // A pass the file no longer has is kept, and reads as Auto.
        if (_pass != null && !passes.contains(_pass)) _pass,
      ],
      label: (pass) =>
          pass ?? l10n.interpretRenderPassAuto(_file.autoPass ?? ''),
      onChanged: (pass) => _set(() => _pass = pass),
    );
  }

  Widget _spaceRow(LumitTheme t) {
    final colour = widget.colour;
    final spaces =
        colour != null && colour.loaded ? colour.spaces : const <String>[];
    return dialogDropdown<String?>(
      t,
      id: 'interpret-colour-space',
      value: _colourSpace,
      options: [
        null,
        ...spaces,
        // A name the loaded configuration doesn't have is kept, not dropped.
        if (_colourSpace != null && !spaces.contains(_colourSpace))
          _colourSpace,
      ],
      label: (space) => space ?? l10n.colourSpaceFromFile,
      // The same spaces the Project panel's menu greys out, for the same reason.
      disabledReason: (space) => space == null || colour == null
          ? null
          : colourItemProblem(colour, BridgeColourItem.input, space),
      // Preserved numbers take no colour space at all.
      onChanged:
          _preserveRgb ? null : (space) => _set(() => _colourSpace = space),
    );
  }

  Widget _linearRow(LumitTheme t) => Row(
        children: [
          dialogDropdown<BridgeLinearLight>(
            t,
            id: 'interpret-linear',
            value: _linear,
            options: BridgeLinearLight.values,
            label: (mode) => switch (mode) {
              BridgeLinearLight.auto => _file.float
                  ? l10n.interpretLinearAutoOn
                  : l10n.interpretLinearAutoOff,
              BridgeLinearLight.on_ => l10n.interpretLinearOn,
              BridgeLinearLight.off => l10n.off,
            },
            // An assigned colour space says what curve the file has.
            onChanged: _preserveRgb || _colourSpace != null
                ? null
                : (mode) => _set(() => _linear = mode),
            width: 170,
          ),
        ],
      );

  // ---- the pieces ----------------------------------------------------------

  Widget _row(LumitTheme t, String label, Widget control) =>
      dialogRow(t, label, control, labelColumn: _labelColumn, minHeight: 30);

  /// A tick with its words, set under the controls' own column.
  Widget _tick(LumitTheme t, String key, String label, bool value,
          ValueChanged<bool>? onChanged) =>
      Padding(
        padding: const EdgeInsets.only(left: _labelColumn + 10),
        child: SizedBox(
          height: 26,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: onChanged == null ? null : () => onChanged(!value),
            child: Row(
              children: [
                HouseCheckbox(
                  key: ValueKey<String>(key),
                  value: value,
                  onChanged: onChanged,
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    label,
                    style: t.small.copyWith(
                        color: onChanged == null ? t.textMuted : null),
                  ),
                ),
              ],
            ),
          ),
        ),
      );

  /// A named group of rows under a rule. One the file has no use for, a still's
  /// frame rate, is drawn and deaf: off is not gone.
  Widget _section(LumitTheme t, String title, List<Widget> rows,
          {required bool enabled}) =>
      Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(height: 1, color: t.hairline),
          Padding(
            padding:
                const EdgeInsets.fromLTRB(dialogPadding, 4, dialogPadding, 8),
            child: IgnorePointer(
              ignoring: !enabled,
              child: Opacity(
                opacity: enabled ? 1 : 0.45,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    // A page with one group has its name on the tab already.
                    if (title.isEmpty)
                      const SizedBox(height: 6)
                    else
                      Padding(
                        padding: const EdgeInsets.only(top: 8, bottom: 4),
                        child: Text(t.kickerCase(title), style: t.kicker),
                      ),
                    ...rows,
                  ],
                ),
              ),
            ),
          ),
        ],
      );
}
