// The clips on an Audio timeline track (docs/impl/audio-timeline.md §5).
//
// A converted track holds clips laid end to end, and this draws one row of
// them: a box per clip in the track's label colour, its own wave or spectrogram
// the full height of the box, and over that the name of what it plays. The
// gestures
// live here too - the body slides a clip along its track or carries it to
// another one, either edge trims it - staged in Dart and written once, on
// release, so a drag is one undo step and Escape abandons it having written
// nothing.
//
// An **unconverted** Audio layer is a track of one clip that has not been cut
// yet. It draws no boxes, but its bar takes the same gestures, and the first
// of them converts the layer; that conversion is the panel's to write, so this
// only reports the gesture.
//
// Everything drawn rides in on the read model. The bridge is crossed from a
// released gesture and from the claimed-key picture fetches.

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart'
    show BridgeScalar_Keyframed, BridgeScalar_Static;
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/drag_escape.dart';
import 'audio_timeline_rows_frb.dart';
import 'clip_fades.dart';
import 'graph_maths.dart' show rationalSeconds;
import 'spectral_lane_frb.dart';
import 'timeline_bar_frb.dart' show BarGrab, barGrabAt;
import 'timeline_extras_frb.dart';
import 'timeline_snap.dart';
import 'volume_band_frb.dart';
import 'waveform_frb.dart';

/// How tall a clip's header strip is: the name and the three controls, laid
/// over the top of the clip's picture.
const double audioClipHeader = 16;

/// The narrowest clip that wears its name. Below this a name is a letter and an
/// ellipsis, which says less than the box's own colour already does.
const double audioClipNameMinWidth = 44;

/// The narrowest clip that has room for the three controls beside its name.
const double audioClipHeaderMinWidth = 110;

/// How far the pointer must travel before a press counts as a drag rather than
/// as the click that picks a clip.
const double _clickSlop = 3;

/// Where a clip's picture is anchored, in the clip's own placed seconds.
///
/// Sliding a clip moves the box and the picture together, so the origin holds
/// and the wave rides along. Dragging the **start** edge moves the box over a
/// picture that stays put, so the origin travels with the edge and the wave
/// holds still until the trim commits. The buckets are asked for at this
/// origin and painted from it, which is why it is one function: anchored two
/// ways, the frames the edge uncovered had no bucket to draw and stayed bare.
double audioClipOrigin({
  required double placeStartSeconds,
  required int shiftFrames,
  required double fps,
  required bool trimIn,
}) =>
    placeStartSeconds + (trimIn && fps > 0 ? shiftFrames / fps : 0);

/// Where a clip's own sound begins on the comp's clock, or null when the
/// head hides none of it.
///
/// A head dragged back past the source's first sample plays silence until the
/// sound arrives (`source_in` below zero), and the panel marks that point and
/// snaps the head to it (docs/impl/audio-timeline.md §5). The frame is the
/// clip's **reach**, which the engine already works out for the Sequence
/// lane's ghost rather than this counting it a second way; it is null on a
/// retimed clip and on a source whose length would not read, and null again
/// once the reach has caught up with the head, which is every clip trimmed the
/// ordinary way.
int? audioSourceStartFrame({
  required int startFrame,
  required int? reachStartFrame,
}) =>
    reachStartFrame != null && reachStartFrame > startFrame
        ? reachStartFrame
        : null;

/// The mark at that point: six pixels across, four deep, pointing down at the
/// frame the sound starts on.
const double _sourceMarkWidth = 6;
const double _sourceMarkHeight = 4;

/// How big the header's three glyphs are drawn. Smaller than the outline's
/// [iconSize], because the marks share the strip with a name.
const double _headerIcon = 10;

/// Which track the point [y] lands on, measured down from the top of the first
/// track's block.
///
/// The index of the track whose block holds it, or null when it holds none:
/// above the first track, below the last, or inside a **faded picture row**,
/// which takes no pointer and so is no landing place for a clip or a file.
int? audioTrackAt(List<AudioTrackRow> tracks, double y) {
  if (y < 0) return null;
  var top = 0.0;
  for (var i = 0; i < tracks.length; i++) {
    final bottom = top + tracks[i].height;
    if (y < bottom) return tracks[i].dimmed ? null : i;
    top = bottom;
  }
  return null;
}

/// Whether [y] is past the bottom of the last track: the empty ground where a
/// clip dragged off the table goes on to a track of its own.
bool audioBelowTracks(List<AudioTrackRow> tracks, double y) =>
    y >= tracks.fold<double>(0, (sum, track) => sum + track.height);

/// Which clip a press at [x] takes hold of, and what of it: the strip's own
/// pixels in, and the clip with the part of it the press has hold of out, or
/// null on empty ground.
///
/// **Every clip's edges are hit before any clip's body**
/// (docs/impl/audio-timeline.md §5), which is what keeps the earlier clip's
/// tail reachable under the later clip of a crossfade: each box carries its own
/// pointer, and the topmost one was taking the whole press, so the join under it
/// could not be moved at all. Both passes run from the top of the stack down,
/// which is the order the boxes are drawn in.
({BridgeClip clip, BarGrab grab})? audioClipGrabAt(
    List<BridgeClip> clips, TimelineAxis axis, double x) {
  (double, double) box(BridgeClip clip) =>
      audioClipBox(axis, clip.startFrame.toInt(), clip.endFrame.toInt());
  for (final clip in clips.reversed) {
    final (left, width) = box(clip);
    if (x < left || x > left + width) continue;
    final grab = barGrabAt(x - left, width);
    if (grab != BarGrab.move) return (clip: clip, grab: grab);
  }
  for (final clip in clips.reversed) {
    final (left, width) = box(clip);
    if (x >= left && x <= left + width) return (clip: clip, grab: BarGrab.move);
  }
  return null;
}

/// Where a box covering [start] to [end] comp frames begins and how wide it is
/// drawn. Never under two pixels, so a clip of a single frame is still
/// something the pointer can find.
(double, double) audioClipBox(TimelineAxis axis, num start, num end) {
  final left = axis.xOf(start);
  return (left, (axis.xOf(end) - left).clamp(2.0, double.infinity));
}

/// A clip's level as its chip says it, signed and in dB: the same words in
/// the Audio timeline and on a Cut sound clip.
String clipGainWords(double gainDb) =>
    '${gainDb > 0 ? '+' : ''}${gainDb.toStringAsFixed(1)} ${l10n.unitSymbolDb}';

/// Where a clip's gain line lies, in the strip's own pixels: 0 dB at the top of
/// the box, silence at its foot, on the Volume band's own floor and its linear
/// shape (docs/impl/audio-timeline.md §5). A box is inset two pixels top and
/// bottom, which is the four this takes off [stripHeight] and the two it puts
/// back.
double audioClipGainY(double gainDb, double stripHeight) =>
    volumeBandY(gainDb, math.max(3.0, stripHeight - 4), topDb: 0) + 2;

/// How tall the line's hit band is, and how thick the line drawn down the
/// middle of it. A few pixels either side is enough to take hold of, and every
/// one of them is a pixel the box's own drag cannot have.
const double _gainHit = 7;
const double _gainStroke = 1.4;

/// Whether a track can hold clips at all.
///
/// It already does, or it is a row of sound that could become the one clip it
/// has always been - which is the conversion the first clip gesture on it
/// makes. A Precomp is not such a row, and neither is a **retimed** layer: a
/// retimed clip is silent, so the engine refuses to convert one (docs/09 §7).
bool audioTrackTakesClips(BridgeLayerInfo info) =>
    info.clips.isNotEmpty ||
    (info.retime == null &&
        (info.kind == BridgeLayerKind.footage ||
            info.kind == BridgeLayerKind.audio));

/// A clip gesture that has been let go, for the panel to write.
///
/// [clip] is null on an unconverted Audio layer, whose bar is its one clip
/// before the conversion has been made. [shift] is the travel in whole frames
/// with the magnet already applied, and [at] is where the pointer let go, which
/// is what says whether the clip crossed on to another track. [sub] is the
/// part of a frame a move carries past [shift]: where the pointer is while
/// sound is free of the frame grid, back on to a frame while it is not.
/// [stretch] is an end dragged with Shift held, which changes the clip's speed
/// rather than its trim.
typedef AudioClipEdit = ({
  BridgeClip? clip,
  BarGrab grab,
  int shift,
  double sub,
  Offset at,
  bool stretch,
});

/// A clip drag in flight, as the fade layer stacked over the strip reads it:
/// the clip by id, what the drag has hold of, and how far it has travelled in
/// whole frames. Null between gestures, and [clip] is null on an unconverted
/// Audio layer, whose bar is its one clip.
typedef AudioClipDrag = ({String? clip, BarGrab grab, int shift});

/// A gesture in flight: what it has hold of, where it began, where the pointer
/// is now, and whether it has travelled far enough to be a drag at all.
typedef _ClipDrag = ({
  BridgeClip? clip,
  BarGrab grab,
  Offset from,
  Offset at,
  bool moved,
  bool stretch,
});

/// One track's clips, over the two lane rows the track is tall.
class AudioClipStrip extends StatefulWidget {
  final AudioTrackRow track;
  final TimelineAxis axis;
  final double fps;
  final double height;

  /// Which picture a clip draws, from the track's own lane-mode chip.
  final LaneMode mode;
  final WaveformStyle style;

  /// The lanes' horizontal scroll, so a clip asks for the summary of the part
  /// of it that is actually on screen rather than of the whole of it.
  final ScrollController? hScroll;

  final List<SnapTarget> snapTargets;

  /// Whether a moved clip lands on a whole frame. Off, it goes where the
  /// pointer puts it.
  final bool quantise;

  /// Where the box stands while a drag is on, for the fade layer the panel
  /// stacks over this strip. They are siblings, so without it the ramps and the
  /// corner handles hold the committed frames while the box moves out from
  /// under them (docs/impl/audio-timeline.md §5).
  final ValueNotifier<AudioClipDrag?> preview;

  /// The picked clips' ids, drawn lit. The table's own list, so a pick reaching
  /// across tracks reads the same on each.
  final Set<String> selected;

  /// Which clips on this track are twirled open, by id - the drop-down rows
  /// are the track's to draw, so all this changes here is which way the
  /// header's twirl points.
  final Set<String> openClips;

  /// The razor, and where a cut at screen x lands - the same function the
  /// blade's line is drawn with, so the cut is where the mark is.
  final bool razor;
  final void Function(int frame) onRazor;
  final double Function(double x) razorFrameAt;

  final void Function(BridgeClip clip) onSelect;
  /// A right click on a clip, or on an unconverted layer's bar with a null
  /// clip.
  final void Function(BridgeClip? clip, Offset at) onMenu;

  /// A right click that landed on a fade, or inside an overlap: the fade menu
  /// rather than the clip menu, aimed at that end of the clip.
  final void Function(BridgeClip clip, bool into, Offset at) onFadeMenu;
  final void Function(AudioClipEdit edit) onCommit;

  /// Committed something from the header strip; the panel refreshes.
  final VoidCallback onChanged;

  /// The header's three controls. Null draws them without a gesture, which is
  /// where they stand on a panel that has nothing to wire them to.
  ///
  /// Each is handed the **button's own** context, because the add-effect menu
  /// drops from the control it was pressed on rather than from the corner of
  /// the panel; the other two are given it too, so the three read alike.
  final void Function(BridgeClip clip, BuildContext at)? onToggleFx;
  final void Function(BridgeClip clip, BuildContext at)? onAddEffect;
  final void Function(BridgeClip clip, BuildContext at)? onToggleOpen;

  const AudioClipStrip({
    super.key,
    required this.track,
    required this.axis,
    required this.fps,
    required this.height,
    required this.mode,
    required this.style,
    required this.hScroll,
    required this.snapTargets,
    this.quantise = true,
    required this.preview,
    required this.selected,
    this.openClips = const {},
    required this.razor,
    required this.onRazor,
    required this.razorFrameAt,
    required this.onSelect,
    required this.onMenu,
    required this.onFadeMenu,
    required this.onCommit,
    required this.onChanged,
    this.onToggleFx,
    this.onAddEffect,
    this.onToggleOpen,
  });

  @override
  State<AudioClipStrip> createState() => _AudioClipStripState();
}

class _AudioClipStripState extends State<AudioClipStrip> {
  /// The gesture in flight: which clip it has hold of, what of it, where it
  /// started and where the pointer is now. Null between gestures.
  _ClipDrag? _drag;

  /// Escape's way out of the drag, which writes nothing.
  final DragEscape _escape = DragEscape();

  /// Whether the press that is running began on one of the header's controls.
  ///
  /// The header stands over the box, so a press on it hits the box's own
  /// `Listener` as well - a `Listener` watches every pointer that reaches it
  /// and cannot be won away from. Without this a tap on the fx toggle also
  /// picked the clip, and with the razor in hand it also cut it. The header is
  /// hit first, being on top, so the flag is set before the box sees anything.
  bool _pressedControl = false;

  /// The clip the pointer is on, for what waits for it: the header's controls,
  /// and a gain line still at its usual level. Its own notifier, so the pointer
  /// crossing a clip rebuilds those and not the box or its wave.
  final ValueNotifier<String?> _over = ValueNotifier<String?>(null);

  /// Each clip's picture over the stretch of it that is on screen, and what
  /// each was fetched for. Equal keys mean the answer in hand still fits, so a
  /// rebuild asks nothing; the key is claimed before the fetch starts, so a
  /// rebuild mid-decode does not ask twice.
  final Map<String, BridgeAudioPeaks> _peaks = {};
  final Map<String, BridgeSpectrogram> _spectra = {};
  final Map<String, String> _keys = {};

  /// The gain drag in flight: whose line it holds, the dB it has reached, the
  /// dB it took hold at and how far it has travelled. Travel in, travel out, so
  /// the line never jumps to the pointer.
  ({String clip, double db, double grabbedAt, double travelled})? _gain;

  @override
  void dispose() {
    _escape.dispose();
    _over.dispose();
    super.dispose();
  }

  List<BridgeClip> get _clips => widget.track.entry.info.clips;

  /// The comp frames a clip covers as it is drawn right now: the document's,
  /// plus whatever the drag in flight has done to them.
  (int, int) _span(BridgeClip clip) {
    final drag = _drag;
    final moving = drag != null && drag.clip?.id == clip.id;
    final shift = moving ? _shiftOf(drag) : 0;
    return (
      clip.startFrame.toInt() +
          (moving && drag.grab != BarGrab.trimOut ? shift : 0),
      clip.endFrame.toInt() +
          (moving && drag.grab != BarGrab.trimIn ? shift : 0),
    );
  }

  /// How far a drag has travelled in whole frames, with the magnet applied.
  int _shiftOf(_ClipDrag drag) => _travelOf(drag).$1;

  /// The part of a frame a move carries past [_shiftOf].
  double _subOf(_ClipDrag drag) => _travelOf(drag).$2;

  /// A drag's travel: whole frames, and for a move the part of a frame past
  /// them.
  ///
  /// Taken afresh from the raw pixel travel every time it is asked for, never
  /// from the snapped answer, so an edge caught on a target can still be pulled
  /// off it. `Ctrl` held suspends the snap; this panel's bar carries no magnet.
  ///
  /// A move with [AudioClipStrip.quantise] off goes where the pointer is,
  /// unless it has caught on a target. With it on, or caught, a clip that sat
  /// between two frames lands on one.
  (int, double) _travelOf(_ClipDrag drag) {
    final info = widget.track.entry.info;
    final from = drag.clip?.startFrame.toInt() ?? info.inFrame.toInt();
    final to = drag.clip?.endFrame.toInt() ?? info.outFrame.toInt();
    // Where the sound itself begins and ends. The head lands on the one and the
    // tail on the other whichever way the edge is moving, so an end taken in
    // comes back out to the sound's own edge. A reach standing where the
    // dragged edge already is stays out of the list: a target at no travel at
    // all pins the drag where it started.
    final edge = drag.grab == BarGrab.trimOut ? to : from;
    final reach = switch (drag.grab) {
      BarGrab.trimIn => drag.clip?.reachStartFrame?.toInt(),
      BarGrab.trimOut => drag.clip?.reachEndFrame?.toInt(),
      BarGrab.move => null,
    };
    final sound = reach == edge ? null : reach;
    final raw = widget.axis.framesOfPx(drag.at.dx - drag.from.dx);
    final snapped = snappedDelta(
      rawFrames: raw,
      perFrame: widget.axis.perFrame,
      sources: switch (drag.grab) {
        BarGrab.move => [from.toDouble(), to.toDouble()],
        BarGrab.trimIn => [from.toDouble()],
        BarGrab.trimOut => [to.toDouble()],
      },
      targets: [
        // The dragged clip's own ends are dropped: a target standing where a
        // source already is pins the drag where it started.
        ...widget.snapTargets
            .where((target) => target.frame != from && target.frame != to),
        if (sound != null) SnapTarget(sound.toDouble(), SnapKind.editPoint),
      ],
      magnet: !snapSuspended(
          controlPressed: HardwareKeyboard.instance.isControlPressed),
    );
    if (drag.grab != BarGrab.move) return (snapped.delta, 0);
    // An unconverted Audio layer's bar is its one clip, and moves the same.
    if (!widget.quantise && snapped.caught == null) {
      return (raw.floor(), raw - raw.floor());
    }
    final clip = drag.clip;
    return (
      snapped.delta,
      clip == null ? 0 : -(clip.startExact - clip.startFrame.toInt()),
    );
  }

  // ------------------------------------------------------------ the gestures

  /// The pointer went down on a clip's box, or on an unconverted layer's bar.
  ///
  /// **A raw `Listener`, never a drag recogniser.** A clip sits inside two
  /// scroll views, and a recogniser that wants the horizontal axis is in the
  /// arena against both of them - which win, so the first drag died the moment
  /// it had travelled far enough for them to claim it. A `Listener` is not in
  /// the arena at all, and it also means the press is claimed before the
  /// marquee behind it sees anything.
  void _down(PointerDownEvent event, BridgeClip? clip, double width) {
    if (event.buttons == kSecondaryMouseButton) {
      if (clip == null) {
        widget.onMenu(null, event.position);
        return;
      }
      // A right click on a fade, or anywhere in an overlap, is about the fade
      // and not about the clip.
      final side = clipFadeSideAt(_clips, clip,
          widget.axis.frameAtExact(_localX(event.position)), widget.fps);
      if (side == null) {
        widget.onMenu(clip, event.position);
      } else {
        widget.onFadeMenu(clip, side, event.position);
      }
      return;
    }
    // What the press is really on. The box under the pointer is not always the
    // clip that answers: inside an overlap the earlier clip's tail is hit
    // first, because an edge is hit before a body.
    final hit = clip == null
        ? null
        : audioClipGrabAt(_clips, widget.axis, _localX(event.position));
    final grab = hit?.grab ?? barGrabAt(event.localPosition.dx, width);
    setState(() => _drag = (
          clip: hit?.clip ?? clip,
          grab: grab,
          from: event.position,
          at: event.position,
          moved: false,
          // Shift on an end changes the clip's speed instead of its trim.
          stretch: grab != BarGrab.move &&
              HardwareKeyboard.instance.isShiftPressed,
        ));
    _escape.begin(() {
      setState(() => _drag = null);
      _publish();
    });
    _publish();
  }

  /// Tell the fade layer stacked over this strip where the box stands now.
  void _publish() {
    final held = _drag;
    widget.preview.value = held == null
        ? null
        : (
            clip: held.clip?.id.toString(),
            grab: held.grab,
            shift: _shiftOf(held),
          );
  }

  /// A pointer taken away - the gesture never happened, and neither did the
  /// control press that may have started it. The claim on Escape comes off
  /// with it, or the next Escape is taken by a drag that is not in flight and
  /// never reaches the picked clip.
  void _cancel() {
    _escape.end();
    _pressedControl = false;
    setState(() => _drag = null);
    _publish();
  }

  void _move(PointerMoveEvent event) {
    final held = _drag;
    if (held == null || !_escape.running) return;
    setState(() => _drag = (
          clip: held.clip,
          grab: held.grab,
          from: held.from,
          at: event.position,
          moved:
              held.moved || (event.position - held.from).distance > _clickSlop,
          stretch: held.stretch,
        ));
    _publish();
  }

  void _up(PointerUpEvent event) {
    final held = _drag;
    final commit = _escape.end();
    final onControl = _pressedControl;
    _pressedControl = false;
    setState(() => _drag = null);
    _publish();
    if (held == null || !commit) return;
    // The header's own control answered this press; the clip is not also
    // picked by it, and the razor does not also cut through it.
    if (onControl) return;
    if (!held.moved) {
      // A press that never travelled is a click: cut it with the razor in
      // hand, and pick it otherwise.
      if (widget.razor) {
        widget.onRazor(widget.razorFrameAt(_localX(event.position)).round());
      } else if (held.clip case final clip?) {
        widget.onSelect(clip);
      }
      return;
    }
    final shift = _shiftOf(held);
    // A trim that moved nothing is not an edit. A move that moved no frames
    // still is: it may have crossed on to another track.
    if (shift == 0 && held.grab != BarGrab.move) return;
    widget.onCommit((
      clip: held.clip,
      grab: held.grab,
      shift: shift,
      sub: _subOf(held),
      at: held.at,
      stretch: held.stretch,
    ));
  }

  // ---------------------------------------------------------------- the gain

  /// The pointer went down on a clip's gain line.
  ///
  /// A raw `Listener` over the box's own, as the box is one over the marquee: a
  /// `Listener` cannot be won away from once it holds the pointer, so the press
  /// is the line's and the box does not also start sliding under it.
  void _gainDown(PointerDownEvent event, BridgeClip clip, double width) {
    // The line takes every press that lands on it, so a right click has to be
    // handed back: it is about the clip and not about its level.
    if (event.buttons != kPrimaryMouseButton) {
      _down(event, clip, width);
      return;
    }
    setState(() => _gain = (
          clip: clip.id.toString(),
          db: clip.gainDb,
          grabbedAt: clip.gainDb,
          travelled: 0,
        ));
    _escape.begin(() => setState(() => _gain = null));
  }

  /// The band's own arithmetic read from 0 dB down: the dB the drag took hold
  /// of, less its travel over the height of the box.
  void _gainMove(double dy) {
    final held = _gain;
    if (held == null || !_escape.running) return;
    final travelled = held.travelled + dy;
    final height = math.max(3.0, widget.height - 4);
    final db = (held.grabbedAt - travelled / (height - 2) * -volumeBandFloorDb)
        .clamp(volumeBandFloorDb, 0.0);
    setState(() => _gain = (
          clip: held.clip,
          db: db,
          grabbedAt: held.grabbedAt,
          travelled: travelled,
        ));
  }

  /// Let go: the gain is written once, and only where the drag moved it.
  void _gainUp(BridgeClip clip) {
    final held = _gain;
    final commit = _escape.end();
    setState(() => _gain = null);
    if (held == null || !commit || held.db == held.grabbedAt) return;
    widget.track.entry.layer.setClipGain(clip: clip.id, db: held.db);
    widget.onChanged();
  }

  /// This clip's picture origin, with the drag in flight in it.
  double _originOf(BridgeClip clip) {
    final drag = _drag;
    final trimIn =
        drag != null && drag.clip?.id == clip.id && drag.grab == BarGrab.trimIn;
    return audioClipOrigin(
      placeStartSeconds: rationalSeconds(clip.placeStart),
      shiftFrames: drag != null && trimIn ? _shiftOf(drag) : 0,
      fps: widget.fps,
      trimIn: trimIn,
    );
  }

  /// Where this clip's sound starts as the clip is drawn right now.
  ///
  /// A whole-clip move carries the source along with the box. A trim moves the
  /// box over material that stays where it is, so the mark keeps its comp
  /// frame while the head is dragged over it, which is the point of it.
  int? _sourceStartOf(BridgeClip clip) {
    final drag = _drag;
    final moving = drag != null && drag.clip?.id == clip.id;
    final shift = moving && drag.grab == BarGrab.move ? _shiftOf(drag) : 0;
    final reach = clip.reachStartFrame?.toInt();
    return audioSourceStartFrame(
      startFrame: _span(clip).$1,
      reachStartFrame: reach == null ? null : reach + shift,
    );
  }

  /// A global x in this strip's own pixels, which is where the axis measures.
  double _localX(Offset global) {
    final box = context.findRenderObject();
    return box is RenderBox ? box.globalToLocal(global).dx : global.dx;
  }

  // ------------------------------------------------------------- the picture

  /// Ask for the summary of the part of [clip] that is on screen, at one bucket
  /// per pixel column of it, so a clip's picture gains detail as the lanes zoom
  /// in rather than stretching the one it was given first.
  void _wantPicture(BridgeClip clip, double left, double width) {
    final id = clip.id.toString();
    // A clip whose sound was detached is heard from the clip that took it,
    // and draws no wave of its own. A hold plays silence.
    if (!clip.audible || clipHolds(clip)) {
      _peaks.remove(id);
      _spectra.remove(id);
      _keys.remove(id);
      return;
    }
    final axis = widget.axis;
    if (axis.perFrame <= 0 || widget.fps <= 0 || width <= 0) return;
    final secondsPerPixel = 1 / (axis.perFrame * widget.fps);
    final scroll = widget.hScroll;
    final viewLeft = scroll != null && scroll.hasClients ? scroll.offset : 0.0;
    final viewWidth = scroll != null && scroll.hasClients
        ? scroll.position.viewportDimension
        : axis.width;
    final from = math.max(left, viewLeft);
    final to = math.min(left + width, viewLeft + viewWidth);
    if (!(to > from)) return;
    // A clip's own placed clock, which is the clock both engine calls bucket
    // in, and the clock the picture is painted from.
    final localStart = _originOf(clip);
    final request = WaveformRequest.forView(
      startSeconds: localStart + (from - left) * secondsPerPixel,
      endSeconds: localStart + (to - left) * secondsPerPixel,
      pixels: to - from,
    );
    if (request == null) return;
    // The trim and the map are part of the key: both change which source
    // moments the buckets stand for, and neither moves the clip's box.
    final key = '${widget.mode.name}|${request.key}|${clip.startFrame}'
        '|${clip.endFrame}|${clip.retimed}|${widget.style.needsBands}';
    if (_keys[id] == key) return;
    _keys[id] = key;
    final layer = widget.track.entry.layer;
    if (widget.mode == LaneMode.spectral) {
      _peaks.remove(id);
      layer
          .clipAudioSpectrogram(
        clip: clip.id,
        startSeconds: request.startSeconds,
        endSeconds: request.endSeconds,
        columns: request.buckets,
      )
          .then((grid) {
        if (!mounted || _keys[id] != key) return;
        setState(() => _spectra[id] = grid);
      });
      return;
    }
    _spectra.remove(id);
    layer
        .clipAudioPeaks(
      clip: clip.id,
      startSeconds: request.startSeconds,
      endSeconds: request.endSeconds,
      buckets: request.buckets,
      multiwave: widget.style.needsBands,
      stereo: true,
    )
        .then((peaks) {
      if (!mounted || _keys[id] != key) return;
      setState(() => _peaks[id] = peaks);
    });
  }

  // ---------------------------------------------------------------- the draw

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final info = widget.track.entry.info;
    if (_clips.isEmpty) {
      // An unconverted row: no boxes, but its bar takes the gestures, and the
      // first of them converts it - so a row that cannot be converted offers
      // no clip gesture at all.
      if (!audioTrackTakesClips(info)) return const SizedBox.shrink();
      final left = widget.axis.xOf(info.inFrame.toInt());
      final width =
          math.max(0.0, widget.axis.xOf(info.outFrame.toInt()) - left);
      return Stack(children: [
        Positioned(
          key: ValueKey<String>('atl-bar-grab-${widget.track.id}'),
          left: left,
          width: width,
          top: 0,
          bottom: 0,
          child: MouseRegion(
            // The razor's own blade is drawn over the lanes, and the innermost
            // cursor is the one shown - so this stands down while it is armed.
            cursor: _cursor(),
            onHover: (e) => _hoverAt(barGrabAt(e.localPosition.dx, width)),
            onExit: (_) => _hoverAt(null),
            child: Listener(
              behavior: HitTestBehavior.opaque,
              onPointerDown: (e) => _down(e, null, width),
              onPointerMove: _move,
              onPointerUp: _up,
              onPointerCancel: (_) => _cancel(),
            ),
          ),
        ),
      ]);
    }
    final drag = _drag;
    return Stack(children: [
      for (final clip in _clips)
        if (drag != null && drag.moved && drag.clip?.id == clip.id)
          _ghost(t, clip),
      for (final clip in _clips) _clip(t, clip),
      // The headers last, so their controls stand over the box's own pointer
      // and over the gain line drawn on it.
      for (final clip in _clips)
        if (_widthOf(clip) >= audioClipNameMinWidth) _headerBox(t, clip),
    ]);
  }

  /// What the pointer would take hold of where it is resting.
  BarGrab? _hover;

  void _hoverAt(BarGrab? grab) {
    if (grab != _hover) setState(() => _hover = grab);
  }

  /// A hand over the body, as a layer bar shows one, and the trim arrows only
  /// at the ends that trim.
  MouseCursor _cursor() {
    if (widget.razor) return SystemMouseCursors.none;
    return switch (_drag?.grab ?? _hover) {
      BarGrab.trimIn || BarGrab.trimOut => SystemMouseCursors.resizeLeftRight,
      _ when _drag != null => SystemMouseCursors.grabbing,
      _ => SystemMouseCursors.grab,
    };
  }

  /// Where a clip's box begins and how wide it is drawn, with whatever the drag
  /// in flight has done to it - the one answer the box and its header share, so
  /// a strip cannot sit anywhere but on its own clip.
  (double, double) _place(BridgeClip clip) {
    final (start, end) = _span(clip);
    // To the part of a frame: where the clip sits, and where a move in hand
    // has carried it.
    final drag = _drag;
    final sub = drag != null && drag.clip?.id == clip.id ? _subOf(drag) : 0.0;
    return audioClipBox(
      widget.axis,
      start + (clip.startExact - clip.startFrame.toInt()) + sub,
      end + (clip.endExact - clip.endFrame.toInt()) + sub,
    );
  }

  double _widthOf(BridgeClip clip) => _place(clip).$2;

  /// Where a clip being dragged came from, while it is being dragged: a
  /// hairline and nothing in it, so the box under the pointer is read as the
  /// clip and this as the place it is leaving.
  Widget _ghost(LumitTheme t, BridgeClip clip) {
    final left = widget.axis.xOf(clip.startExact);
    return Positioned(
      key: ValueKey<String>('atl-clip-ghost-${clip.id}'),
      left: left,
      width: (widget.axis.xOf(clip.endExact) - left)
          .clamp(1.0, double.infinity),
      top: 2,
      bottom: 2,
      child: IgnorePointer(
        child: DecoratedBox(
          decoration: BoxDecoration(
            border: Border.all(
                color: t
                    .labelColour(widget.track.entry.info.label)
                    .withValues(alpha: 0.25)),
          ),
        ),
      ),
    );
  }

  /// One clip's header strip, laid over the band on the clip's own box.
  Widget _headerBox(LumitTheme t, BridgeClip clip) {
    final (left, width) = _place(clip);
    return Positioned(
      key: ValueKey<String>('atl-clip-header-${clip.id}'),
      left: left,
      width: width,
      top: 2,
      height: audioClipHeader,
      child: _header(t, clip, width),
    );
  }

  Widget _clip(LumitTheme t, BridgeClip clip) {
    // The clip's own colour where it has one, its track's where not.
    final label = t.labelColour(clip.label ?? widget.track.entry.info.label);
    final (left, width) = _place(clip);
    // After layout, since a zoom only moves the scroll there and the offset
    // read during build is the last frame's.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _wantPicture(clip, left, width);
    });
    final picked = widget.selected.contains(clip.id.toString());
    return Positioned(
      key: ValueKey<String>('atl-clip-${clip.id}'),
      left: left,
      width: width,
      top: 2,
      bottom: 2,
      child: AnimatedContainer(
        duration: ThemeScope.of(context).motion.mark.duration,
        curve: ThemeScope.of(context).motion.mark.curve,
        decoration: BoxDecoration(
          // The label's own colour thinned over the ground, with the solid
          // leading edge below carrying it at full strength - the layer bar's
          // own treatment, and a picked clip is the same colour held stronger.
          color: label.withValues(
              alpha: picked ? clipFillSelectedAlpha : clipFillAlpha),
          // Desk marks a picked clip with its accent outline, the other
          // shapes with the foreground.
          border: Border.all(
              color: picked
                  ? (t.shape == ThemeShape.desk ? t.accent : t.textPrimary)
                  : t.surface0),
          borderRadius: BorderRadius.circular(clipRadius(t)),
        ),
        // The edge again over the content, which would otherwise cover it where
        // the corner curves.
        foregroundDecoration: BoxDecoration(
          border: Border.all(
              color: picked
                  ? (t.shape == ThemeShape.desk ? t.accent : t.textPrimary)
                  : t.surface0),
          borderRadius: BorderRadius.circular(clipRadius(t)),
        ),
        clipBehavior: Clip.hardEdge,
        child: Stack(children: [
          // The box's own pointer, under the header's controls: a press
          // anywhere else on the clip is a drag or a click on the clip itself.
          Positioned.fill(
            child: MouseRegion(
              // The razor's own blade is drawn over the lanes, and the innermost
              // cursor is the one shown - so this stands down while it is armed.
              cursor: _cursor(),
              onEnter: (_) => _over.value = clip.id.toString(),
              onHover: (e) => _hoverAt(audioClipGrabAt(
                      _clips, widget.axis, _localX(e.position))
                  ?.grab),
              onExit: (_) {
                _hoverAt(null);
                if (_over.value == clip.id.toString()) _over.value = null;
              },
              child: Listener(
                behavior: HitTestBehavior.opaque,
                onPointerDown: (e) => _down(e, clip, width),
                onPointerMove: _move,
                onPointerUp: _up,
                onPointerCancel: (_) => _cancel(),
                child: _content(t, clip, label, width),
              ),
            ),
          ),
          ..._gainParts(t, clip),
        ]),
      ),
    );
  }

  /// The clip's gain line across its box, and the dB it reads while it is
  /// dragged.
  ///
  /// The line is the clip's level the way the band is the track's: 0 dB at the
  /// top of the box, silence at its foot. It is drawn inside the box's own
  /// stack, so a slide or a trim carries it along for nothing. A clip at its
  /// usual level keeps the line back until the pointer is on it.
  List<Widget> _gainParts(LumitTheme t, BridgeClip clip) {
    final id = clip.id.toString();
    final held = _gain?.clip == id ? _gain : null;
    final box = math.max(0.0, widget.height - 4);
    final y = audioClipGainY(held?.db ?? clip.gainDb, widget.height) - 2;
    final usual = held == null && clip.gainDb == 0;
    final line = Padding(
      padding:
          const EdgeInsets.symmetric(vertical: (_gainHit - _gainStroke) / 2),
      child: usual
          ? ValueListenableBuilder<String?>(
              valueListenable: _over,
              builder: (context, over, _) => ColoredBox(
                  color: over == id ? t.accent : t.accent.withValues(alpha: 0)),
            )
          : ColoredBox(color: t.accent),
    );
    return [
      Positioned(
        key: ValueKey<String>('atl-gain-$id'),
        left: 0,
        right: 0,
        top: y - _gainHit / 2,
        height: _gainHit,
        // With the razor in hand the line is a place to cut like any other, so
        // it draws itself and takes nothing.
        child: widget.razor
            ? IgnorePointer(child: line)
            : MouseRegion(
                cursor: SystemMouseCursors.resizeUpDown,
                child: Listener(
                  behavior: HitTestBehavior.opaque,
                  onPointerDown: (e) => _gainDown(e, clip, _widthOf(clip)),
                  onPointerMove: (e) => _gainMove(e.delta.dy),
                  onPointerUp: (_) => _gainUp(clip),
                  onPointerCancel: (_) {
                    _escape.end();
                    setState(() => _gain = null);
                  },
                  child: line,
                ),
              ),
      ),
      if (held != null)
        Positioned(
          key: ValueKey<String>('atl-gain-readout-$id'),
          right: 4,
          top: (y + 2).clamp(0.0, math.max(0.0, box - 10)),
          child: IgnorePointer(
            child: Text(
              held.db <= volumeBandFloorDb
                  ? l10n.volumeNegInf
                  : '${held.db.toStringAsFixed(1)} dB',
              style: t.mono.copyWith(fontSize: 9, color: t.textPrimary),
            ),
          ),
        ),
    ];
  }

  /// The clip's leading edge and its own picture, behind the header.
  Widget _content(LumitTheme t, BridgeClip clip, Color label, double width) {
    final id = clip.id.toString();
    final originSeconds = _originOf(clip);
    final secondsPerPixel = widget.axis.perFrame <= 0 || widget.fps <= 0
        ? 0.0
        : 1 / (widget.axis.perFrame * widget.fps);
    // The whole box: a name laid over the wave costs the wave nothing, and a
    // short track has none of its height to spare.
    final height = math.max(1.0, widget.height - 4);
    // The box's own left, which the mark below is measured back from: it is a
    // comp frame and this is drawn in the box's pixels.
    final boxLeft = _place(clip).$1;
    return Stack(children: [
      Positioned(
        key: ValueKey<String>('atl-clip-edge-${clip.id}'),
        left: 0,
        top: 0,
        bottom: 0,
        width: clipEdgeWidth,
        child: ColoredBox(color: label),
      ),
      Positioned(
        left: 0,
        right: 0,
        top: 0,
        bottom: 0,
        child: widget.mode == LaneMode.spectral
            ? SpectralLane(
                key: ValueKey<String>('atl-clip-spectral-${clip.id}'),
                grid: _spectra[id],
                originSeconds: originSeconds,
                secondsPerPixel: secondsPerPixel,
                left: 0,
                right: width,
                height: height,
              )
            : CustomPaint(
                key: ValueKey<String>('atl-clip-wave-${clip.id}'),
                painter: WaveformPainter(
                  peaks: _peaks[id],
                  originSeconds: originSeconds,
                  secondsPerPixel: secondsPerPixel,
                  left: 0,
                  right: width,
                  colours: clipWaveColours(label, t.textPrimary),
                  style: widget.style,
                  height: height,
                ),
              ),
      ),
      // The box clips this, so a sound starting past the tail draws nothing.
      if (_sourceStartOf(clip) case final frame?)
        Positioned(
          key: ValueKey<String>('atl-source-start-${clip.id}'),
          left: widget.axis.xOf(frame) - boxLeft - _sourceMarkWidth / 2,
          top: 0,
          width: _sourceMarkWidth,
          height: _sourceMarkHeight,
          child: CustomPaint(painter: _SourceStartMark(t.textPrimary)),
        ),
    ]);
  }

  /// The header strip: what the clip plays, on a shade that lets it read over
  /// the wave, and the three controls beside it. Left, not right: an overlap
  /// draws the incoming clip over the outgoing one's end, and controls that
  /// stood there were covered.
  ///
  /// The controls wait for the pointer, unless one has something to say: a
  /// bypassed stack, or a clip twirled open. They still take a press while they
  /// wait, which needs a pointer on the clip, and that is what brings them up.
  Widget _header(LumitTheme t, BridgeClip clip, double width) {
    final open = widget.openClips.contains(clip.id.toString());
    final bypassed = !clip.fx;
    final room = width >= audioClipHeaderMinWidth;
    // On a plate of the ground, because a muted glyph is lost on a busy wave.
    Widget waits(bool always, Widget control) => ValueListenableBuilder<String?>(
          valueListenable: _over,
          builder: (context, over, child) => Opacity(
            opacity: always || over == clip.id.toString() ? 1 : 0,
            child: ColoredBox(
                color: t.surface0.withValues(alpha: 0.7), child: child),
          ),
          child: control,
        );
    return IgnorePointer(
      ignoring: !room,
      child: Stack(children: [
        Positioned.fill(
          child: IgnorePointer(
            child: DecoratedBox(
              decoration: BoxDecoration(
                gradient: LinearGradient(
                  begin: Alignment.topCenter,
                  end: Alignment.bottomCenter,
                  colors: [
                    t.surface0.withValues(alpha: 0.55),
                    t.surface0.withValues(alpha: 0),
                  ],
                ),
                borderRadius: BorderRadius.vertical(
                    top: Radius.circular(clipRadius(t))),
              ),
            ),
          ),
        ),
        Padding(
          padding: const EdgeInsets.only(left: clipEdgeWidth + 3, right: 2),
          child: Row(children: [
            if (clipHolds(clip)) ...[
              lumitIcon(LumitIcon.pause, size: _headerIcon, color: t.textPrimary),
              const SizedBox(width: 2),
            ],
            // The press goes through the name to the box, so a clip can be
            // taken hold of by it.
            Flexible(
              child: IgnorePointer(
                child: Text(
                  clip.sourceName,
                  key: ValueKey<String>('atl-clip-name-${clip.id}'),
                  style: t.small.copyWith(color: t.textPrimary),
                  maxLines: 1,
                  softWrap: false,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ),
            // The clip's own level, while it is off 0 dB: a normalised clip
            // looks like the others otherwise.
            if (clip.gainDb.abs() >= 0.05) ...[
              const SizedBox(width: 4),
              IgnorePointer(
                child: Container(
                  key: ValueKey<String>('atl-clip-gain-${clip.id}'),
                  padding: const EdgeInsets.symmetric(horizontal: 3),
                  decoration: BoxDecoration(
                    color: t.surface0.withValues(alpha: 0.7),
                    borderRadius: BorderRadius.circular(clipRadius(t)),
                  ),
                  child: Text(clipGainWords(clip.gainDb),
                      style: t.mono.copyWith(
                          fontSize: barValueTextSize,
                          color: t.textSecondary)),
                ),
              ),
            ],
            if (room) ...[
              const SizedBox(width: 4),
              waits(
                bypassed,
                _control(
                  t,
                  id: 'fx',
                  clip: clip,
                  tip: clip.fx
                      ? l10n.switchEffectsOn
                      : l10n.switchEffectsBypassed,
                  on: bypassed,
                  icon: (ink) =>
                      lumitIcon(LumitIcon.fx, size: _headerIcon, color: ink),
                  onPressed: widget.onToggleFx,
                ),
              ),
              waits(
                false,
                _control(
                  t,
                  id: 'add-effect',
                  clip: clip,
                  tip: l10n.addEffect,
                  on: false,
                  icon: (ink) => glyph.LumitIcon(LumitIcons.addEffect,
                      size: _headerIcon, colour: ink),
                  onPressed: widget.onAddEffect,
                ),
              ),
              waits(
                open,
                _control(
                  t,
                  id: 'twirl',
                  clip: clip,
                  tip: open ? l10n.tipHideProperties : l10n.tipProperties,
                  on: open,
                  icon: (ink) => TwirlTurn(
                    open: open,
                    child: glyph.LumitIcon(
                        open ? LumitIcons.collapse : LumitIcons.expand,
                        size: _headerIcon,
                        colour: ink),
                  ),
                  onPressed: widget.onToggleOpen,
                ),
              ),
            ],
            const Spacer(),
          ]),
        ),
      ]),
    );
  }

  /// One of the header's three controls.
  ///
  /// With no [onPressed] the glyph is drawn and takes no pointer, which is
  /// where a control stands on a panel that has nothing to wire it to. The
  /// builder is what gives the press the button's own context, which is what
  /// the add-effect menu drops from.
  Widget _control(
    LumitTheme t, {
    required String id,
    required BridgeClip clip,
    required String tip,
    required bool on,
    required Widget Function(Color ink) icon,
    required void Function(BridgeClip clip, BuildContext at)? onPressed,
  }) {
    Widget cell(double lit) => SizedBox(
        width: audioClipHeader,
        height: audioClipHeader,
        child: Center(
            child: icon(on
                ? t.textPrimary
                : Color.lerp(t.textMuted, t.textPrimary, lit)!)));
    if (onPressed == null) return IgnorePointer(child: cell(0));
    return LumitTooltip(
      message: tip,
      child: HoverMark(
        builder: (buttonContext, lit) => Listener(
          // Says the press was the control's, before the box beneath sees it.
          onPointerDown: (_) => _pressedControl = true,
          child: GestureDetector(
            key: ValueKey<String>('atl-clip-$id-${clip.id}'),
            behavior: HitTestBehavior.opaque,
            onTap: () => onPressed(clip, buttonContext),
            onSecondaryTapUp: (d) => widget.onMenu(clip, d.globalPosition),
            child: cell(lit),
          ),
        ),
      ),
    );
  }
}

/// The mark at the source's start: a filled triangle pointing down at the
/// frame the sound begins on.
class _SourceStartMark extends CustomPainter {
  final Color colour;

  const _SourceStartMark(this.colour);

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawPath(
      Path()
        ..moveTo(0, 0)
        ..lineTo(size.width, 0)
        ..lineTo(size.width / 2, size.height)
        ..close(),
      Paint()..color = colour,
    );
  }

  @override
  bool shouldRepaint(_SourceStartMark old) => old.colour != colour;
}

/// Whether [clip] holds one frame: its source time stands still from end to
/// end, as Freeze frame leaves it.
bool clipHolds(BridgeClip clip) => switch (clip.retime) {
      BridgeScalar_Keyframed(:final field0) when clip.retimed =>
        field0.length > 1 &&
            field0.every((k) => (k.value - field0.first.value).abs() < 1e-9),
      BridgeScalar_Static() => clip.retimed,
      _ => false,
    };
