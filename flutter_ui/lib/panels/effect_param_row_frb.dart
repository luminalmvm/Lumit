// One effect parameter as an editable row — shared by the Effect controls panel
// and the Timeline's fold-out, so a parameter behaves the same wherever it is
// shown.
//
// **What a row is.** The stopwatch and ◄ ◆ ► navigator for the kinds that can
// animate, the parameter's label, and whatever control its kind asks for: a
// scrub-draggable number, a colour swatch, a choice list, a seed, a layer
// picker, or a file name.
//
// **Why the writes go out through callbacks.** A `BridgeEffectInstance` is an
// opaque Rust handle, and the calls that take a whole stack — `setEffects`,
// `renderFrameWithPreview` — take it *by value*: frb moves it and disposes the
// Dart side. A row must therefore never write into the instance it was built
// from and hand that same instance on; it says what it wants written and the
// owner of the stack mints fresh handles for the one call that consumes them.
// Getting this wrong is what stopped effect parameters being draggable at all:
// the first preview tick killed the handles and the rest of the gesture threw.

import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart' show BridgePortType;
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart' show iconSize;
import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/dropper.dart';
import '../state/file_dialogs.dart';
import '../state/preview_throttle.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/angle_dial.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import '../widgets/curve_editor.dart';
import 'fx_section.dart';
import 'graph_editor_frb.dart';
import 'graph_panel.dart' show portColour;
import 'keyframe_controls_frb.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/widgets/autofill.dart';
import 'package:syntax_highlight/syntax_highlight.dart';

/// How wide one value cell is.
const double effectCellWidth = 78;

/// The gap between a value well and whatever rides beside it — the mockup's
/// own 6 (§12A.3).
const double effectRiderGap = 6;

/// What a parameter's declared unit reads as beside its value (§12A.3),
/// or `null` for a number that genuinely has none.
///
/// Read off the *declaration*, never off the parameter's id: `centre_x` is a
/// per cent of the frame on Radial blur and pixels at composition size on a
/// dozen other effects, which is precisely what the id-keyed table this
/// replaced could not tell apart.
String? unitRiderText(BridgeUnit unit) => switch (unit) {
      BridgeUnit.px => l10n.unitSymbolPx,
      BridgeUnit.percent => l10n.unitSymbolPercent,
      BridgeUnit.degrees => l10n.unitSymbolDegrees,
      BridgeUnit.seconds => l10n.unitSymbolSeconds,
      BridgeUnit.frames => l10n.unitSymbolFrames,
      BridgeUnit.raw => null,
    };

/// How the rider is set: plain mono at 10, muted, no tracking (§12A.3's own
/// computed style). It is a fact about the number beside it, not a label
/// naming anything, so it is deliberately not a kicker.
TextStyle unitRiderStyle(LumitTheme t) =>
    t.mono.copyWith(fontSize: 10, color: t.textMuted);

/// `control` with its unit beside it, or `control` alone for a parameter whose
/// number has no unit.
Widget withUnitRider(LumitTheme t, BridgeUnit unit, Widget control) {
  final text = unitRiderText(unit);
  if (text == null) return control;
  return Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      // Flexible so a narrow panel shrinks the well rather than overflowing
      // the row — the rider is three characters and the well is the part with
      // room to give.
      Flexible(child: control),
      const SizedBox(width: effectRiderGap),
      Text(text, style: unitRiderStyle(t)),
    ],
  );
}

/// One parameter: its label, and the control its kind asks for.
///
/// Takes the effect's *id* and this parameter's *value* rather than the opaque
/// instance handle: every read through a handle is a bridge crossing, and the
/// owner already fetched everything in one `getInfo`.
class EffectParamRowFrb extends StatelessWidget {
  final UuidValue effectId;
  final BridgeParamInfo param;

  /// This parameter's current value, from the owner's one `getInfo` read —
  /// staged value during a drag. Null when the instance does not carry the
  /// parameter (a schema newer than the saved document); the row then draws
  /// nothing rather than a misleading zero.
  final BridgeEffectValue? value;
  final CompositionReference comp;
  final int playheadFrame;
  final ValueChanged<int> onSeek;

  /// A typed value, or the release of a drag: commit it as one op.
  final void Function(UuidValue effect, String param, BridgeEffectValue value)
      onWrite;

  /// A drag tick: preview it without committing.
  final void Function(UuidValue effect, String param, BridgeEffectValue value)
      onLive;

  /// When set (the Timeline's fold-out), the control sits inside this fixed
  /// span so it lines up under the render-switch column group (docs/07 §4.3).
  final ValueColumn? valueColumn;

  /// Padding inside the row. The Timeline's fold-out passes zero: its rows are
  /// exactly one lane tall, and padding on top of that clipped the fields
  /// (the Effect controls card has the room, so it keeps its breathing space).
  final EdgeInsets rowPadding;

  /// Lay the row out as the Effect controls panel's two columns — name left,
  /// control left-aligned in the rest — rather than pushing the control to the
  /// row's right edge. Ignored when [valueColumn] is set: the Timeline's rows
  /// answer to the render-switch column group instead.
  final bool twoColumn;

  /// The layer this effect sits on, and every layer in the comp — what a
  /// layer-valued parameter picks from. The owner is offered too:
  /// picking it means "this layer", the effect's own input. Both
  /// ride in from the read model, so the closed picker costs nothing.
  final UuidValue ownerLayerId;
  final List<BridgeLayerEntry> ownerLayers;

  /// Whether this row is editable, per the effect's conditional-enablement
  /// rules (`EnabledWhen` in the schema, `listParamLayout` across the bridge).
  ///
  /// A greyed row still draws its value — you can read what focus distance
  /// *would* be — but takes no gesture, because while Use focus point is ticked
  /// the number decides nothing and offering it to drag would be a lie about
  /// what is in charge.
  final bool enabled;

  /// Clicking the parameter's *name* selects it for the graph editor
  /// (docs/07 §4.3) — the name, not the whole row.
  final VoidCallback? onLabelTap;

  /// The parameter's graph line colour while it is selected.
  final Color? graphColour;

  /// The group this row came out of, drawn before its name on a **flat
  /// sheet**: the dope sheet lists `Glow · Intensity`, where the
  /// fold-out draws `Intensity` under the effect's own heading. Null wherever
  /// the row sits inside that heading.
  final String? nameGroup;

  /// The rest of this effect's parameter values, by id — what a control needs
  /// when its behaviour depends on a *sibling*. The depth-of-field focal point
  /// is the case that asks for it: its dropper reads the layer named by the
  /// effect's own `depth` parameter, and inverts what it reads when
  /// `depth_invert` is set, so it cannot be built from this parameter alone.
  /// Empty is a fair default — the row then simply offers no dropper.
  final Map<String, BridgeEffectValue> siblings;

  /// Parameters that ride beside this row's control on the **same row**, in
  /// drawing order, each with its current value (staged drag included): the
  /// Matte row's Channel choice and Invert switch, the Mix
  /// row's Blend choice. Empty on every other row.
  ///
  /// The matte row is one row everywhere — picker, then what to read from it
  /// and whether to flip it — so it is one widget rather than rows that
  /// happen to be adjacent. A rider keeps its own parameter id, so the key it
  /// draws under and the value it writes are the ones a separate row would
  /// have used; only the layout is shared. A null value draws a switch off
  /// and a choice at its first option, the reading an absent parameter gets
  /// everywhere else (a project saved before the rider simply has none).
  final List<(BridgeParamInfo, BridgeEffectValue?)> riders;

  /// An **Action** row's press: a button, so an event rather than a
  /// value. Null leaves the button drawn but dead, which is what a row shown
  /// somewhere that cannot fire one (the Timeline's twirl-down) should do.
  final void Function(UuidValue effect, String param)? onAction;

  /// A **driver** is wired to this parameter in the Graph panel: the
  /// name it draws under, and what its wire carries.
  ///
  /// **The mark sits on the left, where the stopwatch was**. A driven
  /// parameter has no keyframes of its own to step between or key — the wire
  /// decides the value — so the stopwatch and the key navigator are meaningless
  /// on it, and their column is exactly the room the *driven* mark needs. The
  /// value column keeps drawing the number so you can still read what the row
  /// holds, but takes no gesture: a spinner you could drag would be a lie about
  /// what is in charge, the same reasoning `enabled` follows.
  ///
  /// `noStream` is the hazard mark: the box at the other end reads a
  /// points stream and has none wired into it, so what arrives along this wire
  /// is the documented empty-stream answer rather than anything the picture
  /// contains. It is a fact about the *wiring*, not a value — the panel never
  /// asks for a driven number, which is what keeps this off the rebuild path.
  final ({String driver, BridgePortType type, bool noStream})? driven;

  const EffectParamRowFrb({
    super.key,
    required this.effectId,
    required this.param,
    required this.value,
    required this.comp,
    required this.playheadFrame,
    required this.onSeek,
    required this.onWrite,
    required this.onLive,
    required this.ownerLayerId,
    required this.ownerLayers,
    this.valueColumn,
    this.rowPadding = const EdgeInsets.symmetric(vertical: 2),
    this.onLabelTap,
    this.graphColour,
    this.twoColumn = false,
    this.siblings = const {},
    this.enabled = true,
    this.riders = const [],
    this.onAction,
    this.driven,
    this.nameGroup,
  });

  @override
  Widget build(BuildContext context) {
    // The live playhead: an animated field shows (and edits) the value under
    // it, so it must follow a scrub.
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    // **A still row does not.** With every channel static there is no curve to
    // sample and no key to sit on, so the row draws the same at every frame —
    // and `scalarWithValueAt` ignores the frame when it writes one. Most rows in
    // most stacks are still, so this is most of what a scrub used to cost here.
    if (_still) return _build(context, playhead.value);
    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) => _build(context, frame),
    );
  }

  /// True when nothing this row draws can change as the playhead moves: every
  /// channel static, or a kind that carries no curve at all — a choice, a
  /// switch, a file. Those have no stopwatch and nothing to sample.
  bool get _still {
    final scalars = _animatableScalarsOf(value);
    return scalars == null || scalars.every((s) => s is BridgeScalar_Static);
  }

  Widget _build(BuildContext context, int frame) {
    final t = ThemeScope.of(context).theme;
    final id = effectId;
    final scalars = _animatableScalarsOf(value);
    // Only the interpolatable kinds animate; a choice or a file has nothing to
    // blend between, so those rows carry no stopwatch at all. **A driven row
    // has none either**: the wire decides its value, so there is no key
    // to add and no neighbour to step to, and the mark takes the column.
    final keyframes = driven != null
        ? _drivenMark(t, id)
        : scalars == null
            ? null
            : KeyframeControlsFrb(
                // One channel for a number, four for a colour — and one stopwatch
                // over them either way.
                scalars: scalars,
                comp: comp,
                playheadFrame: playheadFrame,
                onSeek: onSeek,
                rowKey: '$id-${param.id}',
                // The panel's fixed columns; the Timeline's fold-out takes
                // the other branch and keeps its narrow gutter.
                fixedColumns: twoColumn && valueColumn == null,
                onWrite: (next) => _set(next.length == 4
                    ? BridgeEffectValue.colour(BridgeColour(
                        r: next[0], g: next[1], b: next[2], a: next[3]))
                    : BridgeEffectValue.float(next.single)),
              );

    // The driven mark is not greyed with the rest of the row: it is the one
    // thing on it that is *in* charge, and it carries the driver's name in a
    // tooltip an [IgnorePointer] would swallow.
    final keyframeSlot = keyframes == null
        ? null
        : (driven != null ? keyframes : _greyed(keyframes));

    // The name is the row's handle for the graph editor, so it is built once
    // and drawn by whichever layout the row takes. A greyed row's name is
    // muted with it: half a row going quiet reads as a rendering fault rather
    // than as "this control is not the one in charge".
    final labelStyle = _off
        ? t.body.copyWith(color: t.textDisabled)
        : (graphColour == null ? t.body : t.body.copyWith(color: graphColour));
    final labelText = Text(
      engineLabel(param.label),
      style: labelStyle,
      overflow: TextOverflow.ellipsis,
    );
    final label = GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onLabelTap,
      child: nameGroup == null
          ? labelText
          : Row(children: [
              ...flatGroupPrefix(t, nameGroup),
              Flexible(child: labelText),
            ]),
    );

    // The unit goes on the control itself, inside the riders: `100 %` then the
    // Blend dropdown, never `100` then Blend then `%`.
    final control = _greyed(_withRiders(t, id,
        withUnitRider(t, param.unit, _control(context, t, id, value, frame))));

    // **An Action is a button, and a button says its own name**. Drawn
    // in the value column with the name column left empty, rather than as a
    // label beside a button repeating it: the row is one statement, and the
    // house style is that a control carrying words does not need them twice.
    if (param.kind is BridgeParamKind_Action) {
      return fxTwoColumnRow(
        context: context,
        name: const SizedBox.shrink(),
        keyframeControls: null,
        control: control,
      );
    }

    if (twoColumn && valueColumn == null) {
      // No padding of its own: the Effect controls panel gives every row the
      // same fixed height ([fxRowHeight]), and padding on top of that would
      // eat into the room the controls sit in.
      return fxTwoColumnRow(
        context: context,
        name: label,
        keyframeControls: keyframeSlot,
        control: control,
      );
    }

    return Padding(
      padding: rowPadding,
      child: Row(
        children: [
          if (keyframeSlot != null) keyframeSlot,
          const SizedBox(width: 4),
          Expanded(child: label),
          if (valueColumn case final col?) ...[
            SizedBox(
              width: col.width,
              child: Align(
                alignment: Alignment.centerLeft,
                child: control,
              ),
            ),
            SizedBox(width: col.rightInset),
          ] else ...[
            const SizedBox(width: 10),
            control,
          ],
        ],
      ),
    );
  }

  /// The scalar behind this row when the kind is one that can animate, else
  /// null. Float, Int and Slider are the single-scalar animatable kinds the
  /// schema declares — all three store a float, differing only in the control
  /// drawn over it; a colour animates per channel, which the swatch has no
  /// room to key.
  /// Draw `child` as a quiet row when another parameter has taken it over.
  ///
  /// Deaf but **not** faded (docs/15 §5): [IgnorePointer] is what makes it
  /// honest, since a control that still answers a drag while looking disabled
  /// is worse than one that never changed. The dimming is gone — the label
  /// already carries `text_disabled`, and the value stays fully legible, so you
  /// can read what Focus distance *would* be. Being off is not being gone.
  /// **A driven row is off for the same reason**: the wire decides the
  /// value, so the field draws what the row holds and answers nothing.
  bool get _off => !enabled || driven != null;

  Widget _greyed(Widget child) => _off ? IgnorePointer(child: child) : child;

  /// What a driven row shows **in the stopwatch's column**: a hollow
  /// ring in the wire's own colour with the word *driven* beside it, and the
  /// driver's name in its tooltip.
  ///
  /// The ring is hollow because the value is not held here — it arrives along
  /// a wire, and a filled mark is what a socket uses to say a wire has landed.
  /// The name is a tooltip rather than a well because the column is 72 pixels
  /// wide and a box's name is whatever the user called it; the mark itself has
  /// to fit on a Timeline lane as well as in a panel.
  Widget _drivenMark(LumitTheme t, UuidValue id) {
    final it = driven!;
    // The wire is honest about its type until it is dry: a source with no
    // stream behind it draws in the warning family, because what this row is
    // following is a no-op and not a measurement.
    final colour = it.noStream ? t.warning : portColour(t, it.type);
    return LumitTooltip(
      message: l10n.tipDrivenBy(it.driver),
      // Bounded to the column it stands in, so the word shortens rather than
      // running under the label — and so the mark is safe in the Timeline's
      // gutter, which hands its children unbounded width.
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: fxKeyColumnWidth),
        child: Row(
          key: ValueKey<String>('fx-driven-$id-${param.id}'),
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 10,
              height: 10,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                border: Border.all(color: colour),
              ),
            ),
            const SizedBox(width: 4),
            // The word is the state. A wire whose source has no stream is not
            // *driven* by anything — it is following a documented no-op — so
            // the row says which of the two it is in the slot the word already
            // had, rather than growing a second mark there is no room for.
            Flexible(
              child: Text(
                it.noStream ? l10n.graphNoStream : l10n.graphDriven,
                key: it.noStream
                    ? ValueKey<String>('fx-no-stream-$id-${param.id}')
                    : null,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: t.kicker.copyWith(letterSpacing: 0.54, color: colour),
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// The animations this row's stopwatch covers, or null for a kind with
  /// nothing to interpolate.
  ///
  /// **A colour is four of them under one stopwatch**. The engine has
  /// always stored a colour as four independent properties and sampled them
  /// one by one (`EffectParams::colour_at`, "channels animate independently"),
  /// and [KeyframeControlsFrb] has always taken a list — Position's x and y are
  /// the same shape. This helper answered with a single scalar, so a colour row
  /// asked for no controls at all and the stopwatch simply was not drawn: the
  /// one thing missing between an engine that could key colours and a panel
  /// that could not.
  List<BridgeScalar>? _animatableScalarsOf(BridgeEffectValue? value) {
    // Int is a Float value with integer display (docs/08 §1.2), and a Slider is
    // a Float value inside a closed range, so both animate exactly like
    // Float — the kind is the control, not the storage.
    if (param.kind is BridgeParamKind_Float ||
        param.kind is BridgeParamKind_Int ||
        param.kind is BridgeParamKind_Slider ||
        param.kind is BridgeParamKind_Angle) {
      return switch (value) {
        BridgeEffectValue_Float(:final field0) => [field0],
        _ => null,
      };
    }
    if (param.kind is BridgeParamKind_Colour) {
      return switch (value) {
        BridgeEffectValue_Colour(:final field0) => [
            field0.r,
            field0.g,
            field0.b,
            field0.a,
          ],
        _ => null,
      };
    }
    return null;
  }

  /// Write this parameter. The value goes up to the panel rather than being
  /// written into an instance here: the owner of the stack mints fresh handles
  /// for the one call that consumes them.
  void _set(BridgeEffectValue value) => onWrite(effectId, param.id, value);

  /// The same value, previewed rather than committed — one drag tick.
  void _setLive(BridgeEffectValue value) => onLive(effectId, param.id, value);

  Widget _control(BuildContext context, LumitTheme t, UuidValue id,
      BridgeEffectValue? value, int frame) {
    // Checked before the missing-value guard, because an Action row genuinely
    // has none: it carries no `EffectValue` anywhere in the model, which is
    // what makes it a press rather than a write.
    if (param.kind is BridgeParamKind_Action) {
      return HouseButton(
        key: ValueKey<String>('fx-action-$id-${param.id}'),
        small: true,
        onPressed: onAction == null ? null : () => onAction!(id, param.id),
        child: Text(engineLabel(param.label), style: t.body),
      );
    }
    if (value == null) return Text('—', style: t.small);

    switch (param.kind) {
      case BridgeParamKind_Float(
          :final sliderMin,
          :final sliderMax,
          :final hardMin,
          :final hardMax
        ):
        if (value case BridgeEffectValue_Float(:final field0)) {
          // A driven parameter is a line of code, not a number to drag, so it
          // gets the editor row instead of the value field.
          if (field0 case BridgeScalar_Expression expr) {
            return EffectParamRowExpression(
              key: ValueKey<String>(
                  'fx-expression-$id-${param.id}-${param.hashCode}'),
              value: expr,
              comp: comp,
              frame: frame,
              layer: currentLayer,
              set: _set,
              setLive: _setLive,
            );
          }

          final field = _scalarField(
            context,
            scalar: field0,
            setExpression: () {
              // Seed the expression with the value showing now, so turning one
              // on does not move the picture until it is edited.
              final sampled = sampleScalarWithContext(
                  scalar: field0,
                  time: timeOfFrame(comp, frame),
                  layer: currentLayer);
              _set(BridgeEffectValue.float(
                  BridgeScalar.expression(sampled.toString())));
            },
            frame: frame,
            sliderMin: sliderMin,
            sliderMax: sliderMax,
            hardMin: hardMin,
            hardMax: hardMax,
            keyName: '$id-${param.id}',
            write: (s) => _set(BridgeEffectValue.float(s)),
          );
          // A number picked off the picture rather than typed: the focal point
          // of a depth-of-field, read straight off its own depth pass.
          final depth =
              _depthDropper(context, id, field0, frame, hardMin, hardMax);
          if (depth == null) return field;
          return Row(
            mainAxisSize: MainAxisSize.min,
            children: [field, const SizedBox(width: 4), depth],
          );
        }
        return Text('—', style: t.small);

      case BridgeParamKind_Int(
          :final sliderMin,
          :final sliderMax,
          :final hardMin,
          :final hardMax
        ):
        if (value case BridgeEffectValue_Float(:final field0)) {
          return _scalarField(
            context,
            scalar: field0,
            frame: frame,
            sliderMin: sliderMin.toDouble(),
            sliderMax: sliderMax.toDouble(),
            hardMin: hardMin?.toDouble(),
            hardMax: hardMax?.toDouble(),
            keyName: '$id-${param.id}',
            integer: true,
            write: (s) => _set(BridgeEffectValue.float(s)),
          );
        }
        return Text('—', style: t.small);

      case BridgeParamKind_Slider(:final min, :final max):
        if (value case BridgeEffectValue_Float(:final field0)) {
          return _sliderControl(
            context,
            scalar: field0,
            frame: frame,
            min: min,
            max: max,
            keyName: '$id-${param.id}',
          );
        }
        return Text('—', style: t.small);

      // Answered above, before the missing-value guard — a button has no
      // value and so never reaches here. Spelled out because the switch is
      // exhaustive over the kinds, which is what makes a kind added to the
      // engine a compile error rather than a blank row.
      case BridgeParamKind_Action():
        return const SizedBox.shrink();
      case BridgeParamKind_Curve():
        if (value case BridgeEffectValue_Curve(:final field0)) {
          // The lone-curve case. Curves' five channels fold into one tabbed
          // editor before they ever reach here (the panel's `_paramRows`),
          // exactly as an `_x`/`_y` pair folds into one point row; a schema
          // declaring a single curve gets the same plot without the tabs.
          return CurveEditor(
            key: ValueKey<String>('fx-curve-$id-${param.id}'),
            points: curvePointsOf(field0),
            onLive: (p) => _setLive(curveValue(p)),
            onCommit: (p) => _set(curveValue(p)),
          );
        }
        return Text('—', style: t.small);

      case BridgeParamKind_Angle(:final dialStep):
        if (value case BridgeEffectValue_Float(:final field0)) {
          return _angleControl(
            context,
            scalar: field0,
            frame: frame,
            step: dialStep,
            keyName: '$id-${param.id}',
          );
        }
        return Text('—', style: t.small);

      case BridgeParamKind_Colour(:final min, :final max):
        if (value case BridgeEffectValue_Colour(:final field0)) {
          return _colourSwatch(context, id, field0, min, max, frame);
        }
        return Text('—', style: t.small);

      case BridgeParamKind_Bool():
        if (value case BridgeEffectValue_Bool(:final field0)) {
          return SizedBox(
            width: effectCellWidth,
            child: Align(
              alignment: Alignment.centerLeft,
              child: HouseCheckbox(
                key: ValueKey<String>('fx-bool-$id-${param.id}'),
                value: field0,
                onChanged: (on) => _set(BridgeEffectValue.bool(on)),
              ),
            ),
          );
        }
        return Text('—', style: t.small);

      case BridgeParamKind_Choice(:final options):
        if (value case BridgeEffectValue_Choice(:final field0)) {
          final index = field0 < options.length ? field0.toInt() : 0;
          return SizedBox(
            width: effectCellWidth + 40,
            // A long list gets the searchable, lazily-built picker: a
            // plain dropdown builds every row eagerly, and at 1299 options
            // (the lens library) that took the app down in layout.
            // No shipped list is that long since the lists were curated, but
            // the guard stays for the next one.
            child: options.length >= searchableOptionThreshold
                ? BareSearchDropdown(
                    key: ValueKey<String>('fx-choice-$id-${param.id}'),
                    value: index,
                    options: options,
                    // "Maker · Model" labels group by their maker.
                    group: (label) {
                      final i = label.indexOf(' · ');
                      return i > 0 ? label.substring(0, i) : null;
                    },
                    hint:
                        l10n.searchFor(engineLabel(param.label).toLowerCase()),
                    onChanged: (i) => _set(BridgeEffectValue.choice(i)),
                  )
                : BareDropdown<int>(
                    key: ValueKey<String>('fx-choice-$id-${param.id}'),
                    value: index,
                    options: [for (var i = 0; i < options.length; i++) i],
                    label: (i) => engineLabel(options[i]),
                    onChanged: (i) => _set(BridgeEffectValue.choice(i)),
                  ),
          );
        }
        return Text('—', style: t.small);

      case BridgeParamKind_Seed():
        if (value case BridgeEffectValue_Seed(:final field0)) {
          return SizedBox(
            width: effectCellWidth,
            child: DragValueField(
              key: ValueKey<String>('fx-seed-$id-${param.id}'),
              value: field0,
              min: 0,
              max: 0xFFFFFFFF,
              speed: 1,
              onChanged: (v) =>
                  _set(BridgeEffectValue.seed(v.toInt().clamp(0, 0xFFFFFFFF))),
            ),
          );
        }
        return Text('—', style: t.small);

      case BridgeParamKind_Layer():
        if (value case BridgeEffectValue_Layer(:final field0)) {
          return _layerPicker(context, id, field0);
        }
        return Text('—', style: t.small);

      case BridgeParamKind_MaskPath():
        if (value case BridgeEffectValue_MaskPath(:final field0)) {
          return _maskPicker(context, id, field0);
        }
        return Text('—', style: t.small);

      case BridgeParamKind_File(:final filter, :final filterName):
        if (value case BridgeEffectValue_File(:final field0)) {
          final paths = field0.paths;
          // The row is the picker: click to choose a file through
          // the schema's own filter, and an unset row says so. It was a
          // bare label for a while — the parameter existed and nothing
          // in the panel could set it, which the owner found within the
          // hour. A set row grows a clear button, because a File value's
          // neutral state is "none" and there was no way back to it.
          return SizedBox(
            width: effectCellWidth + 60,
            child: Row(
              children: [
                Flexible(
                  child: LumitTooltip(
                    message:
                        paths.isEmpty ? l10n.chooseA(filterName) : paths.first,
                    child: HouseButton(
                      key: ValueKey<String>('fx-file-$id-${param.id}'),
                      onPressed: () async {
                        final path =
                            await pickEffectInputFile(filter, filterName);
                        if (path == null) return;
                        _set(BridgeEffectValue.file(BridgeFileParam(
                          paths: [path],
                          index: const BridgeScalar.static_(0),
                        )));
                      },
                      child: Text(
                        paths.isEmpty
                            ? l10n.chooseEllipsis
                            : _basename(paths.first),
                        style: t.small,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                  ),
                ),
                if (paths.isNotEmpty)
                  LumitTooltip(
                    message: l10n.clear,
                    child: HouseButton(
                      key: ValueKey<String>('fx-file-clear-$id-${param.id}'),
                      onPressed: () => _set(BridgeEffectValue.file(
                          const BridgeFileParam(
                              paths: [], index: BridgeScalar.static_(0)))),
                      child: Text('×', style: t.small),
                    ),
                  ),
              ],
            ),
          );
        }
        return Text('—', style: t.small);
    }
  }

  /// The layer this row's effect sits on.
  ///
  /// An expression is evaluated about a particular layer — `time`, `cut_in`,
  /// `layer()` all mean something only relative to one — so the row has to say
  /// which, and the effect stack it was drawn from knows.
  LayerReference get currentLayer => ownerLayers
      .firstWhere((i) => i.layer.internallayerId == ownerLayerId)
      .layer;

  /// A number field for a scalar. A static value drags with live preview; an
  /// animated one shows the value under the playhead and a change writes it
  /// into the key sitting there — or plants one — never flattening the curve
  /// (docs/07 §4.3).
  Widget _scalarField(
    BuildContext context, {
    required BridgeScalar scalar,
    required int frame,
    required double sliderMin,
    required double sliderMax,
    required double? hardMin,
    required double? hardMax,
    required String keyName,
    required void Function(BridgeScalar) write,
    void Function()? setExpression,
    bool integer = false,
  }) {
    // The drag paces itself by the declared slider span, so a 0–1 parameter and
    // a 0–500 one both feel the same under the pointer. An integer row steps
    // whole numbers and never shows decimals (docs/08 §1.2's Int kind).
    final span = (sliderMax - sliderMin).abs();
    final speed = integer
        ? (span <= 40 ? 0.08 : span / 400)
        : (span <= 0 ? 0.5 : span / 200);
    double snap(num v) => integer ? v.roundToDouble() : v.toDouble();

    if (scalar case BridgeScalar_Keyframed()) {
      final sampled =
          sampledScalar(scalar, timeOfFrame(comp, frame));
      return SizedBox(
        width: effectCellWidth,
        child: KeyedValueField(
          fieldKey: ValueKey<String>('fx-float-$keyName'),
          value: sampled,
          min: hardMin ?? -1000000,
          max: hardMax ?? 1000000,
          speed: speed,
          onCommit: (v) {
            rowValueDrag.value = null;
            write(scalarWithValueAt(scalar, snap(v), comp, frame));
          },
          // A key under the playhead the moment the drag starts: it
          // holds the value already there, so nothing moves — and the drag
          // then has a key to carry in the graph.
          onStart: () {
            if (scalar.field0
                .any((k) => comp.frameAtTime(time: k.time) == frame)) {
              return;
            }
            write(scalarWithValueAt(scalar, sampled, comp, frame));
          },
          // Each tick: the picture through the staged effect stack, and the
          // curve through the published drag — the same pair a
          // transform row's drag feeds.
          onLive: (v) {
            rowValueDrag.value = RowValueDrag(
              layer: ownerLayerId.toString(),
              effectId: effectId.toString(),
              paramId: param.id,
              frame: frame,
              value: snap(v),
            );
            _setLive(BridgeEffectValue.float(
                scalarWithValueAt(scalar, snap(v), comp, frame)));
          },
        ),
      );
    }

    return SizedBox(
      width: effectCellWidth,
      child: DragValueField(
        key: ValueKey<String>('fx-float-$keyName'),
        value: (scalar as BridgeScalar_Static).field0,
        // Typing may leave the slider's travel; only the hard bounds clamp
        // (docs/08 §1.2).
        min: hardMin ?? -1000000,
        max: hardMax ?? 1000000,
        speed: speed,
        decimals: integer ? 0 : 2,
        onChanged: (v) => write(BridgeScalar.static_(snap(v))),
        onChangeLive: (v) =>
            _setLive(BridgeEffectValue.float(BridgeScalar.static_(snap(v)))),
        onChangeEnd: (v) => write(BridgeScalar.static_(snap(v))),
        setExpression: setExpression,
      ),
    );
  }

  /// The dropper beside a depth-of-field focal point, or null when this row is
  /// not one (docs/07 §6.1, docs/08 §3.22).
  ///
  /// It is offered on the `focus` parameter of an effect that carries a `depth`
  /// layer, and reads that layer **alone** — a depth pass is nearly always
  /// hidden, so what the composite shows at that pixel is not the number the
  /// effect uses. `depth_invert` is applied here, at the pick, so the value
  /// written is the one the parameter means; the caption and the committed
  /// number can never disagree.
  ///
  /// **A pick is a typed value, not a reset**. It takes `scalar` and
  /// writes through `scalarWithValueAt` for the same reason the colour swatch
  /// beside it does: a keyed Focus takes a key at the playhead, and a static one
  /// stays static. Writing a bare static here flattened the curve — every
  /// keyframe on the focal distance gone, for the gesture that was meant to set
  /// one of them.
  Widget? _depthDropper(BuildContext context, UuidValue id, BridgeScalar scalar,
      int frame, double? hardMin, double? hardMax) {
    if (param.id != 'focus') return null;
    if (siblings['depth'] case BridgeEffectValue_Layer(:final field0)
        when field0 != null) {
      final entry = ownerLayers
          .where((l) => l.layer.internallayerId == field0)
          .firstOrNull;
      if (entry == null) return null;
      final invert = switch (siblings['depth_invert']) {
        BridgeEffectValue_Bool(:final field0) => field0,
        _ => false,
      };
      // The value the pick would write, wherever the pointer is now: shared by
      // the previews and the one commit, so what is watched and what lands can
      // never be two different numbers.
      BridgeEffectValue focus(DropperSample sample) {
        final d = invert ? 1 - sample.depth : sample.depth;
        final low = hardMin ?? 0, high = hardMax ?? 1;
        return BridgeEffectValue.float(
            scalarWithValueAt(scalar, d.clamp(low, high), comp, frame));
      }

      final before = value;
      return _DropperButton(
        id: 'fx-$id-${param.id}',
        tip: l10n.tipPickFocalPoint,
        arm: (ui) => ui.armDropper(DropperArm(
          id: 'fx-$id-${param.id}',
          reads: DropperReads.depth,
          label: engineLabel(param.label),
          sampleLayer: entry.layer,
          sampleLayerName: entry.info.name,
          onPreview: (sample) => _setLive(focus(sample)),
          onPick: (sample) => _set(focus(sample)),
          // Abandoned: put the focus the row had back through the same preview
          // path. Nothing was committed, so this is the whole of the undoing.
          onRevert: before == null ? null : () => _setLive(before),
        )),
      );
    }
    return null;
  }

  /// A closed range: the track and thumb, with the number beside it
  /// (docs/08 §1.2).
  ///
  /// The same shape the angle row has, and for the same reason — the track is a
  /// second grip on one value, not a second control, so it sits beside the
  /// number rather than under it. `min`/`max` are the travel *and* the hard
  /// bound, which is what makes the range closed: there is no picture either
  /// side of a wipe's Completion, so nothing may be typed past its ends either.
  Widget _sliderControl(
    BuildContext context, {
    required BridgeScalar scalar,
    required int frame,
    required double min,
    required double max,
    required String keyName,
  }) {
    // A driven parameter is a line of code, not a number to drag — the same
    // answer the Float row gives.
    if (scalar case BridgeScalar_Expression expr) {
      return EffectParamRowExpression(
        key: ValueKey<String>('fx-expression-$keyName-${param.hashCode}'),
        value: expr,
        comp: comp,
        frame: frame,
        layer: currentLayer,
        set: _set,
        setLive: _setLive,
      );
    }

    final animated = scalar is! BridgeScalar_Static;
    final shown = animated
        ? sampledScalar(scalar, timeOfFrame(comp, frame))
        : scalar.field0;

    void write(double v) {
      final clamped = v.clamp(min, max).toDouble();
      _set(BridgeEffectValue.float(animated
          ? scalarWithValueAt(scalar, clamped, comp, frame)
          : BridgeScalar.static_(clamped)));
    }

    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        _scalarField(
          context,
          scalar: scalar,
          frame: frame,
          sliderMin: min,
          sliderMax: max,
          hardMin: min,
          hardMax: max,
          keyName: keyName,
          write: (s) => _set(BridgeEffectValue.float(s)),
          // The same seed the Float row offers: a closed range keeps every
          // float affordance, and turning an expression on must not
          // move the picture until it is edited.
          setExpression: () {
            final sampled = sampleScalarWithContext(
                scalar: scalar,
                time: timeOfFrame(comp, frame),
                layer: currentLayer);
            _set(BridgeEffectValue.float(
                BridgeScalar.expression(sampled.toString())));
          },
        ),
        const SizedBox(width: 6),
        HouseSlider(
          key: ValueKey<String>('fx-slider-$keyName'),
          value: shown.clamp(min, max).toDouble(),
          min: min,
          max: max,
          // The number is already beside it, and a second copy of it would
          // cost the value column room it has not got.
          showValue: false,
          width: 78,
          // A drag previews and commits once on release, exactly as the number
          // field does, so the two grips behave the same.
          commitOnRelease: true,
          onChangeLive: animated
              ? null
              : (v) => _setLive(BridgeEffectValue.float(
                  BridgeScalar.static_(v.clamp(min, max).toDouble()))),
          onChanged: write,
        ),
      ],
    );
  }

  /// A number in degrees with the dial under it (docs/07 §6).
  ///
  /// The dial drags live and commits on release, exactly as the number does, so
  /// the two are interchangeable. It is unbounded in both: an angle animates
  /// through full turns rather than wrapping, and a keyframe pair that wrapped
  /// would spin backwards through the whole circle on the way to the next key.
  Widget _angleControl(
    BuildContext context, {
    required BridgeScalar scalar,
    required int frame,
    required double step,
    required String keyName,
  }) {
    final animated = scalar is! BridgeScalar_Static;
    final shown = animated
        ? sampledScalar(scalar, timeOfFrame(comp, frame))
        : scalar.field0;

    void write(double v) {
      // On a curve the edit lands in the key under the playhead, or plants one
      // — never flattening what is already there.
      final next = animated
          ? scalarWithValueAt(scalar, v, comp, frame)
          : BridgeScalar.static_(v);
      _set(BridgeEffectValue.float(next));
    }

    // Turns, degrees, dial — one row. The dial is a second grip on the same
    // value, not a second control, so it sits beside the numbers rather than
    // under them: a two-storey row is taller than every other row in the panel
    // and reads as two settings.
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        TurnsAndDegreesField(
          keyName: keyName,
          degrees: shown,
          enabled: !_off,
          onChanged: animated
              ? null
              : (v) =>
                  _setLive(BridgeEffectValue.float(BridgeScalar.static_(v))),
          onCommit: write,
        ),
        const SizedBox(width: 6),
        AngleDial(
          key: ValueKey<String>('fx-dial-$keyName'),
          // Row height, not the standalone 34: it is a grip beside a number.
          size: fxRowHeight(ThemeScope.of(context).theme),
          degrees: shown,
          step: step,
          enabled: !_off,
          // A dial drag is a drag like any other: preview each tick, commit
          // the release. On a curve there is no live preview, for the same
          // reason the numbers have none — the value being previewed is not
          // the one that will be stored.
          onChanged: (v) => animated
              ? null
              : _setLive(BridgeEffectValue.float(BridgeScalar.static_(v))),
          onChangeEnd: write,
        ),
      ],
    );
  }

  /// A colour swatch. The four channels animate independently in the model, so a
  /// swatch edit writes all four statics at once; an animated channel is left
  /// alone for the same reason a scalar is.
  Widget _colourSwatch(BuildContext context, UuidValue id, BridgeColour colour,
      double min, double max, int frame) {
    // **The channel as it reads under the playhead**, keyed or not.
    // The swatch used to say the word `animated` and stand down, the way a
    // number field never did — so a colour with keys on it could be looked at
    // and not changed, which is half of "keyframe a colour" missing.
    double chan(BridgeScalar s) =>
        sampledScalar(s, timeOfFrame(comp, frame));
    final t = ThemeScope.of(context).theme;

    int byte(double f) => (f.clamp(0.0, 1.0) * 255).round();
    final shown = documentColour(
        byte(chan(colour.r)), byte(chan(colour.g)), byte(chan(colour.b)), 255);

    // The value written for a picked colour: the three channels clamped to the
    // parameter's declared range, with alpha left alone.
    //
    // **A keyed channel takes a key at the playhead** rather than being
    // flattened back to a static — `scalarWithValueAt` is the same call an
    // animated number's field makes for the same reason, and it is what turns
    // "move the playhead and change the colour" into a second key rather than
    // into the loss of the first.
    BridgeEffectValue valueOf(PickedColour picked) {
      double clamp(double v) => v < min ? min : (v > max ? max : v);
      BridgeScalar at(BridgeScalar was, double v) =>
          scalarWithValueAt(was, clamp(v), comp, frame);
      return BridgeEffectValue.colour(BridgeColour(
        r: at(colour.r, picked.r),
        g: at(colour.g, picked.g),
        b: at(colour.b, picked.b),
        a: colour.a,
      ));
    }

    return SizedBox(
      width: effectCellWidth + 22,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          GestureDetector(
            key: ValueKey<String>('fx-colour-$id-${param.id}'),
            behavior: HitTestBehavior.opaque,
            onTap: () async {
              final box = context.findRenderObject();
              if (box is! RenderBox) return;
              await showColourPicker(
                context: context,
                position: box.localToGlobal(Offset(0, box.size.height + 4)),
                initial: PickedColour(
                    chan(colour.r), chan(colour.g), chan(colour.b)),
                // An effect colour is scene-linear in a float working depth
                // (fp16 today, docs/06 §3.1): 0–1 is black to white, and the
                // parameter's own range says how far past that it may go — an
                // HDR tint really does sit above 1, and a 0–255 dial could not
                // reach it. When the project depth switch lands (docs/06 §3.1),
                // an 8 bpc project is what passes `bytes` here.
                scale: ColourScale.unit,
                min: min,
                max: max,
                // Live all the way through: a drag inside the picker previews
                // on the picture, and each settled change is one undoable edit
                // — the same shape as dragging the number beside it.
                onPreview: (picked) => _setLive(valueOf(picked)),
                onCommit: (picked) => _set(valueOf(picked)),
              );
            },
            child: MouseRegion(
              cursor: SystemMouseCursors.click,
              child: Container(
                width: 28,
                height: 18,
                decoration: BoxDecoration(
                  color: shown,
                  borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                  border: Border.all(color: t.hairlineStrong),
                ),
              ),
            ),
          ),
          const SizedBox(width: 6),
          // The dropper: lift this colour off the picture instead of choosing
          // it (docs/07 §6.1). **Dragging sweeps it**: the parameter
          // takes the colour under the pointer as it travels, previewed the
          // whole way, and the release is the one edit.
          _DropperButton(
            id: 'fx-$id-${param.id}',
            tip: l10n.tipSampleFromViewer,
            arm: (ui) => ui.armDropper(DropperArm(
              id: 'fx-$id-${param.id}',
              reads: DropperReads.colour,
              label: engineLabel(param.label),
              // The very value the picker writes, so a colour lifted off the
              // picture and a colour chosen in the dialogue cannot be clamped
              // or shaped differently. The sample is already scene-linear,
              // exactly as the parameter stores it.
              onPreview: (sample) =>
                  _setLive(valueOf(PickedColour(sample.r, sample.g, sample.b))),
              onPick: (sample) =>
                  _set(valueOf(PickedColour(sample.r, sample.g, sample.b))),
              // Abandoned: the colour the swatch had, back through the same
              // preview path. Nothing was committed, so that is all of it.
              onRevert: () => _setLive(BridgeEffectValue.colour(colour)),
            )),
          ),
        ],
      ),
    );
  }

  /// A picker over the comp's other layers, with None. An unset or dangling
  /// reference is a labelled no-op engine-side, never a fault, so None is a
  /// first-class choice rather than an error state.
  ///
  /// **On a sound row the empty entry means something**. A parameter
  /// named `audio` is measuring sound, not sampling a picture, and unset is
  /// the composition's own mix — so it reads *This comp* rather than None, and
  /// the list offers every layer instead of only the ones that draw. A music
  /// clip is audio-only: the picture filter left it out of the one picker in
  /// the catalogue that exists to point at it.
  ///
  /// **Lazy, and it has to be**: the options are built when the menu
  /// opens, so they can name every layer and ask which of them has a picture
  /// without either crossing the bridge on a rebuild or probing a
  /// container with FFmpeg while drawing a row.
  Widget _layerPicker(BuildContext context, UuidValue id, UuidValue? current) {
    final chosen = current?.toString();
    final sound = param.id == 'audio';
    final empty = sound ? l10n.fxAudioThisComp : l10n.none;
    // The layer the effect is on says so, so "everything below" is readable
    // on an adjustment layer rather than an unexplained self-reference.
    String named(String name, UuidValue layerId) =>
        layerId == ownerLayerId ? l10n.thisLayerSuffix(name) : name;
    return SizedBox(
      width: effectCellWidth + 40,
      child: BareLazyDropdown<UuidValue?>(
        key: ValueKey<String>('fx-layer-$id-${param.id}'),
        // Named from the read model when it can be, so the closed button
        // costs nothing; a reference to a layer since deleted says so.
        label: chosen == null
            ? empty
            : (ownerLayers
                    .where((l) => l.layer.internallayerId == current)
                    .map((l) => named(l.info.name, l.layer.internallayerId))
                    .firstOrNull ??
                l10n.missingLayer),
        options: () => [
          (null, empty),
          // **Numbered as the composition numbers them** (item 6.13): the
          // entry reads "3. Sky", the layer's own place in the stack, so a
          // list of layers that share a name is still a list of different
          // layers. The number is the position, not part of the name — so it
          // counts every layer, including the ones this picker does not
          // offer, and it is data rather than a phrase (no arb entry).
          for (var i = 0; i < ownerLayers.length; i++)
            // A layer-valued parameter samples a *picture*, so a layer with
            // none (a camera, an audio-only clip) is not offered.
            //
            // The layer the effect is ON is always offered, picture or not:
            // picking it does not re-render that layer, it reads
            // the effect's own input at its point in the stack. That is the
            // whole point on an **adjustment layer** — which has no picture
            // of its own, and whose input is the composite of everything
            // below it. A Lens flare added to one starts here.
            if (sound ||
                ownerLayers[i].layer.internallayerId == ownerLayerId ||
                ownerLayers[i].layer.hasPicture())
              (
                ownerLayers[i].layer.internallayerId,
                '${i + 1}. '
                    '${named(ownerLayers[i].info.name, ownerLayers[i].layer.internallayerId)}'
              ),
        ],
        onChanged: (picked) => _set(BridgeEffectValue.layer(picked)),
      ),
    );
  }

  /// The masks of the layer this effect sits on, by name.
  ///
  /// An effect that walks a shape reads one of *this layer's* masks — a mask
  /// belongs to the layer it was drawn on — so the list comes from the owner's
  /// own entry in the read model the panel already holds. No call crosses the
  /// bridge to build it: this runs on every rebuild, and the budget test counts
  /// zero.
  ///
  /// **"First mask"** is the unset entry rather than "None": what an unset row
  /// comes to is the engine's answer (the schema's `self_default`), and for
  /// every effect that takes a path it is the layer's first mask. A layer with
  /// no masks yet still shows it — that is the honest reading of an effect
  /// dropped on before the shape is drawn.
  Widget _maskPicker(BuildContext context, UuidValue id, UuidValue? current) {
    final masks = ownerLayers
            .where((l) => l.layer.internallayerId == ownerLayerId)
            .map((l) => l.info.masks)
            .firstOrNull ??
        const <BridgeMask>[];
    return SizedBox(
      width: effectCellWidth + 40,
      child: BareLazyDropdown<UuidValue?>(
        key: ValueKey<String>('fx-mask-$id-${param.id}'),
        label: current == null
            ? l10n.firstMask
            : (masks
                    .where((m) => m.id == current)
                    .map((m) => m.name)
                    .firstOrNull ??
                l10n.missingMask),
        options: () => [
          (null, l10n.firstMask),
          // Every mask, including one in None mode: that mode is "geometry
          // only, gates nothing", which is exactly the mask somebody draws
          // *for* an effect to walk.
          for (final m in masks) (m.id, m.name),
        ],
        onChanged: (picked) => _set(BridgeEffectValue.maskPath(picked)),
      ),
    );
  }

  /// The row's control with its [riders] beside it — the uniform Matte row
  /// (picker, Channel, Invert), which every effect has and which
  /// the two that owned the idea first (Depth of field's depth pass, the Lens
  /// flare's source matte) share, and the Mix row with its Blend.
  /// Without riders this is the control alone.
  ///
  /// A switch's word is drawn here rather than in the name column because the
  /// name column already holds this row's word. Two controls on one row need
  /// two labels, and the second one goes where the second control is. A choice
  /// says its own value, and a one-word tooltip names it.
  Widget _withRiders(LumitTheme t, UuidValue id, Widget control) {
    if (riders.isEmpty) return control;
    // Below this width the riders' words start eliding into "Nor…"; a value
    // nobody can read is worse than a second line, so the riders drop onto
    // one of their own instead (still inside the control column, so the
    // fixed label edge holds).
    const wrapBelow = 230.0;
    return LayoutBuilder(builder: (context, constraints) {
      // A second line only exists where the host can grow to hold it: the
      // Effect controls' rows size to their content, but a Timeline fold-out
      // row is a fixed lane height, and a wrapped line there is 20px of
      // invisible overflow. In a fixed row the riders stay on the one line
      // and elide instead — "Nor…" can at least be hovered; a clipped second
      // line cannot be seen at all.
      final canGrow =
          !constraints.hasBoundedHeight || constraints.maxHeight >= 44;
      final riderRow = [
        for (final (p, v) in riders) ...[
          const SizedBox(width: 6),
          Flexible(
            child: Row(mainAxisSize: MainAxisSize.min, children: [
              ..._rider(t, id, p, v),
            ]),
          ),
        ],
      ];
      if (constraints.maxWidth >= wrapBelow || !canGrow) {
        return Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            // Flexible, so a narrow panel shrinks the host and its riders
            // rather than overflowing the row; at the panel's working width
            // every one of them gets its natural size.
            Flexible(flex: 3, child: control),
            ...riderRow,
          ],
        );
      }
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          control,
          const SizedBox(height: 2),
          Row(mainAxisSize: MainAxisSize.min, children: [
            // The leading 6px spacer indents the run to match the gap the
            // riders keep beside the control on the one-line layout.
            ...riderRow,
          ]),
        ],
      );
    });
  }

  List<Widget> _rider(
      LumitTheme t, UuidValue id, BridgeParamInfo p, BridgeEffectValue? v) {
    // The key a row of its own would have drawn under: the control moved
    // house, it did not become a different control.
    switch (p.kind) {
      case BridgeParamKind_Bool():
        final on = switch (v) {
          BridgeEffectValue_Bool(:final field0) => field0,
          _ => false,
        };
        return [
          HouseCheckbox(
            key: ValueKey<String>('fx-bool-$id-${p.id}'),
            value: on,
            onChanged: (next) =>
                onWrite(effectId, p.id, BridgeEffectValue.bool(next)),
          ),
          const SizedBox(width: 6),
          Flexible(
            child: Text(
              engineLabel(p.label),
              style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ];
      case BridgeParamKind_Choice(:final options):
        final index = switch (v) {
          BridgeEffectValue_Choice(:final field0)
              when field0 < options.length =>
            field0.toInt(),
          _ => 0,
        };
        return [
          Flexible(
            flex: 2,
            child: LumitTooltip(
              message: engineLabel(p.label),
              child: BareDropdown<int>(
                key: ValueKey<String>('fx-choice-$id-${p.id}'),
                value: index,
                options: [for (var i = 0; i < options.length; i++) i],
                label: (i) => engineLabel(options[i]),
                onChanged: (i) =>
                    onWrite(effectId, p.id, BridgeEffectValue.choice(i)),
              ),
            ),
          ),
        ];
      default:
        return const [];
    }
  }
}

/// Two `_x`/`_y` Float parameters as ONE point row (docs/07 §6.1): the pair
/// convention the Lens flare's light and Radial blur's centre follow. One
/// label (the shared stem), one stopwatch carrying both channels — the
/// Position-row shape — two value fields, and for %-of-frame pairs a
/// position dropper that picks the point off the Viewer.
class EffectPointRowFrb extends StatelessWidget {
  final UuidValue effectId;
  final BridgeParamInfo xParam;
  final BridgeParamInfo yParam;
  final BridgeEffectValue? xValue;
  final BridgeEffectValue? yValue;
  final CompositionReference comp;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final void Function(UuidValue effect, String param, BridgeEffectValue value)
      onWrite;
  final void Function(UuidValue effect, String param, BridgeEffectValue value)
      onLive;

  /// Write several of this effect's parameters as **one op**, for the chain's
  /// proportional write: two [onWrite] calls are two undo steps, and a gesture
  /// that moved one well is one edit. Null where the caller has no stack to
  /// commit through — the Graph panel's driver boxes, which never chain.
  final void Function(UuidValue effect, Map<String, BridgeEffectValue> values)?
      onWritePair;

  final bool twoColumn;

  /// Whether the pair is editable, per the effect's greying rules — the same
  /// affordance [EffectParamRowFrb.enabled] draws, over a row that happens to
  /// carry two parameters.
  final bool enabled;

  /// Whether the two are **chained**: dragging or typing one scales
  /// the other by the same factor. The document holds which pairs are tied
  /// ([BridgeEffectInstanceInfo.linkedPairs]); the scaling itself is done here
  /// while the gesture is live, and deliberately not in the model.
  final bool linked;

  /// Tie or untie the pair. Null where nothing can write — the Timeline's
  /// fold-out today — which is what makes the chain absent rather than dead.
  final VoidCallback? onToggleLink;

  const EffectPointRowFrb({
    super.key,
    required this.effectId,
    required this.xParam,
    required this.yParam,
    required this.xValue,
    required this.yValue,
    required this.comp,
    required this.playheadFrame,
    required this.onSeek,
    required this.onWrite,
    required this.onLive,
    this.onWritePair,
    this.twoColumn = false,
    this.enabled = true,
    this.linked = false,
    this.onToggleLink,
  });

  /// Whether the pair takes the **position dropper**, and in what unit a pick
  /// writes — read off the declaration, never off the parameter's id.
  ///
  /// `null` is no dropper. `Px` writes comp PIXELS (fraction × comp size, read
  /// at CLICK time, never in a rebuild), and `Percent` writes
  /// fraction × 100, which is what Radial blur's Centre has always been. Every
  /// other unit gets no crosshair: a point picked off the picture is a
  /// *position*, and no arrangement of degrees or seconds is one.
  ///
  /// This replaced a Dart map from parameter id to "pixels or per cent", which
  /// could not tell Radial blur's per-cent `centre_x` from the dozen effects
  /// whose `centre_x` is px@comp — so all of them wrote per cent.
  bool? get _pickPixels => switch (xParam.unit) {
        BridgeUnit.px => true,
        BridgeUnit.percent => false,
        _ => null,
      };

  BridgeScalar? _scalar(BridgeEffectValue? v) => switch (v) {
        BridgeEffectValue_Float(:final field0) => field0,
        _ => null,
      };

  @override
  Widget build(BuildContext context) {
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    // Still on both axes: nothing here follows a scrub. See the note on
    // `EffectParamRowFrb.build`.
    if (_scalar(xValue) is BridgeScalar_Static &&
        _scalar(yValue) is BridgeScalar_Static) {
      return _build(context, playhead.value);
    }
    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) => _build(context, frame),
    );
  }

  Widget _build(BuildContext context, int frame) {
    final t = ThemeScope.of(context).theme;
    final id = effectId;
    final sx = _scalar(xValue);
    final sy = _scalar(yValue);
    // The shared stem: "Light x" / "Light y" → "Light".
    var stem = xParam.label;
    if (stem.toLowerCase().endsWith(' x')) {
      stem = stem.substring(0, stem.length - 2);
    }

    final keyframes = (sx == null || sy == null)
        ? null
        : KeyframeControlsFrb(
            scalars: [sx, sy],
            comp: comp,
            playheadFrame: playheadFrame,
            onSeek: onSeek,
            rowKey: '$id-${xParam.id}-pair',
            fixedColumns: twoColumn,
            // Two parameters, so two writes: a keyframe op on the pair costs
            // two undo steps today (the staged editor commits per param).
            onWrite: (next) {
              if (next.length == 2) {
                onWrite(id, xParam.id, BridgeEffectValue.float(next[0]));
                onWrite(id, yParam.id, BridgeEffectValue.float(next[1]));
              }
            },
          );

    final label = Text(
      stem,
      style: enabled ? t.body : t.body.copyWith(color: t.textDisabled),
      overflow: TextOverflow.ellipsis,
    );

    // Deaf, not faded (docs/15 §5): a control that still answers a drag while
    // looking disabled is worse than one that never changed, and the label's
    // `text_disabled` says which it is without hiding the number.
    Widget greyed(Widget child) =>
        enabled ? child : IgnorePointer(child: child);

    // **The chain, while it is on.** Scaling the sibling is UI-time
    // arithmetic for the life of the gesture — the document's business is only
    // *which* pairs are tied, so nothing below reaches the engine
    // except the two writes it would have made anyway.
    //
    // The factor comes off the well being dragged: `next / before`, where
    // *before* is the number that well is showing — its static value, or what
    // its curve reads at the playhead. A value of **nought** has no factor at
    // all — every number is nought times something — so a pair dragged off
    // zero separates instead of staying stuck there.
    //
    // A **keyed** sibling of a static well scales whole: every keyframe's
    // value times the factor, each key keeping its time, its interpolation
    // and its eased shape, which is the same arithmetic `scale_property` does
    // engine-side ([scaledScalar]). Once the well is keyed too, its edit is a
    // key at the playhead, so the sibling takes a key there as well, at the
    // ratio the pair reads on that frame. Stretching its whole curve instead
    // left the two halves agreeing at the playhead and nowhere else.
    double? currentOf(BridgeScalar? scalar) => switch (scalar) {
          BridgeScalar_Static(:final field0) => field0,
          final BridgeScalar_Keyframed keyed =>
            sampledScalar(keyed, timeOfFrame(comp, frame)),
          _ => null,
        };

    void writeChannel(BridgeParamInfo param, double next,
        {required bool live}) {
      final scalar = param.id == xParam.id ? sx : sy;
      final before = currentOf(scalar);
      final value = BridgeEffectValue.float(scalar == null
          ? BridgeScalar.static_(next)
          : scalarWithValueAt(scalar, next, comp, frame));
      final other = param.id == xParam.id ? yParam : xParam;
      final otherScalar = other.id == xParam.id ? sx : sy;
      BridgeEffectValue? scaled;
      if (linked && before != null && before != 0 && otherScalar != null) {
        final otherBefore = currentOf(otherScalar);
        if (otherBefore != null) {
          scaled = BridgeEffectValue.float(scalar is BridgeScalar_Keyframed &&
                  otherScalar is BridgeScalar_Keyframed
              ? scalarWithValueAt(
                  otherScalar, next * otherBefore / before, comp, frame)
              : scaledScalar(otherScalar, next / before));
        }
      }
      // Both halves as **one** op where the caller can commit one: two writes
      // would be two undo steps for a gesture that moved one well.
      if (!live && scaled != null && onWritePair != null) {
        onWritePair!(id, {param.id: value, other.id: scaled});
        return;
      }
      final write = live ? onLive : onWrite;
      write(id, param.id, value);
      if (scaled != null) write(id, other.id, scaled);
    }

    Widget field(BridgeParamInfo param, BridgeScalar? scalar) {
      if (scalar == null) return Text('—', style: t.small);
      final kind = param.kind;
      if (kind is! BridgeParamKind_Float) return Text('—', style: t.small);
      final span = (kind.sliderMax - kind.sliderMin).abs();
      final speed = span <= 0 ? 0.5 : span / 200;
      if (scalar case BridgeScalar_Keyframed()) {
        final sampled =
            sampledScalar(scalar, timeOfFrame(comp, frame));
        return SizedBox(
          width: effectCellWidth,
          child: KeyedValueField(
            fieldKey: ValueKey<String>('fx-float-$id-${param.id}'),
            value: sampled,
            min: kind.hardMin ?? -1000000,
            max: kind.hardMax ?? 1000000,
            speed: speed,
            // Through the same road a static well writes through, so a keyed
            // half drags its chained sibling exactly as a static one does.
            onCommit: (v) => writeChannel(param, v, live: false),
          ),
        );
      }
      return SizedBox(
        width: effectCellWidth,
        child: DragValueField(
          key: ValueKey<String>('fx-float-$id-${param.id}'),
          value: (scalar as BridgeScalar_Static).field0,
          min: kind.hardMin ?? -1000000,
          max: kind.hardMax ?? 1000000,
          speed: speed,
          decimals: 2,
          // Typing keeps the ratio too, not only dragging: the chain is about
          // the two numbers, not about which gesture moved one of them.
          onChanged: (v) => writeChannel(param, v.toDouble(), live: false),
          onChangeLive: (v) => writeChannel(param, v.toDouble(), live: true),
          onChangeEnd: (v) => writeChannel(param, v.toDouble(), live: false),
        ),
      );
    }

    final pickPixels = _pickPixels;
    final control = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        field(xParam, sx),
        // The chain, **between** the two wells, which is where the mockup puts
        // it: it belongs to neither half and to both.
        if (onToggleLink != null) ...[
          const SizedBox(width: effectRiderGap),
          LumitTooltip(
            message: linked ? l10n.tipUnlinkPair : l10n.tipLinkPair,
            child: GestureDetector(
              key: ValueKey<String>('fx-pair-link-$id-${xParam.id}'),
              behavior: HitTestBehavior.opaque,
              onTap: onToggleLink,
              child: LumitIcon(
                linked ? LumitIcons.link : LumitIcons.unlink,
                size: 10,
                colour: linked ? t.textPrimary : t.textMuted,
              ),
            ),
          ),
        ],
        const SizedBox(width: effectRiderGap),
        field(yParam, sy),
        // **One rider for the pair**, after both wells, as the mockup draws
        // it: x and y are two halves of one measurement, and stating the unit
        // twice would read as two.
        if (unitRiderText(xParam.unit) case final unit?) ...[
          const SizedBox(width: effectRiderGap),
          Text(unit, style: unitRiderStyle(t)),
        ],
        if (pickPixels != null) ...[
          const SizedBox(width: effectRiderGap),
          _DropperButton(
            id: 'fx-$id-${xParam.id}',
            glyph: LumitIcons.pointPicker,
            tip: l10n.tipPickOnViewer,
            arm: (ui) {
              // What one unit of the fraction is worth. A `Px` pair writes
              // fraction × comp size; a `Percent` pair writes
              // fraction × 100 — which is the same sum with the comp's size
              // set to a hundred, so there is one arithmetic here and not two.
              //
              // The size is read **once, when the tool is armed** — a tap, and
              // so an edit rather than a rebuild. It used to be read
              // at the pick, which was also once; now that a pick previews per
              // pointer move, reading it there would be a bridge call per
              // move, which the budget forbids outright.
              var spanX = 100.0, spanY = 100.0;
              if (pickPixels) {
                try {
                  final size = comp.getSize();
                  spanX = size.width.toDouble();
                  spanY = size.height.toDouble();
                } catch (_) {
                  return; // the comp has gone; do not arm at all
                }
              }
              // Both halves at once, so a pick is never scaled by the chain:
              // it is a position, and a position is stated rather than nudged.
              //
              // **A pick is a typed value, not a reset**: a keyed axis
              // takes a key at the playhead through the same `scalarWithValueAt`
              // the well beside it writes through, and a static one stays
              // static. It used to state both halves as bare statics, which is
              // how picking a Centre on an animated point threw its whole path
              // away.
              BridgeEffectValue at(BridgeScalar? was, double v) =>
                  BridgeEffectValue.float(was == null
                      ? BridgeScalar.static_(v)
                      : scalarWithValueAt(was, v, comp, frame));

              void put(
                void Function(UuidValue, String, BridgeEffectValue) write,
                DropperSample sample,
              ) {
                write(id, xParam.id, at(sx, sample.xFrac * spanX));
                write(id, yParam.id, at(sy, sample.yFrac * spanY));
              }

              ui.armDropper(DropperArm(
                id: 'fx-$id-${xParam.id}',
                reads: DropperReads.position,
                label: stem,
                // **The drag is the pick**: the point follows the
                // pointer through the preview, and the release states it once.
                onPreview: (sample) => put(onLive, sample),
                onPick: (sample) => put(onWrite, sample),
                // Abandoned: the two numbers the row had, back through the
                // same preview path. Nothing was committed either half.
                onRevert: (sx == null || sy == null)
                    ? null
                    : () {
                        onLive(id, xParam.id, BridgeEffectValue.float(sx));
                        onLive(id, yParam.id, BridgeEffectValue.float(sy));
                      },
              ));
            },
          ),
        ],
      ],
    );

    if (twoColumn) {
      return fxTwoColumnRow(
        context: context,
        name: label,
        keyframeControls: keyframes == null ? null : greyed(keyframes),
        control: greyed(control),
      );
    }
    // No padding of its own: the Timeline's fold-out row is 22 and a
    // 20px value well fills all but its hairline, so two pixels either side
    // pushed the wells out of the row.
    return Row(
      children: [
        if (keyframes != null) greyed(keyframes),
        const SizedBox(width: 4),
        Expanded(child: label),
        const SizedBox(width: 10),
        greyed(control),
      ],
    );
  }
}

/// The effect schema, fetched once per session and then answered from here.
///
/// `listEffects` serialises every built-in's declaration and `listParameters`
/// one effect's worth; both are static for the life of the process, yet they
/// were being re-fetched per card per rebuild — the whole schema crossing the
/// bridge to look up one display label. Memoised, a rebuild costs nothing here.
List<BridgeEffectInfo>? _effectSchema;
List<BridgeEffectInfo> cachedListEffects() => _effectSchema ??= listEffects();

/// All nine layer styles, memoised for exactly [cachedListEffects]'
/// reason: a fixed table that was crossing the bridge every time a menu tree or
/// a panel heading was rebuilt.
///
/// Nine, because two questions are asked of it. [offeredStyles] is the menus'
/// answer; this one is the *naming* answer, and an imported Satin has to be
/// nameable even though nothing may add one.
List<BridgeStyleInfo>? _styleSchema;
List<BridgeStyleInfo> styleCatalogue() => _styleSchema ??= listStyles();

/// The styles a menu may offer — the seven this version renders (§8).
Iterable<BridgeStyleInfo> offeredStyles() =>
    styleCatalogue().where((s) => s.offered);

final Map<String, List<BridgeParamInfo>> _paramSchema = {};
List<BridgeParamInfo> cachedListParameters(String effect) =>
    _paramSchema[effect] ??= listParameters(effect: effect);

final Map<String, List<BridgeParamGroup>> _groupSchema = {};

/// An effect's parameter groups (docs/08 §1.2), memoised like
/// the parameters: the twirls and conditional runs the panel folds the flat
/// parameter list into.
List<BridgeParamGroup> cachedListParameterGroups(String effect) =>
    _groupSchema[effect] ??= listParameterGroups(effect: effect);

final Map<String, List<BridgeEnabledWhen>> _enabledWhenSchema = {};

/// An effect's greying rules (`EnabledWhen` in the schema), memoised like the
/// groups: which rows go quiet while another control has taken them over.
List<BridgeEnabledWhen> cachedListEnabledWhen(String effect) =>
    _enabledWhenSchema[effect] ??= listEnabledWhen(effect: effect);

final Map<String, List<BridgeParamPair>> _pairSchema = {};

/// An effect's **vector pairs** — the `_x`/`_y` runs the panel folds
/// into one point row — memoised like the rest of the schema, and for the same
/// reason: it never changes, and a fetch per card per rebuild is the traffic
/// the budget test forbids.
///
/// Read from the declaration rather than worked out from the ids here. The
/// convention is the same one either way; having the engine state it is what
/// lets the panel, the link flag on the instance and the render agree about
/// what a pair *is* without three copies of the rule.
List<BridgeParamPair> cachedListPairs(String effect) =>
    _pairSchema[effect] ??= listPairs(effect: effect);

/// The stem the pair beginning at `xId` is keyed under, or null when that
/// parameter does not begin one.
String? pairStemOf(String effect, String xId) {
  for (final pair in cachedListPairs(effect)) {
    if (pair.x == xId) return pair.stem;
  }
  return null;
}

/// Which of `effect`'s parameters are currently NOT editable, given the values
/// the panel is showing.
///
/// Mirrors `lumit_core::fx::param_enabled`, which is the authority: the rules
/// are evaluated here rather than asked for across the bridge because the panel
/// already holds every value they read, and a round trip per row per rebuild for
/// an answer it can compute is exactly the hover-hot bridge traffic the budget
/// test forbids. Several rules may name the same parameter; every one of them
/// has to be satisfied, so one unsatisfied rule greys the row.
///
/// A rule naming a parameter the instance does not carry cannot be judged, so it
/// greys nothing — an older instance that predates the deciding control stays
/// fully editable rather than locking a row it can never unlock.
Set<String> disabledParams(
  String effect,
  Map<String, BridgeEffectValue> values,
) {
  final out = <String>{};
  for (final rule in cachedListEnabledWhen(effect)) {
    final on = values[rule.on_];
    if (on == null) continue;
    final ok = switch ((rule.cond, on)) {
      (
        BridgeEnabledCond_BoolIs(:final field0),
        BridgeEffectValue_Bool(field0: final v)
      ) =>
        v == field0,
      (
        BridgeEnabledCond_ChoiceIs(:final field0),
        BridgeEffectValue_Choice(field0: final v)
      ) =>
        v == field0,
      (
        BridgeEnabledCond_ChoiceIsNot(:final field0),
        BridgeEffectValue_Choice(field0: final v)
      ) =>
        v != field0,
      (
        BridgeEnabledCond_LayerSet(),
        BridgeEffectValue_Layer(field0: final v)
      ) =>
        v != null,
      // A rule pointed at the wrong kind of parameter is a schema mistake the
      // Rust-side test fails the build for; here it leaves the row live rather
      // than locking one the owner can never reach.
      _ => true,
    };
    if (!ok) out.add(rule.param);
  }
  return out;
}

/// An effect's display label from the schema, falling back to its match name
/// for an effect this build does not know.
///
/// **The layer styles answer here too**. They are deliberately not in
/// the effect catalogue — the Add-effect menu must never offer "Drop shadow
/// (style)" beside the Drop shadow effect — but a style's card, its Timeline
/// heading and the dope sheet's flat list all ask this one question, and a
/// heading reading `style_drop_shadow` is what leaving them out looks like.
/// The engine makes the same join in `fx::def`, and for the same reason.
String effectLabelOf(String name) {
  for (final info in cachedListEffects()) {
    if (info.name == name) return engineLabel(info.label);
  }
  for (final style in styleCatalogue()) {
    if (style.name == name) return engineLabel(style.label);
  }
  return name;
}

/// What a parameter holds before anything touches it — what Reset writes.
///
/// Read straight off the schema, which already carries every default and is
/// memoised here, so resetting an effect costs no bridge call to work out *what*
/// to write. Seed, file and layer declare none: a seed's default is zero, an
/// unset file is no paths, and an unset layer reference is None — each of which
/// is the identity the effect treats as "not configured".
/// **An Action has no value**, so it has no default either: Reset
/// walks every parameter and this answers `null` for the one kind that is a
/// button, which the caller skips.
BridgeEffectValue? defaultEffectValue(BridgeParamKind kind) => switch (kind) {
      BridgeParamKind_Action() => null,
      BridgeParamKind_Float(:final default_) =>
        BridgeEffectValue.float(BridgeScalar.static_(default_)),
      BridgeParamKind_Int(:final default_) =>
        BridgeEffectValue.float(BridgeScalar.static_(default_.toDouble())),
      // An angle is a number of degrees, so it resets like any other scalar.
      BridgeParamKind_Angle(:final default_) =>
        BridgeEffectValue.float(BridgeScalar.static_(default_)),
      BridgeParamKind_Choice(:final default_) =>
        BridgeEffectValue.choice(default_),
      BridgeParamKind_Bool(:final default_) => BridgeEffectValue.bool(default_),
      BridgeParamKind_Colour(:final default_) =>
        BridgeEffectValue.colour(BridgeColour(
          r: BridgeScalar.static_(_channel(default_, 0)),
          g: BridgeScalar.static_(_channel(default_, 1)),
          b: BridgeScalar.static_(_channel(default_, 2)),
          a: BridgeScalar.static_(_channel(default_, 3, fallback: 1)),
        )),
      BridgeParamKind_Seed() => const BridgeEffectValue.seed(0),
      BridgeParamKind_File() => BridgeEffectValue.file(
          const BridgeFileParam(paths: [], index: BridgeScalar.static_(0))),
      BridgeParamKind_Layer() => const BridgeEffectValue.layer(),
      // Unset is "First mask", not "no mask" — what it comes to is
      // the engine's answer, not a value written here.
      BridgeParamKind_MaskPath() => const BridgeEffectValue.maskPath(),
      // A closed range stores an ordinary float: the kind is the
      // control, not the storage.
      BridgeParamKind_Slider(:final default_) =>
        BridgeEffectValue.float(BridgeScalar.static_(default_)),
      // Every curve's default is the identity diagonal — there is
      // nothing per-parameter to declare, which is why the kind carries no
      // fields.
      BridgeParamKind_Curve() => curveValue(curveIdentity),
    };

/// A curve value's points as plain `[x, y]` pairs, straightened only as far as
/// the editor needs: the diagonal stands in for anything with fewer than two
/// usable points, exactly as the engine's own read does (docs/08 §3.30).
List<List<double>> curvePointsOf(List<Float32List> raw) {
  final points = <List<double>>[
    for (final p in raw)
      if (p.length >= 2) [p[0].toDouble(), p[1].toDouble()],
  ];
  return points.length >= 2 ? points : curveIdentity;
}

/// The bridge value for a point list. Written as the editor holds it — the
/// engine straightens what it reads, so a curve mid-drag is never refused for
/// being momentarily out of order (docs/17).
BridgeEffectValue curveValue(List<List<double>> points) =>
    BridgeEffectValue.curve([
      for (final p in points) Float32List.fromList([p[0], p[1]]),
    ]);

/// One channel of a declared colour default, tolerating a short list — a schema
/// that names only RGB still resets to an opaque colour rather than throwing.
double _channel(List<double> rgba, int i, {double fallback = 0}) =>
    i < rgba.length ? rgba[i] : fallback;

/// The last path segment, for showing a chosen file without its whole path.
String _basename(String path) {
  final cut = path.lastIndexOf(RegExp(r'[/\\]'));
  return cut < 0 ? path : path.substring(cut + 1);
}

/// The staging behind a drag on an effect parameter, and the writes that end it.
///
/// Held by whichever panel is showing the rows — the Effect controls card and
/// the Timeline's fold-out each keep one. It exists because the *handles* cannot
/// be kept: what is staged is the edit (which effect, which parameter, which
/// value), and every call that consumes a stack gets a freshly read one with
/// that edit written into it.
class EffectStackEditor {
  /// Where a **group header's** stack is read from
  /// (docs/impl/group-effects.md §6), set by whichever panel is showing a
  /// header's rows — the third place [stackWith] can look, after the layer's
  /// effects and its styles, exactly as the engine's own shared instance
  /// lookup does. Null for every ordinary editor.
  List<BridgeEffectInstance> Function()? groupStack;

  /// The parameters this gesture has staged, by effect and parameter id.
  ///
  /// **A map rather than one edit, because a gesture can move two parameters
  /// at once.** A linked pair scales its sibling as it drags, and a point
  /// picked on the Viewer moves x and y together — with a single slot the
  /// second `live` call simply erased the first, so the preview showed one half
  /// of the change moving and the other standing still. Committing is
  /// unaffected: [write] still commits the one parameter it is given.
  final Map<(UuidValue, String), BridgeEffectValue> _staged = {};

  /// Roughly one preview render per 20 ms, so a fast drag cannot outrun the
  /// renderer and queue up work it will only throw away — but the tick that
  /// lands inside the interval is *held*, not dropped, so the pointer's last
  /// position always reaches the picture ([PreviewThrottle]).
  final PreviewThrottle _throttle = PreviewThrottle();

  /// The value a row should *show*, which during a drag is the staged one.
  BridgeEffectValue? stagedValue(UuidValue effect, String param) =>
      _staged[(effect, param)];

  /// The layer's stack with the drag in progress written into it, freshly read
  /// — **or its style list**, when that is where the staged parameter lives
  /// (docs/impl/layer-styles.md §5).
  ///
  /// Which one is read off the ids, here and only here, so a style row's drag,
  /// its typed value, its keyframe and its preview are the effect row's code
  /// unchanged. The second read costs nothing on the ordinary path: every
  /// staged id being in the stack is the common case and settles it.
  List<BridgeEffectInstance> stackWith(LayerReference layer) {
    final effects = layer.getEffects();
    final ids = {for (final instance in effects) instance.id()};
    var stack = effects;
    if (!_staged.keys.every((key) => ids.contains(key.$1))) {
      // Styles next, then — for an editor showing a group header's rows —
      // the header's own list: the same order the engine's shared
      // lookup searches, so the commit lands on the list the id lives in.
      final styles = layer.getStyles();
      final styleIds = {for (final s in styles) s.id()};
      stack = _staged.keys.every((key) => styleIds.contains(key.$1))
          ? styles
          : (groupStack?.call() ?? styles);
    }
    for (final instance in stack) {
      final id = instance.id();
      for (final entry in _staged.entries) {
        if (entry.key.$1 == id) {
          instance.setValue(id: entry.key.$2, value: entry.value);
        }
      }
    }
    return stack;
  }

  /// A drag tick: stage the value and render it, throttled. Nothing is
  /// committed, so the document and the undo history never see a tick.
  void live(
    CompositionReference comp,
    LayerReference layer,
    UuidValue effect,
    String param,
    BridgeEffectValue value, {
    required int frame,
    required double scale,
  }) {
    _staged[(effect, param)] = value;
    // A group header's drag stages without a live preview render: the
    // preview overlay stands a stack in for the LAYER's own, and a header's
    // stack is not that — the picture would blur the carrier alone. The row
    // still shows the staged value, and the real picture follows on commit.
    // ponytail: no live group preview; ceiling = a header drag repaints on
    // release only. Upgrade: a group overlay on render_frame_with_preview.
    // Trigger: the owner asking why the drag does not show.
    if (groupStack != null) return;
    // The stack is read *inside* the closure: a held tick must send the newest
    // staged value, not the one that was current when it was held.
    _throttle.request(() => comp.renderFrameWithPreview(
          frame: BigInt.from(frame),
          scale: scale,
          layer: layer,
          effects: stackWith(layer),
        ));
  }

  /// A release, or a typed value: the whole stack as one op.
  ///
  /// A stack another panel has changed under us is refused engine-side
  /// (`StaleEffectStack`); re-reading is the recovery, so the panel shows the
  /// document rather than insisting on its own copy.
  void write(
    LayerReference layer,
    UuidValue effect,
    String param,
    BridgeEffectValue value,
  ) =>
      writeAll(layer, effect, {param: value});

  /// Several parameters of one effect as **one** op, and so one undo step —
  /// what a chained pair's proportional write is, where two [write]
  /// calls would undo half a gesture at a time.
  void writeAll(
    LayerReference layer,
    UuidValue effect,
    Map<String, BridgeEffectValue> values,
  ) {
    // A release ends the drag: a held preview tick would render provisional
    // values *after* the commit, putting the pre-commit picture back on screen.
    _throttle.cancel();
    // **These parameters and no others.** Everything the gesture previewed is
    // dropped first, so a commit writes exactly what it was handed rather than
    // folding a stale tick into someone else's op.
    _staged.clear();
    for (final entry in values.entries) {
      _staged[(effect, entry.key)] = entry.value;
    }
    try {
      layer.setEffects(effects: stackWith(layer));
    } catch (_) {
      // Someone else edited the stack mid-drag. Drop ours and re-read.
    }
    _staged.clear();
  }

  /// Forget any drag in progress — a cancelled gesture.
  void clear() {
    _throttle.cancel();
    _staged.clear();
  }
}

/// The little pipette beside a parameter: click to arm the dropper, click it
/// again (or press Escape, or click away from the picture) to put it away.
///
/// It lights while *this* parameter's pick is the armed one, so a dropper armed
/// from one row and forgotten is visible from across the panel.
class _DropperButton extends StatelessWidget {
  /// This button's arm id — compared with the armed one to know when to light.
  final String id;
  final String tip;
  final void Function(LumitUiState ui) arm;

  /// Which glyph it wears: the pipette for a colour or a depth, the crosshair
  /// for a **point** picked on the Viewer (§12A.3 — a position parameter gets a
  /// point picker exactly as a colour parameter gets an eyedropper).
  final String glyph;

  const _DropperButton(
      {required this.id,
      required this.tip,
      required this.arm,
      this.glyph = LumitIcons.eyedropper});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return ValueListenableBuilder<DropperArm?>(
      valueListenable: ui.dropper,
      builder: (context, armed, _) {
        final lit = armed?.id == id;
        return LumitTooltip(
          message: tip,
          child: GestureDetector(
            key: ValueKey<String>('dropper-$id'),
            behavior: HitTestBehavior.opaque,
            onTap: () => lit ? ui.disarmDropper() : arm(ui),
            child: MouseRegion(
              cursor: SystemMouseCursors.click,
              child: SizedBox(
                width: 18,
                height: 18,
                child: Center(
                  child: LumitIcon(
                    glyph,
                    size: iconSize,
                    colour: lit ? t.accent : t.textSecondary,
                  ),
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

class EffectParamRowExpression extends StatefulWidget {
  const EffectParamRowExpression(
      {required this.value,
      required this.set,
      required this.comp,
      required this.frame,
      required this.setLive,
      required this.layer,
      super.key});
  final BridgeScalar_Expression value;
  final CompositionReference comp;
  final int frame;
  final void Function(BridgeEffectValue value) set;
  final void Function(BridgeEffectValue value) setLive;
  final LayerReference layer;

  @override
  State<EffectParamRowExpression> createState() =>
      _EffectParamRowExpressionState();
}

const _defaultLightThemeFiles = [
  'packages/syntax_highlight/themes/light_vs.json',
  'packages/syntax_highlight/themes/light_plus.json',
];

const _defaultDarkThemeFiles = [
  'packages/syntax_highlight/themes/dark_vs.json',
  'packages/syntax_highlight/themes/dark_plus.json',
];

class ExpressionTextEditingController extends TextEditingController {
  static HighlighterTheme? darkTheme;
  static HighlighterTheme? lightTheme;

  static Future<void> initSyntaxHighlighting() async {
    await Highlighter.initialize(["dart"]);

    darkTheme = await HighlighterTheme.loadFromAssets(
        _defaultDarkThemeFiles, LumitTheme.dark().mono);

    lightTheme = await HighlighterTheme.loadFromAssets(
        _defaultLightThemeFiles, LumitTheme.light().mono);
  }

  ExpressionTextEditingController({super.text});

  @override
  TextSpan buildTextSpan(
      {required BuildContext context,
      TextStyle? style,
      required bool withComposing}) {
    final theme = ThemeScope.of(context).theme.mode == ThemeMode2.dark
        ? darkTheme
        : lightTheme;

    // Highlighting is loaded asynchronously at startup by
    // `initSyntaxHighlighting`, so there is a window in which it is not there
    // yet — and a widget test never runs that startup at all. Draw the line
    // plainly until it is ready rather than throwing, which is the same choice
    // the completion list already makes when the engine has not answered.
    if (theme == null) {
      return super.buildTextSpan(
          context: context, style: style, withComposing: withComposing);
    }

    var highlighter = Highlighter(
      language: 'dart',
      theme: theme,
    );

    var span = highlighter.highlight(text);
    return span;
  }
}

class _EffectParamRowExpressionState extends State<EffectParamRowExpression> {
  late TextEditingController controller;

  double value = 0.0;
  late ValueNotifier<int> playhead;

  String lastText = "";

  @override
  void initState() {
    playhead = Provider.of<LumitUiState>(context, listen: false).playheadFrame;

    Provider.of<LumitState>(context, listen: false)
        .onChange
        .listen(onProjectChanged);

    playhead.addListener(onFrameChanged);

    controller = ExpressionTextEditingController(text: widget.value.field0);
    controller.addListener(onTextChanged);
    lastText = controller.text;

    value = sampleScalarWithContext(
        scalar: widget.value,
        time: timeOfFrame(widget.comp, playhead.value),
        layer: widget.layer);
    super.initState();
  }

  void onProjectChanged(ScopedChange event) {
    // print(event.layer);
    // print("Project changed!");

    // setState(() {
    //   controller.text = widget.value.field0;
    // });
  }

  @override
  void dispose() {
    playhead.removeListener(onFrameChanged);
    controller.removeListener(onTextChanged);

    super.dispose();
  }

  @override
  void didUpdateWidget(covariant EffectParamRowExpression oldWidget) {
    if (widget.value.field0 != controller.text) {
      // we dont want to trigger the update when setting text manually, so remove it then add it back
      controller.removeListener(onTextChanged);
      WidgetsBinding.instance.addPostFrameCallback((_) {
        controller.text = widget.value.field0;
        controller.addListener(onTextChanged);
      });
    }
    super.didUpdateWidget(oldWidget);
  }

  void onFrameChanged() {
    final expr = controller.text;

    setState(() {
      value = sampleScalarWithContext(
          scalar: BridgeScalar_Expression(expr),
          time: timeOfFrame(widget.comp, playhead.value),
          layer: widget.layer);
    });
  }

  void onTextChanged() {
    final expr = controller.text;
    if (expr != lastText) {
      widget.setLive(BridgeEffectValue.float(BridgeScalar.expression(expr)));

      setState(() {
        value = sampleScalarWithContext(
            scalar: BridgeScalar_Expression(expr),
            time: timeOfFrame(widget.comp, playhead.value),
            layer: widget.layer);
      });
    }

    lastText = expr;
  }

  void removeExpression() {
    final expr = controller.text;

    var v = sampleScalarWithContext(
        scalar: BridgeScalar_Expression(expr),
        time: timeOfFrame(widget.comp, playhead.value),
        layer: widget.layer);

    widget.set(BridgeEffectValue.float(BridgeScalar_Static(v)));
  }

  @override
  Widget build(BuildContext context) {
    return _build(context);
  }

  Widget _build(BuildContext context) {
    final t = ThemeScope.of(context).theme;

    return Row(
      spacing: 4,
      mainAxisAlignment: MainAxisAlignment.spaceBetween,
      children: [
        Flexible(
            child: HouseContextMenu(
          itemBuilder: (close) {
            return [
              MenuRow(
                onPressed: () {
                  removeExpression();
                  close();
                },
                child: Text(l10n.removeExpression),
              )
            ];
          },
          child: HouseTextField(
            controller: controller,
            width: double.infinity,
            style: t.mono,
            submitOnLostFocus: true,
            autofill: ExpressionAutofillGenerator(),
            onSubmitted: (value) {
              widget
                  .set(BridgeEffectValue.float(BridgeScalar_Expression(value)));
              onTextChanged();
            },
          ),
        )),
        SizedBox(
          width: 78,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text(
                " = ",
                style: t.body.copyWith(color: t.textMuted),
              ),
              // Six significant figures can run to twelve characters
              // ("1.00000e+16"), which is wider than this readout: it gives
              // way rather than striping the row.
              Flexible(
                child: Text(
                  value.toStringAsPrecision(6),
                  style: t.mono.copyWith(color: t.textMuted),
                  softWrap: false,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ],
          ),
        )
      ],
    );
  }
}
