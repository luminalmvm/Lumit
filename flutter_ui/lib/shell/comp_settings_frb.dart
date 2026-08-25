// The Composition settings dialog, and its twin the New composition dialog.
//
// One body serves both, because they ask the same questions and differ only in
// what happens when you press the button at the bottom. Reached from the menu
// bar's Composition ▸ Composition settings…, from the Project panel's context
// menu on a comp, and from the Project panel's New composition button
// (including when footage is dropped on it, which prefills the fields from the
// media — docs/07 §3.1).
//
// **The shape is the approved drawing's** (K-469), and it is the *same* popup
// the export dialog is built from: a kicker title strip, label-left rows in a
// 110px column with 12 after it, kicker-titled sections separated by a rule,
// and a footer carrying Cancel and the single filled action. `dialog_frame.dart`
// holds the pieces; this file holds the questions.
//
// **The frame rate is one number, and the duration is a length of time.** Both
// are deliberate:
//
// * The rate reads as `600` or `23.976`, not as a numerator over a denominator.
//   The exact pair still crosses the bridge — 23.976 is 24000/1001 and a float
//   round trip would not give that back (docs/14 §2) — but the pair is worked out
//   here from the number typed, and the awkward rates are one click away in the
//   Presets list, so nobody has to know that 1001 exists.
// * The duration reads and edits as `HH:MM:SS:FF` timecode — the same clock
//   face the Viewer shows — but what is *written* is still a length in seconds,
//   converted at the rate typed above it. Seconds in the document is what fixes
//   the old "changing the rate retimes the comp" bug (K-180): a frame count
//   means nothing without the rate it was counted at, so storing yesterday's
//   count back at a new rate changed how long the comp really was while every
//   layer kept its own seconds — which looked exactly like the layers speeding
//   up or slowing down.

import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

/// The frame the drawing gives this dialog, and its row — a 110px label column
/// with 12 after it, in rows of 30. Not the Export dialog's (K-458, K-469:
/// each drawing measures its own).
const double compDialogWidth = 520;
const double compLabelColumn = 110;
const double compRowGap = 12;
const double compRowHeight = 30;

/// A section: the rule above it, the 4 of air the drawing leaves under that,
/// and the kicker band over the rows.
const double compSectionKicker = 24;
const double compSectionTop = 4;

/// The widths the drawing gives this dialog's wells and its rate list.
const double compSizeWell = 80;
const double compDurationWell = 120;
const double compSamplesWell = 60;
const double compRateList = 90;

/// The background swatch, and the room between the shutter angle and the
/// sample count beside it.
const Size compSwatch = Size(20, 16);
const double compShutterGap = 50;

/// The frame rates worth one click. The `1001` denominators are the NTSC family,
/// which is the whole reason the rate crosses the bridge as a pair.
const List<(String, int, int)> _ratePresets = [
  ('23.976', 24000, 1001),
  ('24', 24, 1),
  ('25', 25, 1),
  ('29.97', 30000, 1001),
  ('30', 30, 1),
  ('50', 50, 1),
  ('59.94', 60000, 1001),
  ('60', 60, 1),
  ('120', 120, 1),
];

/// The whole frames the drawing's Preset list offers: a name, a size and a
/// rate. The names are proper nouns of the trade and are not translated; the
/// row reads "HD 1080p · 25" through one arb pattern.
const List<(String, int, int, int, int)> _compPresets = [
  ('HD 1080p', 1920, 1080, 25, 1),
  ('HD 1080p', 1920, 1080, 30, 1),
  ('HD 1080p', 1920, 1080, 60, 1),
  ('HD 720p', 1280, 720, 25, 1),
  ('UHD 4K', 3840, 2160, 25, 1),
  ('Cinema 2K', 2048, 1080, 24, 1),
  ('Vertical 1080', 1080, 1920, 30, 1),
];

/// Edit an existing comp. Returns true when settings were applied, so the caller
/// can refresh; false when cancelled.
Future<bool> showCompSettingsFrb({
  required BuildContext context,
  required CompositionReference comp,
}) async {
  final applied = await showLumitModal<bool>(
    context: context,
    id: 'comp-settings',
    builder: (close) => _CompSettingsBody(
      title: l10n.compositionSettings,
      confirm: l10n.save,
      initial: comp.getSettings(),
      onConfirm: (settings) {
        // **The playhead keeps its moment, not its number** (K-572). A frame
        // count means nothing without the rate it was counted at, so the time
        // under the playhead is read *before* the rate is written and the
        // nearest frame of the new grid asked for after — the engine does both
        // conversions, exactly, because 29.97's boundaries do not survive a
        // float. Markers and the work area need none of this: both are stored
        // as rational time already, so they keep their moments untouched.
        //
        // Only when this comp is the one being looked at: there is one
        // playhead, and changing a background comp's rate must not move it.
        final ui = Provider.of<LumitUiState>(context, listen: false);
        final mine = ui.selectedComp?.internalid == comp.internalid;
        final was = mine ? comp.timeOfFrame(frame: ui.playheadFrame.value) : null;
        comp.setSettings(settings: settings);
        if (was != null) {
          ui.playheadFrame.value = comp.nearestFrameAtTime(time: was);
        }
        close(true);
      },
      onCancel: () => close(false),
    ),
  );
  return applied ?? false;
}

/// Make a comp, asking first. Returns the new comp, or null when cancelled.
///
/// `footage` is what was dropped on the New composition button: the fields open
/// on the media's own size, rate and length, and every item lands in the finished
/// comp as a layer. An empty list is the plain New composition command.
Future<CompositionReference?> showNewCompositionFrb({
  required BuildContext context,
  required ProjectReference project,
  List<FootageReference> footage = const [],

  /// Settings ▸ Interface ▸ Editing ▸ *Video arrives as a Sequence layer*
  /// (K-246), forwarded to the engine for each item placed below. Taken as an
  /// argument rather than read from the workspace here, because this file is
  /// a dialog and knows nothing about where settings live.
  bool asSequence = false,
}) async {
  // Probed before the dialog opens rather than inside it: `mediaInfo` reads the
  // container with FFmpeg, and a dialog that popped up and then rearranged itself
  // is worse than one that appears already right.
  var initial = BridgeCompSettings.defaults();
  for (final item in footage) {
    final info = await item.mediaInfo();
    if (info == null) continue;
    initial = BridgeCompSettings(
      name: initial.name,
      // Audio-only media has no picture to size a comp by, so it keeps whatever
      // the previous item (or the default) said.
      width: info.width > 0 ? info.width : initial.width,
      height: info.height > 0 ? info.height : initial.height,
      fpsNum: info.fpsNum > 0 ? info.fpsNum : initial.fpsNum,
      fpsDen: info.fpsNum > 0 ? info.fpsDen : initial.fpsDen,
      // The longest item wins: a comp shorter than something dropped into it
      // would clip the very thing that was asked for.
      duration: _longer(initial.duration, info.duration),
      background: initial.background,
      shutterAngle: initial.shutterAngle,
      motionBlurSamples: initial.motionBlurSamples,
    );
  }
  if (!context.mounted) return null;

  final name = project.nextCompName();
  return showLumitModal<CompositionReference>(
    context: context,
    id: 'new-comp',
    builder: (close) => _CompSettingsBody(
      title: l10n.newComposition,
      confirm: l10n.create,
      initial: BridgeCompSettings(
        name: name,
        width: initial.width,
        height: initial.height,
        fpsNum: initial.fpsNum,
        fpsDen: initial.fpsDen,
        duration: initial.duration,
        background: initial.background,
        shutterAngle: initial.shutterAngle,
        motionBlurSamples: initial.motionBlurSamples,
      ),
      onConfirm: (settings) {
        final comp =
            project.newComposition(name: settings.name, settings: settings);
        for (final item in footage) {
          comp.addFootageLayer(footage: item, asSequence: asSequence);
        }
        close(comp);
      },
      onCancel: () => close(null),
    ),
  );
}

/// The longer of two exact durations, compared by cross-multiplication so no
/// float ever decides which of two lengths is bigger.
BridgeRational _longer(BridgeRational a, BridgeRational b) =>
    a.num.toInt() * b.den.toInt() >= b.num.toInt() * a.den.toInt() ? a : b;

class _CompSettingsBody extends StatefulWidget {
  final String title;
  final String confirm;
  final BridgeCompSettings initial;
  final void Function(BridgeCompSettings) onConfirm;
  final VoidCallback onCancel;

  const _CompSettingsBody({
    required this.title,
    required this.confirm,
    required this.initial,
    required this.onConfirm,
    required this.onCancel,
  });

  @override
  State<_CompSettingsBody> createState() => _CompSettingsBodyState();
}

class _CompSettingsBodyState extends State<_CompSettingsBody> {
  late final TextEditingController _name;
  late final TextEditingController _fps;
  late final TextEditingController _duration;
  late int _width;
  late int _height;
  late List<double> _background;
  late double _shutterAngle;
  late int _samples;

  /// Keep the shape when one side is edited. On by default, because resizing a
  /// comp to a shape it was never meant to be is nearly always a slip.
  bool _locked = true;

  @override
  void initState() {
    super.initState();
    final s = widget.initial;
    _name = TextEditingController(text: s.name);
    _fps = TextEditingController(text: _formatRate(s.fpsNum, s.fpsDen))
      // The list beside the field names whatever the field says, so it has to
      // follow every keystroke — not wait for the field to be submitted, which
      // would leave it naming the rate before last.
      ..addListener(() => setState(() {}));
    _duration = TextEditingController(
        text: timecodeOfDuration(s.duration, s.fpsNum, s.fpsDen));
    _width = s.width;
    _height = s.height;
    _background = List<double>.from(s.background);
    _shutterAngle = s.shutterAngle;
    _samples = s.motionBlurSamples;
  }

  @override
  void dispose() {
    _name.dispose();
    _fps.dispose();
    _duration.dispose();
    super.dispose();
  }

  /// The preset label matching what is typed, or null for a rate of one's own.
  String? get _presetLabel {
    final rate = parseRate(_fps.text);
    if (rate == null) return null;
    return _ratePresets
        .where((p) => p.$2 == rate.$1 && p.$3 == rate.$2)
        .map((p) => p.$1)
        .firstOrNull;
  }

  /// The whole-frame preset the fields currently say, or null when they say
  /// something of their own.
  (String, int, int, int, int)? get _compPreset {
    final rate = parseRate(_fps.text);
    if (rate == null) return null;
    for (final preset in _compPresets) {
      if (preset.$2 == _width &&
          preset.$3 == _height &&
          preset.$4 == rate.$1 &&
          preset.$5 == rate.$2) {
        return preset;
      }
    }
    return null;
  }

  void _confirm() {
    final rate =
        parseRate(_fps.text) ?? (widget.initial.fpsNum, widget.initial.fpsDen);
    // Timecode first, at the rate typed above; the old `HH:MM:SS.mmm` and
    // bare-seconds forms still parse as a courtesy. A duration that cannot be
    // read at all is the one that was already there rather than a comp of no
    // length: a typo must not be able to throw work away.
    final frames = framesOfTimecode(_duration.text, rate.$1, rate.$2);
    final duration = frames != null
        ? secondsOfFrames(frames, rate.$1, rate.$2)
        : parseDurationHms(_duration.text) ?? widget.initial.duration;
    widget.onConfirm(BridgeCompSettings(
      name: _name.text.trim().isEmpty ? widget.initial.name : _name.text.trim(),
      width: _width,
      height: _height,
      fpsNum: rate.$1,
      fpsDen: rate.$2,
      duration: duration,
      background: F32Array4(Float32List.fromList(_background)),
      shutterAngle: _shutterAngle,
      motionBlurSamples: _samples,
    ));
  }

  /// Editing one side of the size, carrying the other with it when locked.
  void _setSize({int? width, int? height}) {
    setState(() {
      if (width != null) {
        final ratio = _height / _width;
        _width = width;
        if (_locked) _height = (width * ratio).round().clamp(16, 16384);
      }
      if (height != null) {
        final ratio = _width / _height;
        _height = height;
        if (_locked) _width = (height * ratio).round().clamp(16, 16384);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: compDialogWidth,
      children: [
        dialogTitleBar(
          t,
          title: widget.title,
          onClose: widget.onCancel,
          keyPrefix: 'comp',
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(
              dialogPadding, dialogPadding, dialogPadding, 10),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _row(
                t,
                l10n.name,
                SizedBox(
                  height: dialogControlHeight,
                  child: HouseTextField(
                    key: const ValueKey('comp-name'),
                    controller: _name,
                    width: double.infinity,
                    padding: const EdgeInsets.symmetric(horizontal: 8),
                    fill: t.surface0,
                    onSubmitted: (_) => _confirm(),
                  ),
                ),
              ),
              _row(t, l10n.exportPreset, _presetRow(t)),
            ],
          ),
        ),
        _section(t, l10n.compGroupFrame, [
          _row(t, l10n.size, _sizeRow(t)),
          _row(t, l10n.frameRate, _rateRow(t)),
          _row(t, l10n.duration, _durationRow(t)),
          _row(t, l10n.compBackground, _backgroundRow(t)),
        ]),
        _section(t, l10n.compGroupMotionBlur, [
          _row(t, l10n.compShutterAngle, _shutterRow(t)),
        ]),
        dialogFooter(
          t,
          keyPrefix: 'comp',
          actions: [
            HouseButton(
              key: const ValueKey('comp-cancel'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: widget.onCancel,
              child: Text(l10n.cancel),
            ),
            HouseButton(
              key: const ValueKey('comp-apply'),
              // The window's default action (K-319): focused on open so Enter
              // applies; a field being typed in keeps Enter for its own submit,
              // which calls the same confirm.
              primary: true,
              autofocus: true,
              padding: const EdgeInsets.symmetric(horizontal: 16),
              onPressed: _confirm,
              child: Text(widget.confirm),
            ),
          ],
        ),
      ],
    );
  }

  // ---- the rows ------------------------------------------------------------

  Widget _presetRow(LumitTheme t) {
    final current = _compPreset;
    return dialogDropdown<(String, int, int, int, int)?>(
      t,
      id: 'comp-preset',
      value: current,
      options: [null, ..._compPresets],
      label: (preset) => preset == null
          ? l10n.custom
          : l10n.compPresetLabel(preset.$1, _formatRate(preset.$4, preset.$5)),
      onChanged: (preset) {
        if (preset == null) return;
        setState(() {
          _width = preset.$2;
          _height = preset.$3;
          _fps.text = _formatRate(preset.$4, preset.$5);
        });
      },
    );
  }

  Widget _sizeRow(LumitTheme t) => Row(
        children: [
          SizedBox(
            width: compSizeWell,
            height: dialogControlHeight,
            child: DragValueField(
              key: const ValueKey('comp-width'),
              value: _width,
              min: 16,
              max: 16384,
              fill: t.surface0,
              onChanged: (v) => _setSize(width: v.toInt()),
            ),
          ),
          // The drawing's chain link between the two sides: the shape is kept
          // while it is joined, and each side is its own while it is not.
          //
          // **A link, not a padlock.** The drawing puts a chain here, and a
          // chain is what the gesture means — these two numbers move together
          // — where a padlock says the numbers cannot be changed at all, which
          // is the opposite of true.
          LumitTooltip(
            message: _locked ? l10n.tipAspectLocked : l10n.tipAspectUnlocked,
            child: GestureDetector(
              key: const ValueKey('comp-size-lock'),
              behavior: HitTestBehavior.opaque,
              onTap: () => setState(() => _locked = !_locked),
              child: SizedBox(
                width: 24,
                height: dialogControlHeight,
                child: Center(
                  child: lumitIcon(
                    _locked ? LumitIcon.link : LumitIcon.unlink,
                    size: 12,
                    color: _locked ? t.textPrimary : t.textMuted,
                  ),
                ),
              ),
            ),
          ),
          SizedBox(
            width: compSizeWell,
            height: dialogControlHeight,
            child: DragValueField(
              key: const ValueKey('comp-height'),
              value: _height,
              min: 16,
              max: 16384,
              fill: t.surface0,
              onChanged: (v) => _setSize(height: v.toInt()),
            ),
          ),
          const SizedBox(width: 6),
          Text(l10n.unitSymbolPx, style: dialogMono(t)),
          const Spacer(),
          Text(
            aspectRatioLabel(_width, _height),
            key: const ValueKey('comp-aspect'),
            style: dialogMono(t),
          ),
        ],
      );

  Widget _rateRow(LumitTheme t) => Row(
        children: [
          SizedBox(
            width: compSizeWell,
            height: dialogControlHeight,
            child: HouseTextField(
              key: const ValueKey('comp-fps'),
              controller: _fps,
              width: double.infinity,
              padding: const EdgeInsets.symmetric(horizontal: 8),
              fill: t.surface0,
              // The drawing right-aligns every numeric well it draws, and the
              // rate is one: the digits line up with the size wells above it
              // and the shutter angle below.
              textAlign: TextAlign.right,
              onSubmitted: (_) => _confirm(),
            ),
          ),
          const SizedBox(width: 6),
          Text(l10n.unitFps, style: dialogMono(t)),
          const Spacer(),
          dialogDropdown<String>(
            t,
            id: 'comp-fps-presets',
            // A rate of one's own reads as "Custom" rather than as an empty
            // invitation: the list is where you *change* the rate, and what it
            // shows is what the field beside it currently says.
            value: _presetLabel ?? l10n.custom,
            options: [..._ratePresets.map((p) => p.$1)],
            label: (s) => s,
            onChanged: (picked) => setState(() => _fps.text = picked),
            width: compRateList,
          ),
        ],
      );

  Widget _durationRow(LumitTheme t) {
    final rate =
        parseRate(_fps.text) ?? (widget.initial.fpsNum, widget.initial.fpsDen);
    final frames = framesOfTimecode(_duration.text, rate.$1, rate.$2);
    final seconds = rate.$1 == 0 ? 0.0 : (frames ?? 0) * rate.$2 / rate.$1;
    return Row(
      children: [
        SizedBox(
          width: compDurationWell,
          height: dialogControlHeight,
          child: HouseTextField(
            key: const ValueKey('comp-duration'),
            controller: _duration,
            width: double.infinity,
            padding: const EdgeInsets.symmetric(horizontal: 8),
            fill: t.surface0,
            onSubmitted: (_) => _confirm(),
          ),
        ),
        const SizedBox(width: 6),
        // The drawing's reading, which is also the note the old dialog spent a
        // whole line on: what the timecode above means, said in frames and
        // seconds rather than explained in a sentence.
        Flexible(
          child: Text(
            l10n.compDurationReading(
                '${frames ?? 0}', seconds.toStringAsFixed(1)),
            key: const ValueKey('comp-duration-reading'),
            style: dialogMono(t),
            overflow: TextOverflow.ellipsis,
          ),
        ),
      ],
    );
  }

  Widget _backgroundRow(LumitTheme t) => Row(
        children: [
          GestureDetector(
            key: const ValueKey('comp-background'),
            behavior: HitTestBehavior.opaque,
            onTapDown: (details) => _pickBackground(details.globalPosition),
            child: Container(
              width: compSwatch.width,
              height: compSwatch.height,
              decoration: BoxDecoration(
                color: _backgroundColour,
                border: Border.all(color: t.hairline),
                borderRadius: BorderRadius.circular(dialogGroupRadius),
              ),
            ),
          ),
          const SizedBox(width: 8),
          Text(_backgroundHex, style: dialogMono(t)),
        ],
      );

  Widget _shutterRow(LumitTheme t) => Row(
        children: [
          SizedBox(
            width: compSizeWell,
            height: dialogControlHeight,
            child: DragValueField(
              key: const ValueKey('comp-shutter-angle'),
              value: _shutterAngle,
              min: 0,
              max: 720,
              decimals: 1,
              fill: t.surface0,
              onChanged: (v) => setState(() => _shutterAngle = v.toDouble()),
            ),
          ),
          const SizedBox(width: 6),
          Text(l10n.unitSymbolDegrees, style: dialogMono(t)),
          const SizedBox(width: compShutterGap),
          Text(l10n.compSamples, style: t.body),
          const SizedBox(width: 6),
          SizedBox(
            width: compSamplesWell,
            height: dialogControlHeight,
            child: DragValueField(
              key: const ValueKey('comp-samples'),
              value: _samples,
              min: 1,
              max: 256,
              fill: t.surface0,
              onChanged: (v) => setState(() => _samples = v.toInt()),
            ),
          ),
        ],
      );

  // ---- the pieces ----------------------------------------------------------

  Widget _row(LumitTheme t, String label, Widget control) => dialogRow(
        t,
        label,
        control,
        labelColumn: compLabelColumn,
        gap: compRowGap,
        minHeight: compRowHeight,
      );

  /// A named group of rows: a rule, the drawing's 4 of air, a kicker band, and
  /// the rows under it. The Settings pages' shape rather than the Export
  /// dialog's box — this drawing separates its sections with a line (K-469).
  Widget _section(LumitTheme t, String title, List<Widget> rows) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(height: 1, color: t.hairline),
          Padding(
            padding: const EdgeInsets.fromLTRB(
                dialogPadding, compSectionTop, dialogPadding, 10),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [
                SizedBox(
                  height: compSectionKicker,
                  child: Padding(
                    padding: const EdgeInsets.only(top: 8, bottom: 4),
                    child: Align(
                      alignment: Alignment.centerLeft,
                      child: Text(title.toUpperCase(), style: t.kicker),
                    ),
                  ),
                ),
                ...rows,
              ],
            ),
          ),
        ],
      );

  Color get _backgroundColour => Color.fromARGB(
        255,
        (_background[0].clamp(0.0, 1.0) * 255).round(),
        (_background[1].clamp(0.0, 1.0) * 255).round(),
        (_background[2].clamp(0.0, 1.0) * 255).round(),
      );

  String get _backgroundHex {
    String two(int i) => (_background[i].clamp(0.0, 1.0) * 255)
        .round()
        .toRadixString(16)
        .padLeft(2, '0');
    return '#${two(0)}${two(1)}${two(2)}';
  }

  /// The colour is chosen here and written when the dialog is, like every other
  /// field: nothing this dialog asks about lands before its button is pressed.
  void _pickBackground(Offset position) {
    final t = ThemeScope.of(context).theme;
    showColourPicker(
      context: context,
      position: position,
      initial: PickedColour.of(_backgroundColour),
      presets: t.backgroundPresets,
      onCommit: (picked) => setState(() {
        _background = [
          picked.r.toDouble(),
          picked.g.toDouble(),
          picked.b.toDouble(),
          1.0,
        ];
      }),
    );
  }
}

/// A rate as one number: `60`, `23.976`. Trailing zeros are dropped, so the
/// ordinary rates read as ordinary numbers.
String _formatRate(int num, int den) {
  if (den <= 0) return '$num';
  if (num % den == 0) return '${num ~/ den}';
  final decimal = (num / den).toStringAsFixed(3);
  return decimal
      .replaceFirst(RegExp(r'0+$'), '')
      .replaceFirst(RegExp(r'\.$'), '');
}

/// A typed rate as the exact `(num, den)` pair the engine stores.
///
/// The NTSC family is matched by name rather than derived, because 23.976 is a
/// *rounding* of 24000/1001 and no amount of arithmetic on the rounded number
/// gets the exact rate back. Anything else is read on the thousandths grid and
/// reduced, so 12.5 is 25/2 rather than 12500/1000.
(int, int)? parseRate(String text) {
  final value = parseNumberField(text.trim())?.toDouble();
  if (value == null || value <= 0 || value > 1000000) return null;
  for (final (label, num, den) in _ratePresets) {
    if ((value - double.parse(label)).abs() < 0.0005) return (num, den);
  }
  const den = 1000;
  final num = (value * den).round();
  final g = _gcd(num, den);
  return (num ~/ g, den ~/ g);
}

/// A duration in exact seconds as `HH:MM:SS:FF` timecode at `fpsNum/fpsDen`.
String timecodeOfDuration(BridgeRational seconds, int fpsNum, int fpsDen) {
  final den = seconds.den.toInt();
  final fps = fpsDen == 0 ? 0.0 : fpsNum / fpsDen;
  final secs = den == 0 ? 0.0 : seconds.num.toInt() / den;
  return timecodeOfRate((secs * fps).round(), fpsNum, fpsDen);
}

/// A whole frame count back to exact seconds at `fpsNum/fpsDen` — the pair the
/// document stores (K-180: seconds, never a frame count).
BridgeRational secondsOfFrames(int frames, int fpsNum, int fpsDen) {
  if (fpsNum <= 0) return BridgeRational(num: frames, den: 1);
  final num = frames * (fpsDen <= 0 ? 1 : fpsDen);
  final g = _gcd(num, fpsNum);
  return BridgeRational(
    num: num ~/ (g == 0 ? 1 : g),
    den: fpsNum ~/ (g == 0 ? 1 : g),
  );
}

/// `HH:MM:SS.mmm` for an exact number of seconds.
String formatDurationHms(BridgeRational seconds) {
  final den = seconds.den.toInt();
  final total = den == 0 ? 0 : (seconds.num.toInt() * 1000 / den).round();
  final ms = total % 1000;
  final s = (total ~/ 1000) % 60;
  final m = (total ~/ 60000) % 60;
  final h = total ~/ 3600000;
  String two(int v) => v.toString().padLeft(2, '0');
  return '${two(h)}:${two(m)}:${two(s)}.${ms.toString().padLeft(3, '0')}';
}

/// `HH:MM:SS.mmm` back to exact seconds, or null when it is not a time.
///
/// Forgiving about the separator before the milliseconds (a colon is what other
/// editors show, a full stop is what this dialog prints) and about missing
/// leading fields, so `11.892` and `1:30` both read as what they obviously mean.
BridgeRational? parseDurationHms(String text) {
  final match = RegExp(r'^(?:(\d+):)?(?:(\d+):)?(\d+)(?:[.:](\d{1,3}))?$')
      .firstMatch(text.trim());
  if (match == null) return null;
  int part(int group) => int.tryParse(match.group(group) ?? '') ?? 0;
  // With one leading field it is minutes, with two it is hours then minutes —
  // which is how everybody reads "1:30".
  final hours = match.group(2) == null ? 0 : part(1);
  final minutes = match.group(2) == null ? part(1) : part(2);
  final ms = int.tryParse((match.group(4) ?? '').padRight(3, '0')) ?? 0;
  final total = ((hours * 3600 + minutes * 60 + part(3)) * 1000) + ms;
  final g = _gcd(total, 1000);
  return BridgeRational(
    num: (total ~/ (g == 0 ? 1 : g)),
    den: 1000 ~/ (g == 0 ? 1 : g),
  );
}

/// `40 : 17` for 1920 × 816 — the shape, in its smallest whole numbers.
String aspectRatioLabel(int width, int height) {
  final g = _gcd(width, height);
  if (g == 0) return '';
  return '${width ~/ g} : ${height ~/ g}';
}

int _gcd(int a, int b) {
  a = a.abs();
  b = b.abs();
  while (b != 0) {
    final t = a % b;
    a = b;
    b = t;
  }
  return a;
}
