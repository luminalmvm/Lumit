// The Cut timeline panel: the fronted comp arranged for cutting long footage.
//
// Two halves over one time axis, as the Audio timeline stands - the track
// headers down the left, the lanes on the right - with the strip across the
// top carrying the clock, the tools and the snap. Every track is a layer of
// the comp: the picture rows above a heavier line, the audio-only rows below
// it. The clips are painted, not built, and only the ones in view
// (cut_timeline_lane.dart); the arithmetic over them is pure
// (cut_timeline_tracks.dart).
//
// Everything drawn rides in on the read model. The bridge is crossed from a
// released gesture and from the picture fetches for the visible clips alone.

import 'dart:math';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart' show setEquals;
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_svg/flutter_svg.dart' show SvgStringLoader, SvgTheme, vg;
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/cut.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeRational;
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart' show UuidValue;

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show exportFrb;
import '../state/comp_model.dart';
import '../state/comp_time.dart';
import '../state/dock.dart';
import '../state/drag_payloads.dart';
import '../state/keymap.dart';
import '../state/timecode.dart';
import '../state/timeline_columns.dart';
import '../state/tools.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/drag_escape.dart';
import '../widgets/escape_ladder.dart';
import '../widgets/marquee.dart' show MarqueeBox;
import '../widgets/smooth_zoom.dart';
import '../widgets/time_readout.dart';
import '../widgets/zoom_anchored_scroll.dart';
import 'cut_timeline_lane.dart';
import 'cut_timeline_tracks.dart';
import 'graph_maths.dart' show rationalSeconds;
import 'placeholder.dart';
import 'timeline_bar_frb.dart' show BarGrab;
import 'timeline_extras_frb.dart';
import 'timeline_key_lane_frb.dart' show RowDividerPainter;
import 'timeline_lane_bottom_bar_frb.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_navigator.dart';
import 'timeline_outline_frb.dart' show GutterScrollbar;
import 'timeline_razor.dart' show RazorOverlay;
import 'timeline_snap.dart';
import 'timeline_toolbar_frb.dart' show timelineChromeControl;
import 'waveform_frb.dart';

/// How wide the header column stands: a name and three switches.
const double cutTimelineOutlineWidth = 180;

/// The narrowest sliver of lane the seam leaves, however tight the panel is.
const double _laneFloor = 60;

/// How many frames full zoom-in shows across the lanes, the Timeline's own.
const int _framesAtFullZoom = 20;

/// How far the pointer travels before a press is a drag rather than a click.
const double _clickSlop = 3;

/// The longest edge of a clip's thumbnail, which is a track's own height.
const int _thumbEdge = 96;

/// A press in flight on the lanes: where it began and is now, whether it has
/// travelled, which track and clip it took hold of, whether it has a fade
/// handle rather than the clip, the start's when [fade] is true, and what
/// the release will do with it.
typedef _Press = ({
  Offset from,
  Offset at,
  bool moved,
  int? track,
  BridgeClip? clip,
  BarGrab grab,
  bool? fade,
  bool marquee,
  bool additive,
  CutTool tool,
});

class CutTimelinePanelFrb extends StatefulWidget {
  const CutTimelinePanelFrb({super.key});

  @override
  State<CutTimelinePanelFrb> createState() => _CutTimelinePanelFrbState();
}

class _CutTimelinePanelFrbState extends State<CutTimelinePanelFrb>
    with SingleTickerProviderStateMixin {
  /// The two halves' vertical scrolls, kept in step: they are one table.
  final ScrollController _vOutline = ScrollController();
  final ScrollController _vLane = ScrollController();
  bool _syncingScroll = false;

  /// The lanes' horizontal scroll, anchored so a zoom holds a frame still.
  final ZoomAnchoredScrollController _hLane = ZoomAnchoredScrollController();

  late final SmoothZoom _zoomMotion;
  double _zoomAnchorFrame = 0;
  double _zoomAnchorViewportX = 0;
  bool _zoomAnchorHeld = false;
  bool _pullingBackZoom = false;
  AnimationLevel _animationLevel = AnimationLevel.all;

  /// The lanes' viewport and the comp's length as the last layout knew them.
  double _laneViewport = 0;
  double _laneHeight = 0;
  int _laneFrames = 1;
  double _trackHeight = 0;

  double get _zoom => _zoomMotion.value;
  double get _maxZoom => max(1.0, _laneFrames / _framesAtFullZoom.toDouble());

  /// The tracks the last model gave, rebuilt once per document revision, and
  /// which track each clip stands on so a selection across tracks resolves
  /// without a walk.
  List<CutTrack> _tracks = const [];
  Map<String, int> _trackOfClip = const {};
  BigInt? _tracksRevision;
  CompositionReference? _tracksComp;

  /// The clips that share a link, by clip id: a picture clip and the clip
  /// carrying its sound, which select and travel together while the strip's
  /// Linked switch is on.
  Map<String, Set<String>> _mates = const {};

  /// The tool armed on the strip, other than the razor, which rides the
  /// shell's own tool state so the toolbar and its chord arm it too.
  CutTool _localTool = CutTool.select;

  /// A tool key held down: which key, the tool it armed, the tool it
  /// displaced, and whether a gesture was made while it was down. A tap
  /// switches tool for good; a hold with a gesture under it gives the tool
  /// back on release.
  ({PhysicalKeyboardKey key, CutTool tool, CutTool before, bool used})?
      _heldTool;

  /// The zoom the fit key left, so pressing it again goes back there.
  double? _zoomBeforeFit;

  /// The picked clips by id. The panel's own: nothing about a picked clip is
  /// in the document. Delete takes them while this panel holds the keys.
  Set<String> _selected = {};

  /// The gesture in flight, the drag the painters read off it, what the
  /// drag has snapped to, the readout beside the pointer, the edges under
  /// the pointer at rest, and the box a marquee is sweeping. The notifiers
  /// repaint the lanes without a rebuild.
  _Press? _press;
  final ValueNotifier<CutDrag?> _drag = ValueNotifier<CutDrag?>(null);
  final ValueNotifier<SnapTarget?> _caught = ValueNotifier<SnapTarget?>(null);
  final ValueNotifier<({Offset at, int row, String text})?> _hint =
      ValueNotifier<({Offset at, int row, String text})?>(null);
  final ValueNotifier<CutHover?> _hover = ValueNotifier<CutHover?>(null);
  final ValueNotifier<Rect?> _marquee = ValueNotifier<Rect?>(null);
  final DragEscape _escape = DragEscape();

  /// The strip's Snap. Ctrl held suspends it for the length of a gesture.
  bool _snap = true;

  /// Each visible clip's picture, by thumbnail key or clip id, and what the
  /// peaks were fetched for. A key is claimed before its fetch starts, so a
  /// rebuild mid-decode asks nothing twice; an arrival bumps [_pictures], and
  /// only the lanes repaint.
  final Map<String, ui.Image?> _thumbs = {};
  final Map<String, BridgeAudioPeaks> _peaks = {};
  final Map<String, String> _peakKeys = {};
  final ValueNotifier<int> _pictures = ValueNotifier<int>(0);
  bool _refreshQueued = false;

  /// Laid-out clip names, by name, shared by every track's painter.
  final Map<String, TextPainter> _names = {};
  TextStyle? _nameStyle;

  /// The composition glyph in the name colour, for the clips that play a
  /// composition, made once per colour and shared by every painter.
  CutGlyph? _compGlyph;
  Color? _glyphColour;

  /// Alt was down when a Select drag took hold of a clip's body: the release
  /// puts copies down and leaves the clips where they were.
  bool _duplicating = false;

  /// Two clicks on a clip close together open it.
  final DoubleTap _doubleTap = DoubleTap();

  /// The work area, held between document revisions.
  ({int start, int end, bool whole})? _workArea;
  BigInt? _workRevision;
  CompositionReference? _workComp;
  final ValueNotifier<({int start, int end, bool whole})?> _workPreview =
      ValueNotifier<({int start, int end, bool whole})?>(null);

  /// Everything a drag can land on, gathered once per build.
  List<SnapTarget> _snapTargets = const [];

  Listenable? _cacheRevision;
  LumitUiState? _ui;
  ToolsState? _boundTools;
  VoidCallback? _escapeRelease;
  bool _claimed = false;
  bool Function()? _heldDelete;
  bool Function()? _heldCopy;
  bool Function()? _heldCut;
  bool Function()? _heldPaste;

  /// The lanes' scrolled content, so a drop can be turned into a track and a
  /// frame.
  final GlobalKey _laneContent = GlobalKey();

  @override
  void initState() {
    super.initState();
    _escapeRelease = EscapeLadder.register(EscapeRung.selection, _escapeClaim);
    _zoomMotion = SmoothZoom(vsync: this, initial: 1, min: 1, max: 64)
      ..addListener(_onZoomTick);
    _vOutline.addListener(() => _followScroll(_vOutline, _vLane));
    _vLane.addListener(() => _followScroll(_vLane, _vOutline));
    // A scroll moves which clips are on screen, and their pictures are
    // fetched for what is on screen alone.
    _hLane.addListener(_queueRefresh);
    _vLane.addListener(_queueRefresh);
    _ui = Provider.of<LumitUiState>(context, listen: false);
    _cacheRevision = Listenable.merge([_ui!.frameArrived, _ui!.cacheChanged]);
    _ui!.activePane.addListener(_onActivePanel);
    _ui!.cutLinked.addListener(_onToolChanged);
    // The playhead stays on screen while the transport runs, as it does in
    // the Timeline.
    _ui!.playheadFrame.addListener(_edgeFollow);
    _onActivePanel();
    HardwareKeyboard.instance.addHandler(_onKey);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    _escapeRelease?.call();
    _escape.dispose();
    _boundTools?.removeListener(_onToolChanged);
    _ui?.activePane.removeListener(_onActivePanel);
    _ui?.cutLinked.removeListener(_onToolChanged);
    _ui?.playheadFrame.removeListener(_edgeFollow);
    _releaseKeys();
    _zoomMotion.dispose();
    _vOutline.dispose();
    _vLane.dispose();
    _hLane.dispose();
    _drag.dispose();
    _caught.dispose();
    _hint.dispose();
    _hover.dispose();
    _marquee.dispose();
    _pictures.dispose();
    _workPreview.dispose();
    for (final image in _thumbs.values) {
      image?.dispose();
    }
    for (final text in _names.values) {
      text.dispose();
    }
    _compGlyph?.picture.dispose();
    super.dispose();
  }

  void _onToolChanged() {
    if (mounted) setState(() {});
  }

  void _bindTools(LumitUiState ui) {
    if (identical(_boundTools, ui.tools)) return;
    _boundTools?.removeListener(_onToolChanged);
    _boundTools = ui.tools..addListener(_onToolChanged);
  }

  /// The tool in hand. Razor rides the shell's own tool, so the toolbar and
  /// its chord arm it here as well; the rest are the strip's own.
  CutTool _toolOf(LumitUiState ui) =>
      ui.tools.tool.group == ToolGroup.razor ? CutTool.razor : _localTool;

  void _armTool(LumitUiState ui, CutTool tool) {
    ui.tools.selectGroup(
        tool == CutTool.razor ? ToolGroup.razor : ToolGroup.select);
    if (tool != CutTool.razor) setState(() => _localTool = tool);
    _retool(tool);
  }

  /// An edge drag in flight follows the Ripple key as it goes down and comes
  /// up: the trim in hand becomes a ripple trim, or a plain one again, the
  /// preview follows at once, and the release commits whichever is in force.
  void _retool(CutTool tool) {
    final held = _press;
    const trims = {CutTool.select, CutTool.ripple};
    if (held == null ||
        held.clip == null ||
        held.grab == BarGrab.move ||
        held.fade != null ||
        held.tool == tool ||
        !trims.contains(held.tool) ||
        !trims.contains(tool)) {
      return;
    }
    _press = (
      from: held.from,
      at: held.at,
      moved: held.moved,
      track: held.track,
      clip: held.clip,
      grab: held.grab,
      fade: held.fade,
      marquee: held.marquee,
      additive: held.additive,
      tool: tool,
    );
    if (held.moved) _showDrag(_press!);
  }

  /// Escape with clips picked lets them go, and nothing else.
  bool _escapeClaim() {
    if (!mounted || _selected.isEmpty) return false;
    setState(() => _selected = {});
    return true;
  }

  /// Where every edit made here ends: the read model freshened.
  void _afterWrite() => _ui?.model.refresh();

  /// Where every Cut edit ends: a refusal is said on the status line and
  /// the document is untouched; a done one is told to everyone.
  void _afterCut(BridgeCutResult result) {
    final ui = _ui;
    if (ui == null || !mounted) return;
    ui.reportCut(result);
    if (result != BridgeCutResult.done) return;
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
    _afterWrite();
  }

  bool get _linked => _ui?.cutLinked.value ?? true;

  /// [id] and, while Linked is on, the clips linked to it.
  Set<String> _withMates(String id) =>
      _linked ? {id, ...?_mates[id]} : {id};

  /// The Cut context's keys: the tools, the trims to the playhead, the cut
  /// at the playhead, the nudges, the selection and the view. On the
  /// hardware keyboard, because a panel holds no focus, and answered only
  /// while this is the focused panel; the shell marks the chord handled and
  /// does nothing else with it. A key coming up is read whichever panel has
  /// the keys, so a held tool key always gives its tool back.
  bool _onKey(KeyEvent event) {
    if (!mounted) return false;
    final ui = _ui;
    if (ui == null) return false;
    if (event is KeyUpEvent) return _onKeyUp(event, ui);
    if (lumitModalOpen || ui.activePanel != Panel.cutTimeline) return false;
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return false;
    }
    final action = ui.keymap.actionFor(BridgeKeyContext.cut, event);
    if (action == null) return false;
    // A held key repeats a nudge, a zoom and a step to the next edit point;
    // everything else is pressed once however long it is held.
    if (event is KeyRepeatEvent &&
        !action.startsWith('cut.nudge.') &&
        !action.startsWith('timeline.zoom.') &&
        !action.startsWith('edit.point.')) {
      return true;
    }
    final comp = ui.selectedComp;
    switch (action) {
      case 'cut.tool.select':
        _holdTool(ui, CutTool.select, event);
      case 'cut.tool.razor':
        _holdTool(ui, CutTool.razor, event);
      case 'cut.tool.ripple':
        _holdTool(ui, CutTool.ripple, event);
      case 'cut.tool.roll':
        _holdTool(ui, CutTool.roll, event);
      case 'cut.tool.slip':
        _holdTool(ui, CutTool.slip, event);
      case 'cut.tool.slide':
        _holdTool(ui, CutTool.slide, event);
      case 'cut.trim.start.ripple':
        _trimToPlayhead(ui, comp, start: true, ripple: true);
      case 'cut.trim.end.ripple':
        _trimToPlayhead(ui, comp, start: false, ripple: true);
      case 'cut.trim.start':
        _trimToPlayhead(ui, comp, start: true, ripple: false);
      case 'cut.trim.end':
        _trimToPlayhead(ui, comp, start: false, ripple: false);
      case 'cut.roll.prev':
        _rollToPlayhead(ui, comp, next: false);
      case 'cut.roll.next':
        _rollToPlayhead(ui, comp, next: true);
      case 'cut.add.edit':
        _addEdit(ui, comp, all: false);
      case 'cut.add.edit.all':
        _addEdit(ui, comp, all: true);
      case 'cut.nudge.left':
        _nudge(comp, frames: -1);
      case 'cut.nudge.right':
        _nudge(comp, frames: 1);
      case 'cut.nudge.left.many':
        _nudge(comp, frames: -5);
      case 'cut.nudge.right.many':
        _nudge(comp, frames: 5);
      case 'cut.nudge.up':
        _nudge(comp, tracks: -1);
      case 'cut.nudge.down':
        _nudge(comp, tracks: 1);
      case 'cut.select.at.playhead':
        _selectAtPlayhead(ui);
      case 'cut.snap.toggle':
        setState(() => _snap = !_snap);
      case 'cut.linked.toggle':
        ui.cutLinked.value = !ui.cutLinked.value;
      case 'cut.delete.ripple':
        _deleteSelected(ripple: true);
      case 'cut.lift':
        _removeWorkArea(ui, comp, ripple: false);
      case 'cut.extract':
        _removeWorkArea(ui, comp, ripple: true);
      case 'cut.match.frame':
        _matchFrame(ui, comp);
      case 'cut.transition.default':
        _defaultTransition(ui, comp);
      // The zoom keys, as the Timeline answers them: `=` in and `-` out
      // about the playhead, `\` between the whole composition and wherever
      // the zoom was before.
      case 'timeline.zoom.in' || 'timeline.zoom.out':
        _zoomBeforeFit = null;
        _setZoom(zoomNudged(_zoomMotion.target,
            inward: action == 'timeline.zoom.in', maxZoom: _maxZoom));
      case 'timeline.zoom.fit':
        final was = _zoomBeforeFit;
        if (_zoomMotion.target > 1) {
          _zoomBeforeFit = _zoomMotion.target;
          _setZoom(1);
        } else {
          _zoomBeforeFit = null;
          _setZoom((was ?? 1).clamp(1.0, _maxZoom));
        }
      case 'edit.point.prev' || 'edit.point.next':
        _toEditPoint(ui, before: action.endsWith('prev'));
      default:
        return false;
    }
    return true;
  }

  /// A tool key down. The tool is armed at once; whether it stays is
  /// decided when the key comes up, by whether a gesture was made under it.
  /// Pressed during a drag, the drag counts as that gesture, which is what
  /// lets the Ripple key turn a trim in hand into a ripple trim and back.
  void _holdTool(LumitUiState ui, CutTool tool, KeyEvent event) {
    if (event is KeyRepeatEvent) return;
    final was = _toolOf(ui);
    if (was == tool) return;
    _heldTool =
        (key: event.physicalKey, tool: tool, before: was, used: _press != null);
    _armTool(ui, tool);
  }

  bool _onKeyUp(KeyUpEvent event, LumitUiState ui) {
    final held = _heldTool;
    if (held == null || held.key != event.physicalKey) return false;
    _heldTool = null;
    // Given back only when the hold was used as a hold, and only when the
    // tool it armed is still in hand: a click on the strip meanwhile wins.
    if (held.used && _toolOf(ui) == held.tool) _armTool(ui, held.before);
    return true;
  }

  // ------------------------------------------------------ the playhead keys

  /// The tracks a playhead command works on: the picked clips' own, top
  /// first, else every track.
  List<CutTrack> _candidateTracks() {
    if (_selected.isEmpty) return _tracks;
    final at = <int>{
      for (final id in _selected)
        if (_trackOfClip[id] case final i?) i,
    };
    return [for (final i in at.toList()..sort()) _tracks[i]];
  }

  /// The clip a playhead command means: the one under the playhead on the
  /// picked clips' tracks, else on the top-most track that has one there.
  /// One clip, since each Cut call takes one and carries its links; the
  /// commands that act across tracks go by a span instead.
  BridgeClip? _underPlayhead(int at) {
    for (final track in _candidateTracks()) {
      final clip = cutClipUnder(track, at);
      if (clip != null) return clip;
    }
    return null;
  }

  /// Q and W, and with Alt: the clip under the playhead has its start or
  /// its end brought to the playhead, so what is before or after it in that
  /// clip is gone, closing the cut with [ripple] and leaving the room
  /// without. With clips picked on one track that is the clip there; with
  /// nothing picked, or a selection across tracks, the same idea is applied
  /// across those tracks as a span, from the playhead to the nearest edit
  /// point on any of them.
  void _trimToPlayhead(LumitUiState ui, CompositionReference? comp,
      {required bool start, required bool ripple}) {
    if (comp == null) return;
    final at = ui.playheadFrame.value;
    final tracks = _candidateTracks();
    if (_selected.isEmpty || tracks.length > 1) {
      _trimSpanToPlayhead(ui, comp, start: start, ripple: ripple);
      return;
    }
    final clip = _underPlayhead(at);
    // A clip starting on the playhead has nothing before it to lose, and
    // one ending there would have nothing left.
    if (clip == null || at <= clip.startFrame.toInt()) {
      ui.reportCut(BridgeCutResult.nothing);
      return;
    }
    _afterCut(_cut(() => comp.cutTrim(
          clip: clip.id,
          startFrame: start ? at : clip.startFrame,
          endFrame: start ? clip.endFrame : at,
          ripple: ripple,
          linked: _linked,
        )));
  }

  /// The trim to the playhead across tracks: W removes from the playhead to
  /// the earliest edit point after it on the tracks in hand, Q from the
  /// latest edit point before it to the playhead, and the playhead lands on
  /// the cut. The tracks are the picked clips' own, or every unlocked track,
  /// which an empty list means to the engine.
  void _trimSpanToPlayhead(LumitUiState ui, CompositionReference comp,
      {required bool start, required bool ripple}) {
    final at = ui.playheadFrame.value;
    final tracks = _selected.isEmpty ? _unlockedTracks() : _candidateTracks();
    int? edge;
    for (final track in tracks) {
      final frame = cutEditPointNear(track, at, before: start);
      if (frame == null) continue;
      if (edge == null || (start ? frame > edge : frame < edge)) edge = frame;
    }
    if (edge == null) {
      ui.reportCut(BridgeCutResult.nothing);
      return;
    }
    final from = start ? edge : at;
    final result = _cut(() => comp.cutRemoveSpan(
          startFrame: from,
          endFrame: start ? at : edge!,
          layers: _selected.isEmpty
              ? const []
              : [for (final track in tracks) track.entry.layer],
          ripple: ripple,
        ));
    if (result == BridgeCutResult.done) ui.scrubTo(from);
    _afterCut(result);
  }

  /// Every track whose layer is not locked, which is what a command with
  /// nothing picked acts on.
  List<CutTrack> _unlockedTracks() =>
      [for (final track in _tracks) if (!track.entry.info.switches.locked) track];

  /// The layer a command with a track in mind means: the picked layer when
  /// it is one of the tracks, else the top-most track the picked clips stand
  /// on, which is a picture track when they stand on one, else the track
  /// last pressed, so a paste after a cut lands where the clips came from,
  /// else none.
  LayerReference? _selectedTrackLayer(LumitUiState ui) {
    final picked = ui.selectedLayer.value?.internallayerId;
    if (picked != null) {
      for (final track in _tracks) {
        if (track.entry.layer.internallayerId == picked) {
          return track.entry.layer;
        }
      }
    }
    if (_selected.isNotEmpty) return _candidateTracks().first.entry.layer;
    for (final track in _tracks) {
      if (track.id == _lastTrack) return track.entry.layer;
    }
    return null;
  }

  /// The id of the track the last press landed on.
  String? _lastTrack;

  /// `;` and `'`: the work area is lifted out, leaving its room, or
  /// extracted, closing it, on the picked clips' tracks or every unlocked
  /// track. Quietly nothing while the work area is the whole composition.
  void _removeWorkArea(LumitUiState ui, CompositionReference? comp,
      {required bool ripple}) {
    final work = _workArea;
    if (comp == null || work == null || work.whole) return;
    final result = _cut(() => comp.cutRemoveSpan(
          startFrame: work.start,
          endFrame: work.end,
          layers: _selected.isEmpty
              ? const []
              : [for (final track in _candidateTracks()) track.entry.layer],
          ripple: ripple,
        ));
    if (ripple && result == BridgeCutResult.done) ui.scrubTo(work.start);
    _afterCut(result);
  }

  /// Mod+D: a transition of one second on the edit point nearest the
  /// playhead. With clips picked the edit points are their own edges, on
  /// their tracks; with nothing picked, every edge on every unlocked track.
  /// The clip ending there takes it, or the one starting there where
  /// nothing ends, which the engine makes a fade in; a picked clip with
  /// no neighbour at the edge gets a fade the same way. One call per track
  /// at that frame, the links left to the engine.
  void _defaultTransition(LumitUiState ui, CompositionReference? comp) {
    if (comp == null) return;
    final at = ui.playheadFrame.value;
    final picked = _selected.isNotEmpty;
    final tracks = picked ? _candidateTracks() : _unlockedTracks();
    int? nearest;
    void offer(int frame) {
      if (nearest == null || (frame - at).abs() < (nearest! - at).abs()) {
        nearest = frame;
      }
    }

    if (picked) {
      for (final id in _selected) {
        final clip = _trackOfClip[id] == null
            ? null
            : _tracks[_trackOfClip[id]!].clip(id);
        if (clip == null) continue;
        offer(clip.startFrame.toInt());
        offer(clip.endFrame.toInt());
      }
    } else {
      for (final track in tracks) {
        if (cutClipUnder(track, at) case final under?) {
          offer(under.startFrame.toInt());
          offer(under.endFrame.toInt());
        }
        for (final before in const [true, false]) {
          if (cutEditPointNear(track, at, before: before) case final frame?) {
            offer(frame);
          }
        }
      }
    }
    final point = nearest;
    if (point == null) {
      ui.reportCut(BridgeCutResult.nothing);
      return;
    }
    final done = <String>{};
    for (final track in tracks) {
      BridgeClip? clip;
      var endEdge = true;
      final (lo, hi) = track.window(point, point);
      for (var i = lo; i < hi; i++) {
        final each = track.clips[i];
        if (each.endFrame.toInt() == point) {
          clip = each;
          endEdge = true;
          break;
        }
        if (each.startFrame.toInt() == point) {
          clip = each;
          endEdge = false;
        }
      }
      if (clip == null || done.contains(clip.id.toString())) continue;
      done.addAll(_withMates(clip.id.toString()));
      _transition(ui, comp, clip, endEdge: endEdge, frames: _secondFrames(ui));
    }
  }

  /// One second in this composition's frames: the default transition.
  int _secondFrames(LumitUiState ui) => max(1, ui.model.fps.round());

  /// A transition on one edge of [clip], or none when [frames] is zero.
  void _transition(LumitUiState ui, CompositionReference comp, BridgeClip clip,
      {required bool endEdge, required int frames}) {
    _afterCut(_cut(() => comp.cutTransition(
          clip: clip.id,
          endEdge: endEdge,
          frames: frames,
          linked: _linked,
        )));
  }

  /// F: what the clip under the playhead is showing, on the track in hand
  /// or the top-most one with a clip there, loaded into the source view and
  /// stood on that frame. Nothing without a source view to load it into.
  void _matchFrame(LumitUiState ui, CompositionReference? comp) {
    if (comp == null || ui.sourceView == null) return;
    final hit = _match(comp, ui.playheadFrame.value, _selectedTrackLayer(ui));
    if (hit == null) {
      ui.reportCut(BridgeCutResult.nothing);
      return;
    }
    ui.openFootageViewAt(hit.footage, hit.sourceTime);
  }

  BridgeMatchFrame? _match(
      CompositionReference comp, int frame, LayerReference? layer) {
    try {
      return comp.cutMatchFrame(frame: frame, layer: layer);
    } catch (_) {
      return null;
    }
  }

  /// A clip opened by a double-click: one playing a composition fronts it;
  /// one playing footage loads the footage into the source view, stood on
  /// the frame under the playhead when that is inside the clip and on its
  /// first frame otherwise, with the In and Out marks on the clip's own span.
  void _openClip(LumitUiState ui, CompositionReference comp, CutTrack track,
      BridgeClip clip) {
    if (clip.sourceIsComp) {
      _openAsComposition(ui, comp, clip);
      return;
    }
    if (ui.sourceView == null) return;
    final at = ui.playheadFrame.value;
    final start = clip.startFrame.toInt();
    final inside = at >= start && at < clip.endFrame.toInt();
    final hit = _match(comp, inside ? at : start, track.entry.layer);
    if (hit == null) {
      ui.reportCut(BridgeCutResult.nothing);
      return;
    }
    ui.openFootageViewAt(hit.footage, hit.sourceTime,
        markIn: clip.sourceIn, markOut: clip.sourceOut);
  }

  /// The clip's footage made into a composition the clip then plays, and
  /// that composition fronted; a clip already playing one fronts it. The
  /// engine answers nothing for a locked track.
  void _openAsComposition(
      LumitUiState ui, CompositionReference comp, BridgeClip clip) {
    CompositionReference? made;
    try {
      made = comp.cutClipToComposition(clip: clip.id);
    } catch (_) {
      made = null;
    }
    if (made == null) {
      ui.reportCut(BridgeCutResult.locked);
      return;
    }
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
    _afterWrite();
    ui.setSelectedComp(made);
  }

  /// Shift+Q and Shift+W: the edit point either side of the playhead on
  /// its track rolls to the playhead. The clip under the playhead owns
  /// both, its start and its end; the engine says when nothing stands
  /// across one.
  void _rollToPlayhead(LumitUiState ui, CompositionReference? comp,
      {required bool next}) {
    if (comp == null) return;
    final at = ui.playheadFrame.value;
    final clip = _underPlayhead(at);
    if (clip == null) {
      ui.reportCut(BridgeCutResult.nothing);
      return;
    }
    _afterCut(_cut(() => comp.cutRoll(
          clip: clip.id,
          endEdge: next,
          toFrame: at,
          linked: _linked,
        )));
  }

  /// Mod+K: a cut at the playhead on the picked clips' tracks, or on every
  /// unlocked track when nothing is picked; with Shift always every track,
  /// which an empty list means to the engine.
  void _addEdit(LumitUiState ui, CompositionReference? comp,
      {required bool all}) {
    if (comp == null) return;
    _afterCut(_cut(() => comp.cutRazor(
          layers: all || _selected.isEmpty
              ? const []
              : [for (final track in _candidateTracks()) track.entry.layer],
          atFrame: ui.playheadFrame.value,
          linked: _linked,
        )));
  }

  /// Alt with an arrow: the picked clips move by a frame or five, or up or
  /// down one track of their kind. The grabbed clip is the top-most picked
  /// for a move up and the bottom-most for a move down, so the engine
  /// carries the rest of its kind across as many tracks as it goes.
  void _nudge(CompositionReference? comp, {int frames = 0, int tracks = 0}) {
    if (comp == null) return;
    final ids = _selectedIds();
    if (ids.isEmpty) return;
    final rows = _candidateTracks();
    final row = tracks > 0 ? rows.last : rows.first;
    final at = _tracks.indexOf(row);
    final grabbed = row.clips.firstWhere(
        (clip) => _selected.contains(clip.id.toString()));
    LayerReference? target;
    if (tracks != 0) {
      final to = at + tracks;
      if (to < 0 || to >= _tracks.length || _tracks[to].kind != row.kind) {
        return;
      }
      target = _tracks[to].entry.layer;
    }
    _afterCut(_cut(() => comp.cutMove(
          clips: ids,
          grabbed: grabbed.id,
          byFrames: frames,
          target: target,
          linked: _linked,
        )));
  }

  /// D: the clips under the playhead become the selection, on the picked
  /// clips' tracks or on every track, with their links while Linked is on.
  void _selectAtPlayhead(LumitUiState ui) {
    final at = ui.playheadFrame.value;
    final caught = <String>{};
    for (final track in _candidateTracks()) {
      final clip = cutClipUnder(track, at);
      if (clip != null) caught.addAll(_withMates(clip.id.toString()));
    }
    setState(() => _selected = caught);
  }

  /// Up and Down: the playhead to the nearest edit point before or after
  /// it, on the picked clips' tracks or on every track. From the sorted
  /// clips, so no track is walked and nothing crosses the bridge.
  void _toEditPoint(LumitUiState ui, {required bool before}) {
    final at = ui.playheadFrame.value;
    final last = ui.model.durationFrames;
    int? to;
    for (final track in _candidateTracks()) {
      final frame = cutEditPointNear(track, at, before: before);
      if (frame == null || frame < 0 || frame >= last) continue;
      if (to == null || (before ? frame > to : frame < to)) to = frame;
    }
    if (to != null) ui.scrubTo(to);
  }

  // ------------------------------------------------------------- the claims

  /// Take the shell's Delete while this is the focused panel, and give it
  /// straight back when another panel takes the keys.
  void _onActivePanel() {
    if (_ui?.activePanel == Panel.cutTimeline) {
      _claimKeys();
    } else {
      _releaseKeys();
    }
  }

  void _claimKeys() {
    final ui = _ui;
    if (ui == null || _claimed) return;
    _claimed = true;
    _heldDelete = ui.deleteClaim;
    _heldCopy = ui.copyClaim;
    _heldCut = ui.cutClaim;
    _heldPaste = ui.pasteClaim;
    ui.deleteClaim = _deleteClaim;
    ui.copyClaim = _copyClaim;
    ui.cutClaim = _cutClaim;
    ui.pasteClaim = _pasteClaim;
  }

  void _releaseKeys() {
    final ui = _ui;
    if (ui == null || !_claimed) return;
    _claimed = false;
    // Only what is still ours: a panel that has taken a slot since keeps it.
    if (ui.deleteClaim == _deleteClaim) ui.deleteClaim = _heldDelete;
    if (ui.copyClaim == _copyClaim) ui.copyClaim = _heldCopy;
    if (ui.cutClaim == _cutClaim) ui.cutClaim = _heldCut;
    if (ui.pasteClaim == _pasteClaim) ui.pasteClaim = _heldPaste;
  }

  /// Whether the keys are this panel's to answer.
  bool get _keysAreOurs => mounted && _ui?.activePanel == Panel.cutTimeline;

  /// Delete: the picked clips go, and leave a gap.
  bool _deleteClaim() {
    if (!_keysAreOurs) return false;
    return _deleteSelected(ripple: false);
  }

  /// Copy: the picked clips, and their links while Linked is on, are held
  /// by the engine for a paste. Says whether it took any.
  bool _copyClaim() {
    final ui = _ui;
    final comp = ui?.selectedComp;
    if (!_keysAreOurs || ui == null || comp == null || _selected.isEmpty) {
      return false;
    }
    final result =
        _cut(() => comp.cutCopy(clips: _selectedIds(), linked: _linked));
    ui.reportCut(result);
    return result == BridgeCutResult.done;
  }

  /// Cut: a copy, then the picked clips go and leave their room.
  bool _cutClaim() {
    if (!_copyClaim()) return false;
    _deleteSelected(ripple: false);
    return true;
  }

  /// Paste: the held clips land at the playhead, on the track in hand when
  /// there is one, and the playhead moves to their end. Nothing held leaves
  /// the chord to the layer clipboard.
  bool _pasteClaim() {
    final ui = _ui;
    final comp = ui?.selectedComp;
    if (!_keysAreOurs || ui == null || comp == null) return false;
    final BridgeCutPaste paste;
    try {
      paste = comp.cutPaste(
          atFrame: ui.playheadFrame.value, target: _selectedTrackLayer(ui));
    } catch (_) {
      return false;
    }
    if (paste.result == BridgeCutResult.nothing) return false;
    _afterCut(paste.result);
    if (paste.result == BridgeCutResult.done) ui.scrubTo(paste.endFrame);
    return true;
  }

  /// The picked clips go, leaving a gap or closing it. One call, one undo
  /// step; a refusal leaves the selection standing.
  bool _deleteSelected({required bool ripple}) {
    final comp = _ui?.selectedComp;
    if (!mounted || comp == null || _selected.isEmpty) return false;
    final result = _cut(() => comp.cutDelete(
          clips: _selectedIds(),
          ripple: ripple,
          linked: _linked,
        ));
    if (result == BridgeCutResult.done) setState(() => _selected = {});
    _afterCut(result);
    return true;
  }

  /// The picked clips' ids, as the engine wants them; a clip the document no
  /// longer holds is left out, since an unknown id throws.
  List<UuidValue> _selectedIds() => [
        for (final id in _selected)
          if (_trackOfClip[id] case final at?)
            if (_tracks[at].clip(id) case final clip?) clip.id,
      ];

  /// A Cut call, with the one throw it can make - a clip that went away
  /// between the draw and the press - read as nothing to act on.
  BridgeCutResult _cut(BridgeCutResult Function() call) {
    try {
      return call();
    } catch (_) {
      return BridgeCutResult.nothing;
    }
  }

  // -------------------------------------------------------------- the model

  /// The tracks, once per document revision. The selection is pruned to the
  /// clips that are still there, and the thumbnails of clips that have gone
  /// are let go.
  void _refreshTracks(CompModel model, CompositionReference comp) {
    final revision = model.revision;
    // The revision is the document's, so fronting another composition
    // without an edit between keeps it: the composition is part of the key.
    if (revision != null &&
        revision == _tracksRevision &&
        comp == _tracksComp) {
      return;
    }
    _tracksRevision = revision;
    _tracksComp = comp;
    _tracks = cutTimelineTracks(model.layers);
    final of = <String, int>{};
    final keys = <String>{};
    final byLink = <String, Set<String>>{};
    for (var i = 0; i < _tracks.length; i++) {
      for (final clip in _tracks[i].clips) {
        final id = clip.id.toString();
        of[id] = i;
        keys.add(cutThumbKey(clip));
        if (clip.link case final link?) {
          byLink.putIfAbsent(link.toString(), () => {}).add(id);
        }
      }
    }
    _trackOfClip = of;
    _mates = {
      for (final group in byLink.values)
        for (final id in group) id: group,
    };
    _selected = {
      for (final id in _selected)
        if (of.containsKey(id)) id
    };
    _thumbs.removeWhere((key, image) {
      if (keys.contains(key)) return false;
      image?.dispose();
      return true;
    });
  }

  /// A laid-out name, kept across paints. The cache empties when the style
  /// changes, which is a theme change.
  TextPainter _nameOf(String name) => _names.putIfAbsent(
        name,
        () => TextPainter(
          text: TextSpan(text: name, style: _nameStyle),
          textDirection: TextDirection.ltr,
          maxLines: 1,
        )..layout(),
      );

  void _setNameStyle(TextStyle style) {
    if (_nameStyle == style) return;
    _nameStyle = style;
    for (final text in _names.values) {
      text.dispose();
    }
    _names.clear();
    if (style.color case final colour?) _makeGlyph(colour);
  }

  /// The composition glyph as a picture in the name colour, once per colour;
  /// its arrival rebuilds the lanes.
  void _makeGlyph(Color colour) {
    if (_glyphColour == colour) return;
    _glyphColour = colour;
    vg
        .loadPicture(
            SvgStringLoader(LumitIcons.composition,
                theme: SvgTheme(currentColor: colour)),
            null)
        .then((info) {
      if (!mounted || _glyphColour != colour) {
        info.picture.dispose();
        return;
      }
      // The lanes hold the glyph they were built with, so they are rebuilt
      // with the new one before the old is let go: a lane repainting with a
      // disposed picture throws.
      final old = _compGlyph;
      setState(() => _compGlyph = (picture: info.picture, size: info.size));
      WidgetsBinding.instance
          .addPostFrameCallback((_) => old?.picture.dispose());
    });
  }

  // ----------------------------------------------------------- the pictures

  /// Fetch the picture of every clip in view, and nothing else: thumbnails
  /// for the picture rows, peaks over the visible stretch for the sound
  /// rows. Called after the frame and from the scrolls, never from a build.
  void _queueRefresh() {
    if (_refreshQueued) return;
    _refreshQueued = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _refreshQueued = false;
      if (mounted) _refreshPictures();
    });
  }

  void _refreshPictures() {
    final ui = _ui;
    if (ui == null || _trackHeight <= 0 || _laneViewport <= 0) return;
    final axis = _laneAxis;
    final fps = ui.model.heldFps;
    if (axis.perFrame <= 0 || fps <= 0) return;
    final viewLeft = _hLane.hasClients ? _hLane.offset : 0.0;
    final viewRight = viewLeft + _laneViewport;
    final top = _vLane.hasClients ? _vLane.offset : 0.0;
    final first = (top / _trackHeight).floor().clamp(0, _tracks.length);
    final last = ((top + max(_laneHeight, _trackHeight)) / _trackHeight)
        .ceil()
        .clamp(first, _tracks.length);
    final from = axis.frameAtExact(viewLeft).floor();
    final to = axis.frameAtExact(viewRight).ceil();
    final secondsPerPixel = 1 / (axis.perFrame * fps);
    final live = <String>{};
    for (var i = first; i < last; i++) {
      final track = _tracks[i];
      final (lo, hi) = track.window(from, to);
      for (var k = lo; k < hi; k++) {
        final clip = track.clips[k];
        if (!track.sound) {
          _wantThumb(track, clip);
          continue;
        }
        final id = clip.id.toString();
        live.add(id);
        final (left, width) =
            cutClipBox(axis, clip.startFrame.toInt(), clip.endFrame.toInt());
        final a = max(left, viewLeft);
        final b = min(left + width, viewRight);
        if (!(b > a)) continue;
        final origin = rationalSeconds(clip.placeStart);
        final request = WaveformRequest.forView(
          startSeconds: origin + (a - left) * secondsPerPixel,
          endSeconds: origin + (b - left) * secondsPerPixel,
          pixels: b - a,
        );
        if (request == null) continue;
        final key =
            '${request.key}|${clip.startFrame}|${clip.endFrame}|${clip.retimed}';
        if (_peakKeys[id] == key) continue;
        _peakKeys[id] = key;
        track.entry.layer
            .clipAudioPeaks(
          clip: clip.id,
          startSeconds: request.startSeconds,
          endSeconds: request.endSeconds,
          buckets: request.buckets,
          multiwave: true,
        )
            .then((peaks) {
          if (!mounted || _peakKeys[id] != key) return;
          _peaks[id] = peaks;
          _pictures.value++;
        });
      }
    }
    // A clip that has left the view keeps nothing: its window is stale by
    // the time it comes back.
    _peaks.removeWhere((id, _) => !live.contains(id));
    _peakKeys.removeWhere((id, _) => !live.contains(id));
  }

  void _wantThumb(CutTrack track, BridgeClip clip) {
    final key = cutThumbKey(clip);
    if (_thumbs.containsKey(key)) return;
    _thumbs[key] = null;
    track.entry.layer
        .clipThumbnail(clip: clip.id, maxEdge: _thumbEdge)
        .then((frame) {
      if (!mounted || frame == null || frame.width == 0) return;
      ui.decodeImageFromPixels(
          frame.rgba, frame.width, frame.height, ui.PixelFormat.rgba8888,
          (image) {
        if (!mounted || !_thumbs.containsKey(key)) {
          image.dispose();
          return;
        }
        _thumbs[key] = image;
        _pictures.value++;
      });
    });
  }

  // ---------------------------------------------------------------- the zoom

  void _followScroll(ScrollController from, ScrollController to) {
    if (_syncingScroll || !from.hasClients || !to.hasClients) return;
    if ((to.offset - from.offset).abs() < 0.5) return;
    _syncingScroll = true;
    to.jumpTo(from.offset.clamp(0.0, to.position.maxScrollExtent));
    _syncingScroll = false;
  }

  void _onZoomTick() {
    if (_laneFrames <= 0) return;
    _hLane.hold(ZoomAnchor(
      frame: _zoomAnchorFrame,
      viewportX: _zoomAnchorViewportX,
      frames: _laneFrames,
      pad: TimelineAxis.pad,
    ));
  }

  /// Point the flight's anchor at the playhead: held where it is if it is on
  /// screen, brought to the middle if it is not.
  void _anchorOnPlayhead() {
    final viewport =
        _hLane.hasClients ? _hLane.position.viewportDimension : _laneViewport;
    final offset = _hLane.hasClients ? _hLane.offset : 0.0;
    final perFrame = _hLane.hasClients && _laneFrames > 0
        ? max(
                0.0,
                _hLane.position.viewportDimension +
                    _hLane.position.maxScrollExtent -
                    TimelineAxis.pad * 2) /
            _laneFrames
        : 0.0;
    final playhead = (_ui?.playheadFrame.value ?? 0).toDouble();
    _zoomAnchorFrame = playhead;
    final x = TimelineAxis.pad + playhead * perFrame - offset;
    _zoomAnchorViewportX =
        perFrame > 0 && x >= 0 && x <= viewport ? x : viewport / 2;
  }

  void _setZoom(double z, {bool fly = true}) {
    if (!_zoomAnchorHeld) _anchorOnPlayhead();
    _zoomMotion.goTo(z,
        duration: fly ? animationDuration(_animationLevel) : Duration.zero);
  }

  void _zoomDragStart() {
    _anchorOnPlayhead();
    _zoomAnchorHeld = true;
  }

  void _zoomDragEnd() => _zoomAnchorHeld = false;

  /// The navigator asked for a window, as the Audio timeline answers it.
  void _navigateTo(double start, double span, {required bool pan}) {
    if (_laneFrames <= 0 || span <= 0) return;
    if (pan) {
      _scrollFrameToLeftEdge(start);
      return;
    }
    _zoomAnchorFrame = start;
    _zoomAnchorViewportX = 0;
    _zoomAnchorHeld = true;
    final want = navigatorZoom(
      span: span,
      frames: _laneFrames,
      viewport: positionOf(_hLane)?.viewportDimension ?? _laneViewport,
    ).clamp(1.0, _maxZoom);
    if ((want - _zoomMotion.target).abs() > 1e-9) {
      _setZoom(want, fly: false);
    } else {
      _scrollFrameToLeftEdge(start);
    }
  }

  void _scrollFrameToLeftEdge(double frame) {
    final position = positionOf(_hLane);
    if (position == null || _laneFrames <= 0) return;
    final span = position.viewportDimension +
        position.maxScrollExtent -
        TimelineAxis.pad * 2;
    if (span <= 0) return;
    _hLane.jumpTo((TimelineAxis.pad + frame * span / _laneFrames)
        .clamp(0.0, position.maxScrollExtent));
  }

  /// Keep the playhead in view during playback, as the Timeline does: when
  /// it leaves the viewport the lanes jump so it lands back at the left
  /// edge, and the next page plays out under a still picture. Only while
  /// playing, so it never fights a hand.
  void _edgeFollow() {
    final ui = _ui;
    if (ui == null || !ui.playing.value || !mounted) return;
    final position = positionOf(_hLane);
    if (position == null || position.maxScrollExtent <= 0) return;
    final viewport = position.viewportDimension;
    final span =
        viewport + position.maxScrollExtent - TimelineAxis.pad * 2;
    if (_laneFrames <= 0 || span <= 0) return;
    final x = TimelineAxis.pad + ui.playheadFrame.value * span / _laneFrames;
    final at = x - position.pixels;
    if (at >= 0 && at <= viewport) return;
    _hLane.jumpTo(
        (x - TimelineAxis.pad).clamp(0.0, position.maxScrollExtent));
  }

  void _pullZoomBackToCeiling() {
    if (_pullingBackZoom || _zoomMotion.target <= _maxZoom) return;
    _pullingBackZoom = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _pullingBackZoom = false;
      if (mounted && _zoomMotion.target > _maxZoom) _setZoom(_maxZoom);
    });
  }

  void _scrollBy(ScrollController c, double by) {
    if (!c.hasClients || by == 0) return;
    c.jumpTo((c.offset + by).clamp(0.0, c.position.maxScrollExtent));
  }

  void _wheel(PointerScrollEvent event, double contentX, TimelineAxis axis) {
    final keymap = Provider.of<LumitUiState>(context, listen: false).keymap;
    if (keymap.wheelHeld(BridgeWheelAction.zoomTime)) {
      _zoomAnchorViewportX = contentX - (_hLane.hasClients ? _hLane.offset : 0);
      _zoomAnchorFrame = axis.frameAtExact(contentX);
      _zoomMotion.nudge(
        wheelDelta(event) < 0 ? 1.2 : 1 / 1.2,
        duration: animationDuration(_animationLevel),
      );
      return;
    }
    if (keymap.wheelHeld(BridgeWheelAction.scrollSideways)) {
      _scrollBy(_hLane, wheelDelta(event));
    }
  }

  // ------------------------------------------------------------ the gestures

  /// The axis the lanes were last drawn with: what a drag and a drop are
  /// measured against.
  TimelineAxis get _laneAxis =>
      TimelineAxis(frames: _laneFrames, width: _laneViewport * _zoom);

  bool get _magnet =>
      _snap &&
      !snapSuspended(
          controlPressed: HardwareKeyboard.instance.isControlPressed);

  /// Where a razor cut at lane x lands: the one answer the blade's line and
  /// the cut both read.
  double _razorFrameAt(double x) {
    final axis = _laneAxis;
    return snapFrame(
      frame: axis.frameAtExact(x),
      targets: _snapTargets,
      perFrame: axis.perFrame,
      magnet: _magnet,
    ).frame.roundToDouble();
  }

  /// The pointer went down on the lanes. A raw listener, never a drag
  /// recogniser: the lanes sit inside two scroll views, which would win the
  /// arena from one.
  void _down(PointerDownEvent event, LumitUiState ui) {
    if (_press != null) return;
    final at = event.localPosition;
    final index = cutTrackAt(_tracks, at.dy, _trackHeight);
    final track = index == null ? null : _tracks[index];
    var hit = track == null ? null : cutClipGrabAt(track, _laneAxis, at.dx);
    if (event.buttons == kSecondaryMouseButton) {
      if (track != null) _menu(ui, track, hit?.clip, at, event.position);
      return;
    }
    if (event.buttons != kPrimaryMouseButton) return;
    final keys = HardwareKeyboard.instance;
    final additive = keys.isShiftPressed || keys.isControlPressed;
    final tool = _toolOf(ui);
    // A fade handle, with Select: it takes its corner from the edge, the
    // body and the empty ground outside, as the Audio timeline's handle
    // does. The edge is still trimmed from anywhere below the handle.
    final fade = _fadeAt(ui, track, index, at, tool);
    if (fade != null) hit = (clip: fade.clip, grab: fade.grab);
    if (hit != null && tool != CutTool.razor) {
      final id = hit.clip.id.toString();
      // A press picks the clip and, while Linked is on, the clips linked to
      // it; Alt picks the one alone. Shift or Ctrl toggles them in the
      // selection; a plain press on a clip not yet picked makes them the
      // selection, and one on a picked clip keeps the set so the whole of
      // it can be dragged.
      final these = keys.isAltPressed ? {id} : _withMates(id);
      setState(() {
        if (additive) {
          _selected = {..._selected};
          if (!_selected.remove(id)) _selected.addAll(these);
        } else if (!_selected.contains(id)) {
          _selected = these;
        }
      });
    }
    // What the press can do with the tool in hand. Roll wants an edit
    // point, which is an edge; Slip and Slide want a body. A press that
    // the tool has no use for still picks, and drags nothing.
    final grab = hit?.grab ?? BarGrab.move;
    _lastPressed = hit?.clip.id.toString();
    if (track != null) _lastTrack = track.id;
    // Alt on a body with Select: the drag puts copies down. A click without
    // a drag has already picked the one clip alone.
    _duplicating = hit != null &&
        grab == BarGrab.move &&
        tool == CutTool.select &&
        keys.isAltPressed;
    final usable = switch (tool) {
      CutTool.roll => grab != BarGrab.move,
      CutTool.slip || CutTool.slide => grab == BarGrab.move,
      _ => true,
    };
    _press = (
      from: at,
      at: at,
      moved: false,
      track: index,
      clip: tool == CutTool.razor || !usable ? null : hit?.clip,
      grab: grab,
      fade: fade?.into,
      marquee: hit == null && tool != CutTool.razor,
      additive: additive,
      tool: tool,
    );
    _hover.value = null;
    // A gesture under a held tool key: the key gives its tool back on
    // release.
    if (_heldTool case final held?) {
      _heldTool =
          (key: held.key, tool: held.tool, before: held.before, used: true);
    }
    _escape.begin(_clearDrag);
  }

  /// Nothing in flight: what Escape, the release and a cancel all leave.
  void _clearDrag() {
    _press = null;
    _drag.value = null;
    _caught.value = null;
    _hint.value = null;
    _marquee.value = null;
  }

  /// The fade handle under [at] on [track], or null: only with Select.
  /// Answers the clip, the end, and the grab that end is.
  ({BridgeClip clip, bool into, BarGrab grab})? _fadeAt(LumitUiState ui,
      CutTrack? track, int? index, Offset at, CutTool tool) {
    if (track == null || index == null || tool != CutTool.select) return null;
    final fade = cutFadeGrabAt(
        track, _laneAxis, at.dx, at.dy - index * _trackHeight, ui.model.fps);
    if (fade == null) return null;
    return (
      clip: fade.clip,
      into: fade.into,
      grab: fade.into ? BarGrab.trimIn : BarGrab.trimOut,
    );
  }

  /// The drag as it stands, published for the lanes: the frames, what it
  /// snapped to, and the readout beside the pointer, which says how far a
  /// clip has gone, or how long a fade in hand now is.
  void _showDrag(_Press held) {
    final drag = _dragOf(held);
    _drag.value = drag;
    final row = cutTrackAt(_tracks, held.at.dy, _trackHeight) ?? held.track ?? 0;
    final String text;
    if (held.fade case final into?) {
      final frames = cutFadeFrames(held.clip!, _ui?.model.fps ?? 0,
          into: into, shift: drag.shift);
      text = l10n.cutFadeLength('$frames');
    } else {
      text = l10n.cutDragDelta(
          drag.shift > 0 ? '+${drag.shift}' : '${drag.shift}');
    }
    _hint.value = (at: held.at, row: row, text: text);
  }

  /// The pointer at rest over the lanes: an edge under it is marked, and
  /// while Linked is on the same edge of the clips linked to it, so it is
  /// plain both will move; a fade handle the same way, the handle rather
  /// than the edge. Not with Alt held, which takes the one clip alone, and
  /// not with a tool that has no use for an edge.
  void _hoverAt(Offset at, LumitUiState ui) {
    CutHover? next;
    if (_press == null && !HardwareKeyboard.instance.isAltPressed) {
      final tool = _toolOf(ui);
      final index = cutTrackAt(_tracks, at.dy, _trackHeight);
      final track = index == null ? null : _tracks[index];
      final hit = track == null ? null : cutClipGrabAt(track, _laneAxis, at.dx);
      // The handle before the edge, the order a press takes them in.
      if (_fadeAt(ui, track, index, at, tool) case final fade?) {
        next = (
          clips: _withMates(fade.clip.id.toString()),
          grab: fade.grab,
          fade: fade.into,
        );
      } else if (hit != null &&
          hit.grab != BarGrab.move &&
          (tool == CutTool.select ||
              tool == CutTool.ripple ||
              tool == CutTool.roll)) {
        next = (
          clips: _withMates(hit.clip.id.toString()),
          grab: hit.grab,
          fade: null,
        );
      }
    }
    final was = _hover.value;
    if (was?.grab == next?.grab &&
        was?.fade == next?.fade &&
        setEquals(was?.clips, next?.clips)) {
      return;
    }
    _hover.value = next;
  }

  void _move(PointerMoveEvent event) {
    if (event.buttons == kMiddleMouseButton) {
      _scrollBy(_hLane, -event.delta.dx);
      _scrollBy(_vLane, -event.delta.dy);
      return;
    }
    final held = _press;
    if (held == null || !_escape.running) return;
    final at = event.localPosition;
    final moved = held.moved || (at - held.from).distance > _clickSlop;
    _press = (
      from: held.from,
      at: at,
      moved: moved,
      track: held.track,
      clip: held.clip,
      grab: held.grab,
      fade: held.fade,
      marquee: held.marquee,
      additive: held.additive,
      tool: held.tool,
    );
    if (!moved) return;
    if (held.marquee) {
      _marquee.value = Rect.fromPoints(held.from, at);
    } else if (held.clip != null) {
      _showDrag(_press!);
    }
  }

  void _up(PointerUpEvent event, LumitUiState ui) {
    final held = _press;
    final commit = _escape.end();
    _clearDrag();
    if (held == null || !commit) return;
    if (held.tool == CutTool.razor) {
      if (held.moved || held.track == null) return;
      _razorCut(_tracks[held.track!].entry, _razorFrameAt(held.at.dx).round());
      return;
    }
    // A second click on the same clip opens it.
    if (!held.moved &&
        held.track != null &&
        _lastPressed != null &&
        _doubleTap.tap(at: held.from, slop: _clickSlop * 2)) {
      final track = _tracks[held.track!];
      final clip = track.clip(_lastPressed!);
      final comp = ui.selectedComp;
      if (clip != null && comp != null) _openClip(ui, comp, track, clip);
      return;
    }
    if (held.marquee) {
      if (!held.moved) {
        if (_selected.isNotEmpty) setState(() => _selected = {});
        return;
      }
      _selectIn(Rect.fromPoints(held.from, held.at), held.additive);
      return;
    }
    if (held.moved && held.clip != null) _commit(_dragOf(held));
  }

  void _cancel() {
    _escape.end();
    _clearDrag();
  }

  /// Every clip the box crossed, on every track it spans.
  void _selectIn(Rect box, bool additive) {
    final axis = _laneAxis;
    final from = axis.frameAtExact(box.left).floor();
    final to = axis.frameAtExact(box.right).ceil();
    final first = cutTrackAt(_tracks, max(0.0, box.top), _trackHeight) ?? 0;
    final last = cutTrackAt(_tracks, box.bottom, _trackHeight) ??
        _tracks.length - 1;
    final caught = <String>{if (additive) ..._selected};
    for (var i = first; i <= last && i < _tracks.length; i++) {
      for (final id in cutClipsBetween(_tracks[i], from, to)) {
        caught.addAll(_withMates(id));
      }
    }
    setState(() => _selected = caught);
  }

  /// What the drag has done so far: the travel in whole frames with the
  /// magnet applied, taken afresh from the raw pixels every time so an edge
  /// caught on a target can still be pulled off it, the tracks crossed, and
  /// what else the engine will move. What the drag snapped to is published
  /// on the way, for the line the lanes draw through it.
  CutDrag _dragOf(_Press held) {
    final clip = held.clip!;
    final id = clip.id.toString();
    final axis = _laneAxis;
    final from = clip.startFrame.toInt();
    final to = clip.endFrame.toInt();
    final tool = held.tool;
    final move = held.grab == BarGrab.move && tool != CutTool.slip;
    final track = held.track == null ? null : _tracks[held.track!];
    final next = track == null || tool != CutTool.roll
        ? null
        : held.grab == BarGrab.trimOut
            ? cutClipAfter(track, clip)
            : _clipBefore(track, clip);
    // The clips that travel: the selection for a move; the pressed clip
    // and, while Linked is on, its links for an edge, a slip or a slide; a
    // roll takes the clips either side of the edit point and their links.
    final Set<String> clips;
    if (tool == CutTool.roll) {
      clips = {
        ..._withMates(id),
        if (next != null) ..._withMates(next.id.toString()),
      };
    } else if (move && tool != CutTool.slide && _selected.contains(id)) {
      clips = _selected;
    } else {
      clips = _withMates(id);
    }
    // The dragged clips' own ends are dropped from the targets: a target
    // standing where a source already is pins the drag where it started.
    final own = <double>{};
    for (final id in clips) {
      final at = _trackOfClip[id];
      final c = at == null ? null : _tracks[at].clip(id);
      if (c == null) continue;
      own
        ..add(c.startFrame.toDouble())
        ..add(c.endFrame.toDouble());
    }
    final rawFrames = axis.framesOfPx(held.at.dx - held.from.dx);
    final targets = _snapTargets.where((t) => !own.contains(t.frame));
    // A fade handle moves the ramp's inner end, held inside the clip, and
    // no box at all.
    if (held.fade case final into?) {
      final fps = _ui?.model.fps ?? 0;
      final now = cutFadeFrames(clip, fps, into: into);
      final snapped = snappedDelta(
        rawFrames: rawFrames,
        perFrame: axis.perFrame,
        sources: [(into ? from + now : to - now).toDouble()],
        targets: targets,
        magnet: _magnet,
      );
      final length = cutFadeFrames(clip, fps, into: into, shift: snapped.delta);
      final shift = into ? length - now : now - length;
      _caught.value = shift == snapped.delta ? snapped.caught : null;
      return (
        tool: tool,
        clips: clips,
        grab: held.grab,
        shift: shift,
        track: held.track,
        trackShift: 0,
        editPoint: null,
        rippleFrom: null,
        others: const {},
        fade: into,
      );
    }
    // A slip moves frames inside the box, so nothing on the axis is a place
    // for it to land.
    final snapped = snappedDelta(
      rawFrames: rawFrames,
      perFrame: axis.perFrame,
      sources: switch (held.grab) {
        BarGrab.move => [from.toDouble(), to.toDouble()],
        BarGrab.trimIn => [from.toDouble()],
        BarGrab.trimOut => [to.toDouble()],
      },
      targets: targets,
      magnet: _magnet && tool != CutTool.slip,
    );
    final shift = cutClampShift(
        clip: clip,
        grab: held.grab,
        tool: tool,
        shift: snapped.delta,
        next: next);
    // A target the clamp pulled the drag off is no longer where it landed.
    _caught.value = shift == snapped.delta ? snapped.caught : null;
    var trackShift = 0;
    if (move && tool != CutTool.slide && held.track != null) {
      final over = cutTrackAt(_tracks, held.at.dy, _trackHeight);
      trackShift = over == null ? 0 : over - held.track!;
    }
    // A slide's neighbours trim and extend to stay against each sliding
    // clip: the one ending where it starts by its end, the one starting
    // where it ends by its start.
    final others = <String, (int, int)>{};
    if (tool == CutTool.slide) {
      for (final each in clips) {
        final at = _trackOfClip[each];
        final c = at == null ? null : _tracks[at].clip(each);
        if (c == null) continue;
        if (_clipBefore(_tracks[at!], c) case final before?) {
          others[before.id.toString()] = (0, shift);
        }
        if (cutClipAfter(_tracks[at], c) case final after?) {
          others[after.id.toString()] = (shift, 0);
        }
      }
    }
    return (
      tool: tool,
      clips: clips,
      grab: held.grab,
      shift: shift,
      track: held.track,
      trackShift: trackShift,
      editPoint: tool != CutTool.roll
          ? null
          : held.grab == BarGrab.trimOut
              ? to
              : from,
      rippleFrom:
          tool == CutTool.ripple && held.grab != BarGrab.move ? to : null,
      others: others,
      fade: null,
    );
  }

  /// The clip that ends where [clip] starts on the same track, or null.
  BridgeClip? _clipBefore(CutTrack track, BridgeClip clip) {
    final start = clip.startFrame.toInt();
    final (lo, hi) = track.window(start, start);
    for (var i = lo; i < hi; i++) {
      if (track.clips[i].endFrame.toInt() == start) return track.clips[i];
    }
    return null;
  }

  /// A gesture let go: written through the one Cut call the tool means, so
  /// the links hold and the whole of it is one undo step. The engine decides
  /// what it will take, and says why when it will not.
  void _commit(CutDrag drag) {
    final comp = _ui?.selectedComp;
    if (comp == null) return;
    if (drag.shift == 0 && drag.trackShift == 0) return;
    final pressed = _pressedClip(drag);
    if (pressed == null) return;
    // A fade handle let go: the fade at that end is the length the handle
    // was dragged to, and none at all back at the corner.
    if (drag.fade case final into?) {
      final ui = _ui!;
      final fps = ui.model.fps;
      final frames = cutFadeFrames(pressed, fps, into: into, shift: drag.shift);
      if (frames == cutFadeFrames(pressed, fps, into: into)) return;
      _transition(ui, comp, pressed, endEdge: !into, frames: frames);
      return;
    }
    final linked = _linked;
    final result = _cut(() => switch ((drag.tool, drag.grab)) {
          (CutTool.roll, _) => comp.cutRoll(
              clip: pressed.id,
              endEdge: drag.grab == BarGrab.trimOut,
              toFrame: drag.editPoint! + drag.shift,
              linked: linked,
            ),
          (CutTool.slip, _) =>
            comp.cutSlip(clip: pressed.id, byFrames: drag.shift, linked: linked),
          (CutTool.slide, _) => comp.cutSlide(
              clip: pressed.id, byFrames: drag.shift, linked: linked),
          (_, BarGrab.trimIn) => comp.cutTrim(
              clip: pressed.id,
              startFrame: pressed.startFrame + drag.shift,
              endFrame: pressed.endFrame,
              ripple: drag.tool == CutTool.ripple,
              linked: linked,
            ),
          (_, BarGrab.trimOut) => comp.cutTrim(
              clip: pressed.id,
              startFrame: pressed.startFrame,
              endFrame: pressed.endFrame + drag.shift,
              ripple: drag.tool == CutTool.ripple,
              linked: linked,
            ),
          // Alt held at the press duplicates: the same move, made by copies.
          (_, BarGrab.move) =>
            (_duplicating ? comp.cutDuplicate : comp.cutMove)(
              clips: [
                for (final id in drag.clips)
                  if (_trackOfClip[id] case final at?)
                    if (_tracks[at].clip(id) case final c?) c.id,
              ],
              grabbed: pressed.id,
              byFrames: drag.shift,
              target: _moveTarget(pressed, drag.trackShift),
              linked: linked,
            ),
        });
    _afterCut(result);
  }

  /// The clip the press began on, which [_press] no longer holds once the
  /// pointer is up: what a move of several is grabbed by, and what an edge
  /// gesture acts on.
  BridgeClip? _pressedClip(CutDrag drag) {
    final id = _lastPressed;
    if (id == null || !drag.clips.contains(id)) return null;
    final at = _trackOfClip[id];
    return at == null ? null : _tracks[at].clip(id);
  }

  /// The id of the clip the last press began on.
  String? _lastPressed;

  /// The layer a moved clip was dropped on, or null to stay on its own.
  LayerReference? _moveTarget(BridgeClip grabbed, int trackShift) {
    if (trackShift == 0) return null;
    final at = _trackOfClip[grabbed.id.toString()];
    if (at == null) return null;
    final target = at + trackShift;
    if (target < 0 || target >= _tracks.length) return null;
    return _tracks[target].entry.layer;
  }

  /// The razor, aimed at the track that was clicked, or with Shift at every
  /// unlocked track, which an empty list means to the engine.
  void _razorCut(BridgeLayerEntry clicked, int frame) {
    final comp = _ui?.selectedComp;
    if (comp == null) return;
    _afterCut(_cut(() => comp.cutRazor(
          layers: HardwareKeyboard.instance.isShiftPressed
              ? const []
              : [clicked.layer],
          atFrame: frame,
          linked: _linked,
        )));
  }

  /// A right click: on a clip, its delete and ripple delete; on the room
  /// between clips, closing the gap.
  Future<void> _menu(LumitUiState ui, CutTrack track, BridgeClip? clip,
      Offset at, Offset global) async {
    final comp = ui.selectedComp;
    if (comp == null) return;
    if (clip == null) {
      if (track.clips.isEmpty) return;
      final frame = _laneAxis.frameAt(at.dx);
      await showMenuAt<void>(
        context: context,
        position: global,
        width: 160,
        rows: (close) => [
          MenuRow(
            key: const ValueKey('ctl-menu-close-gap'),
            onPressed: () {
              close(null);
              _afterCut(_cut(() =>
                  comp.cutCloseGap(layer: track.entry.layer, atFrame: frame)));
            },
            child: Text(l10n.cutCloseGap),
          ),
        ],
      );
      return;
    }
    // The menu acts on the selection the clip is in, or on the clip alone.
    final id = clip.id.toString();
    if (!_selected.contains(id)) setState(() => _selected = _withMates(id));
    // The transitions, as fits what is under the pointer: a dissolve on the
    // nearer edit point where a neighbour meets it, and a fade at each end
    // nothing meets; each taken off again where it is already there.
    final frame = _laneAxis.frameAtExact(at.dx);
    final start = clip.startFrame.toInt();
    final end = clip.endFrame.toInt();
    final endEdge = frame - start > end - frame;
    final across = cutClipAcross(track, clip, endEdge: endEdge) != null;
    final dissolved = across && cutEdgeOverlaps(track, clip, endEdge: endEdge);
    final second = _secondFrames(ui);
    MenuRow transition(void Function(void) close, String key, String label,
            {required bool endEdge, required int frames}) =>
        MenuRow(
          key: ValueKey(key),
          onPressed: () {
            close(null);
            _transition(ui, comp, clip, endEdge: endEdge, frames: frames);
          },
          child: Text(label),
        );
    await showMenuAt<void>(
      context: context,
      position: global,
      width: 180,
      rows: (close) => [
        if (across)
          transition(
            close,
            'ctl-menu-dissolve',
            dissolved ? l10n.cutRemoveDissolve : l10n.cutAddDissolve,
            endEdge: endEdge,
            frames: dissolved ? 0 : second,
          ),
        if (cutClipAcross(track, clip, endEdge: false) == null)
          transition(
            close,
            'ctl-menu-fade-in',
            clip.fadeIn.seconds > 0 ? l10n.cutRemoveFadeIn : l10n.cutFadeIn,
            endEdge: false,
            frames: clip.fadeIn.seconds > 0 ? 0 : second,
          ),
        if (cutClipAcross(track, clip, endEdge: true) == null)
          transition(
            close,
            'ctl-menu-fade-out',
            clip.fadeOut.seconds > 0 ? l10n.cutRemoveFadeOut : l10n.cutFadeOut,
            endEdge: true,
            frames: clip.fadeOut.seconds > 0 ? 0 : second,
          ),
        MenuRow(
          key: const ValueKey('ctl-menu-open-comp'),
          onPressed: () {
            close(null);
            _openAsComposition(ui, comp, clip);
          },
          child: Text(l10n.cutOpenAsComposition),
        ),
        MenuRow(
          key: const ValueKey('ctl-menu-delete'),
          onPressed: () {
            close(null);
            _deleteSelected(ripple: false);
          },
          child: Text(l10n.clipDelete),
        ),
        MenuRow(
          key: const ValueKey('ctl-menu-ripple-delete'),
          onPressed: () {
            close(null);
            _deleteSelected(ripple: true);
          },
          child: Text(l10n.cutRippleDelete),
        ),
      ],
    );
  }

  /// Where [global] falls on the lanes' own scrolled content.
  Offset? _pointOnLanes(Offset global) {
    final box = _laneContent.currentContext?.findRenderObject();
    return box is RenderBox ? box.globalToLocal(global) : null;
  }

  /// Footage let go on the table: the whole of it put down on the track it
  /// landed on at the frame under the pointer, overwriting what is there, or
  /// on a new track when it landed on none. Each item is one Cut call; the
  /// first refusal is said and stops the rest.
  Future<void> _dropFootage(CompositionReference comp,
      List<FootageReference> footage, Offset global) async {
    final ui = _ui;
    if (ui == null) return;
    final at = _pointOnLanes(global);
    final index = at == null ? null : cutTrackAt(_tracks, at.dy, _trackHeight);
    final track = index == null ? null : _tracks[index];
    final target = track == null || track.kind == CutTrackKind.plain
        ? null
        : track.entry.layer;
    var frame = at == null ? ui.playheadFrame.value : _laneAxis.frameAt(at.dx);
    for (final f in footage) {
      // The item's length is what the Project panel has usually probed
      // already. One it has not is probed here, and one that will not probe
      // has no length to place.
      var facts = ui.itemFacts(f);
      try {
        facts ??= await f.mediaInfo();
      } catch (_) {
        continue;
      }
      if (facts == null || !mounted) continue;
      // A still has no length of its own, so it takes five seconds.
      final length = facts.duration.num > 0 && facts.duration.den > 0
          ? facts.duration
          : const BridgeRational(num: 5, den: 1);
      final result = _cut(() => comp.cutPlace(
            target: target,
            footage: f,
            sourceIn: const BridgeRational(num: 0, den: 1),
            sourceOut: length,
            atFrame: frame,
            insert: false,
            linked: _linked,
          ));
      _afterCut(result);
      if (result != BridgeCutResult.done) return;
      // Several files land one after another, in the order they were picked.
      frame += (length.num / length.den * ui.model.fps).round();
    }
  }

  // ---------------------------------------------------------------- the build

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context);
    final comp = ui.selectedComp;
    if (comp == null) {
      return PlaceholderPanel(
        icon: LumitIcon.razor,
        title: l10n.panelCutTimeline,
        hint: l10n.selectACompositionFirst,
      );
    }
    // Everything drawn comes from the read model: zero bridge calls per
    // rebuild.
    return ListenableBuilder(
      listenable: ui.model,
      builder: (context, _) => _body(context, ui, comp),
    );
  }

  Widget _body(
      BuildContext context, LumitUiState ui, CompositionReference comp) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    _animationLevel = scope.animationLevel;
    final frames = ui.model.durationFrames;
    final fps = ui.model.fps;
    _bindTools(ui);
    _refreshTracks(ui.model, comp);
    _setNameStyle(t.small.copyWith(color: t.textSecondary));
    _trackHeight = t.density.laneRow * 2;
    final heights = [for (final _ in _tracks) _trackHeight];

    final revision = ui.model.revision;
    if (_workArea == null || revision != _workRevision || comp != _workComp) {
      _workRevision = revision;
      _workComp = comp;
      _workArea = workAreaFrames(comp);
    }
    final work = _workPreview.value ?? _workArea!;
    _snapTargets = snapTargetsOf(
      layers: [for (final track in _tracks) track.entry],
      compMarkers: markersOf(comp),
      keyRows: const [],
      playheadFrame: ui.playheadFrame.value,
      work: work,
      fps: fps,
    );
    _queueRefresh();

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        CompTabsFrb(
          state: Provider.of<LumitState>(context, listen: false),
          uiState: ui,
          onExport: () => exportFrb(context),
          title: l10n.panelCutTimeline,
        ),
        _strip(t, ui),
        Expanded(
          child: DragTarget<Object>(
            onWillAcceptWithDetails: (details) =>
                details.data is FootageDragData,
            onAcceptWithDetails: (details) async {
              if (details.data case FootageDragData(:final footage)) {
                await _dropFootage(comp, footage, details.offset);
              }
            },
            builder: (context, candidate, _) => Container(
              // Always a decoration, clear until something is over it:
              // adding one only then rebuilt the lanes from nothing, which
              // lost their scroll and had two scroll views on one controller
              // for a frame.
              foregroundDecoration: BoxDecoration(
                  border: Border.all(
                      color: candidate.isEmpty
                          ? t.accent.withValues(alpha: 0)
                          : t.accent,
                      width: 2)),
              child: LayoutBuilder(
                builder: (context, box) {
                  final outlineWidth = min(
                    cutTimelineOutlineWidth,
                    max(100.0, box.maxWidth - scrollGutterWidth - _laneFloor),
                  );
                  final laneViewport =
                      (box.maxWidth - outlineWidth - scrollGutterWidth)
                          .clamp(1.0, 1e6);
                  _laneFrames = frames;
                  _zoomMotion.max = _maxZoom;
                  _pullZoomBackToCeiling();
                  _laneViewport = laneViewport;
                  return ScrollConfiguration(
                    behavior: ScrollConfiguration.of(context).copyWith(
                        dragDevices: const {PointerDeviceKind.trackpad},
                        scrollbars: false),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        _outlineHalf(t, ui, comp,
                            heights: heights, width: outlineWidth),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              TimelineNavigator(
                                trailing: scrollGutterWidth,
                                frames: frames,
                                zoom: _zoomMotion,
                                hScroll: _hLane,
                                playhead: ui.playheadFrame,
                                onWindow: _navigateTo,
                                onWindowEnd: _zoomDragEnd,
                              ),
                              Expanded(
                                // Only this half rebuilds when the zoom
                                // moves.
                                child: ListenableBuilder(
                                  listenable: _zoomMotion,
                                  builder: (context, _) => _laneHalf(
                                    t,
                                    ui,
                                    comp,
                                    axis: TimelineAxis(
                                        frames: frames,
                                        width: laneViewport * _zoom),
                                    heights: heights,
                                    work: work,
                                    frames: frames,
                                    fps: fps,
                                  ),
                                ),
                              ),
                            ],
                          ),
                        ),
                      ],
                    ),
                  );
                },
              ),
            ),
          ),
        ),
      ],
    );
  }

  // --------------------------------------------------------------- the strip

  /// The strip across the top: the clock at the left, the tools, and Snap
  /// and Linked at the right.
  Widget _strip(LumitTheme t, LumitUiState ui) {
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    final lastFrame = ui.model.durationFrames - 1;
    final clockFace = t.mono.copyWith(fontSize: 11, color: t.textPrimary);
    final tool = _toolOf(ui);
    Widget word(String key, String label,
            {required bool on, required VoidCallback onPressed, String? tip}) =>
        LumitTooltip(
          message: tip ?? label,
          child: HouseButton(
            key: ValueKey<String>(key),
            small: true,
            frameless: !on,
            active: on,
            padding: const EdgeInsets.symmetric(horizontal: 6),
            onPressed: onPressed,
            // On Lantern's filled pill the word takes the pill's own ink.
            child: Text(label,
                style: t.small.copyWith(
                    color: !on
                        ? t.textSecondary
                        : t.shape == ThemeShape.lantern
                            ? accentInk(t)
                            : t.textPrimary)),
          ),
        );
    return Container(
      height: t.density.timelineChromeRow,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      child: Row(
        children: [
          Expanded(
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: [
                  ValueListenableBuilder<int>(
                    valueListenable: ui.playheadFrame,
                    builder: (context, frame, _) => timelineChromeControl(
                        t,
                        TimeReadout(
                          key: const ValueKey('ctl-timecode'),
                          frame: frame,
                          format: (f) => timecodeOfRate(f, fpsNum, fpsDen),
                          widthChars: timecodeChars(fpsNum, fpsDen),
                          style: clockFace,
                          parse: (text) =>
                              framesOfTimecode(text, fpsNum, fpsDen),
                          onCommit: ui.scrubTo,
                          minFrame: 0,
                          maxFrame: lastFrame,
                          tooltip: l10n.tipPlayheadTime,
                          well: true,
                        )),
                  ),
                  const SizedBox(width: outlineGap),
                  for (final each in CutTool.values)
                    word(
                      'ctl-tool-${each.name}',
                      switch (each) {
                        CutTool.select => l10n.cutToolSelect,
                        CutTool.razor => l10n.cutToolRazor,
                        CutTool.ripple => l10n.cutToolRipple,
                        CutTool.roll => l10n.cutToolRoll,
                        CutTool.slip => l10n.cutToolSlip,
                        CutTool.slide => l10n.cutToolSlide,
                      },
                      on: tool == each,
                      onPressed: () => _armTool(ui, each),
                    ),
                ],
              ),
            ),
          ),
          word(
            'ctl-snap',
            l10n.cutSnap,
            on: _snap,
            tip: _snap ? l10n.tipSnapOn : l10n.tipSnapOff,
            onPressed: () => setState(() => _snap = !_snap),
          ),
          const SizedBox(width: 4),
          word(
            'ctl-linked',
            l10n.cutLinked,
            on: _linked,
            onPressed: () => ui.cutLinked.value = !ui.cutLinked.value,
          ),
        ],
      ),
    );
  }

  // ------------------------------------------------------------- the outline

  Widget _outlineHalf(
    LumitTheme t,
    LumitUiState ui,
    CompositionReference comp, {
    required List<double> heights,
    required double width,
  }) {
    final divider = cutPictureCount(_tracks) * _trackHeight;
    return SizedBox(
      width: width,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // Level with the lane side's navigator and ruler, so both halves
          // spend the same height above their first track.
          Container(
            height: t.density.navigatorBand + t.density.ruler,
            color: t.surface1,
          ),
          Expanded(
            child: Stack(
              children: [
                LayoutBuilder(
                  builder: (context, box) => SingleChildScrollView(
                    controller: _vOutline,
                    child: ConstrainedBox(
                      constraints: BoxConstraints(minHeight: box.maxHeight),
                      child: LazyBlocks(
                        key: const ValueKey<String>('ctl-outline-blocks'),
                        controller: _vOutline,
                        heights: heights,
                        viewport: box.maxHeight,
                        builder: (context, i) =>
                            _trackHead(t, ui, comp, _tracks[i]),
                      ),
                    ),
                  ),
                ),
                Positioned.fill(
                  child: IgnorePointer(
                    child: AnimatedBuilder(
                      animation: _vOutline,
                      builder: (context, _) {
                        final scrolled = positionOf(_vOutline)?.pixels ?? 0;
                        return Stack(children: [
                          Positioned.fill(
                            child: CustomPaint(
                              painter: RowDividerPainter(
                                step: _trackHeight,
                                colour: rowSeamColour(t),
                                phase: -(scrolled % _trackHeight),
                              ),
                            ),
                          ),
                          if (_tracks.any((track) => track.sound))
                            _divider(t, divider - scrolled),
                        ]);
                      },
                    ),
                  ),
                ),
              ],
            ),
          ),
          SizedBox(height: t.density.secondaryRow),
        ],
      ),
    );
  }

  /// The heavier line between the picture tracks and the sound tracks.
  Widget _divider(LumitTheme t, double y) => Positioned(
        left: 0,
        right: 0,
        top: y - 1,
        height: 2,
        child: IgnorePointer(child: ColoredBox(color: t.hairlineStrong)),
      );

  /// One switch cell, written as the other timelines write theirs: one
  /// `setSwitchOnLayers`, and so one undo step.
  Widget _switch(
    LumitTheme t,
    CompositionReference comp,
    CutTrack track,
    String name,
    bool on,
    BridgeLayerSwitch which, {
    required String mark,
    required String offMark,
    required String tip,
  }) =>
      LumitTooltip(
        message: tip,
        child: GestureDetector(
          key: ValueKey<String>('ctl-$name-${track.id}'),
          behavior: HitTestBehavior.opaque,
          onTap: () {
            try {
              comp.setSwitchOnLayers(
                clicked: track.entry.layer.internallayerId,
                layers: [track.entry.layer.internallayerId],
                switch_: which,
                on_: !on,
              );
            } catch (_) {
              // A locked layer refuses, and quietly.
            }
            _afterWrite();
          },
          child: SizedBox(
            width: switchCellWidth,
            height: _trackHeight,
            child: Center(
              child: glyph.LumitIcon(on ? mark : offMark,
                  size: iconSize, colour: on ? t.textPrimary : t.textMuted),
            ),
          ),
        ),
      );

  /// A track's head: its name, and the switches its kind can use. A press on
  /// the name picks the layer, so the Effect controls panel follows it.
  Widget _trackHead(LumitTheme t, LumitUiState ui, CompositionReference comp,
      CutTrack track) {
    final info = track.entry.info;
    final s = info.switches;
    return SizedBox(
      key: ValueKey<String>('ctl-row-${track.id}'),
      height: _trackHeight,
      child: Padding(
        padding: const EdgeInsets.only(left: outlineGap, right: 4),
        child: Row(children: [
          Container(
            width: 6,
            height: 6,
            decoration: BoxDecoration(
              color: t.labelColour(info.label),
              borderRadius: BorderRadius.circular(3),
            ),
          ),
          const SizedBox(width: outlineGap),
          Expanded(
            child: Listener(
              behavior: HitTestBehavior.opaque,
              onPointerDown: (_) => ui.setSelection([track.entry.layer]),
              child: ValueListenableBuilder<List<LayerReference>>(
                valueListenable: ui.selectedLayers,
                builder: (context, picked, _) => Text(
                  info.name,
                  key: ValueKey<String>('ctl-name-${track.id}'),
                  style: picked.any((l) =>
                          l.internallayerId ==
                          track.entry.layer.internallayerId)
                      ? t.bodyPrimary
                      : t.body,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ),
          ),
          if (track.sound) ...[
            _switch(t, comp, track, 'audible', s.audible,
                BridgeLayerSwitch.audible,
                mark: LumitIcons.audio,
                offMark: LumitIcons.muted,
                tip: s.audible ? l10n.switchAudible : l10n.switchMuted),
            _switch(t, comp, track, 'solo', s.solo, BridgeLayerSwitch.solo,
                mark: LumitIcons.solo,
                offMark: LumitIcons.solo,
                tip: s.solo ? l10n.switchSoloed : l10n.switchSolo),
          ] else
            _switch(t, comp, track, 'visible', s.visible,
                BridgeLayerSwitch.visible,
                mark: LumitIcons.visible,
                offMark: LumitIcons.hidden,
                tip: s.visible ? l10n.switchVisible : l10n.switchHidden),
          _switch(t, comp, track, 'locked', s.locked, BridgeLayerSwitch.locked,
              mark: LumitIcons.lock,
              offMark: LumitIcons.unlocked,
              tip: s.locked ? l10n.switchLocked : l10n.switchLock),
        ]),
      ),
    );
  }

  // ---------------------------------------------------------------- the lanes

  Widget _laneHalf(
    LumitTheme t,
    LumitUiState ui,
    CompositionReference comp, {
    required TimelineAxis axis,
    required List<double> heights,
    required ({int start, int end, bool whole}) work,
    required int frames,
    required double fps,
  }) =>
      Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Expanded(
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Expanded(
                  child: SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    controller: _hLane,
                    child: SizedBox(
                      width: axis.width,
                      child: _laneArea(t, ui, comp,
                          axis: axis,
                          heights: heights,
                          work: work,
                          frames: frames,
                          fps: fps),
                    ),
                  ),
                ),
                SizedBox(
                  width: scrollGutterWidth,
                  child: Column(
                    children: [
                      Container(
                          height: t.density.ruler, color: t.timelineOutOfRange),
                      Expanded(child: GutterScrollbar(controller: _vLane)),
                    ],
                  ),
                ),
              ],
            ),
          ),
          // The zoom slider and the scrollbar. Snap stands on the strip.
          LaneBottomBar(
            zoom: _zoomMotion.target,
            maxZoom: _maxZoom,
            hScroll: _hLane,
            onZoom: _setZoom,
            onZoomLive: (z) => _setZoom(z, fly: false),
            onZoomDragStart: _zoomDragStart,
            onZoomDragEnd: _zoomDragEnd,
            perFrame: axis.perFrame,
            fps: fps,
          ),
        ],
      );

  Widget _laneArea(
    LumitTheme t,
    LumitUiState ui,
    CompositionReference comp, {
    required TimelineAxis axis,
    required List<double> heights,
    required ({int start, int end, bool whole}) work,
    required int frames,
    required double fps,
  }) {
    final razor = _toolOf(ui) == CutTool.razor;
    final colours = (
      edge: t.surface0,
      picked: t.shape == ThemeShape.desk ? t.accent : t.textPrimary,
      cross: t.textSecondary,
      lift: t.textPrimary,
      // A fade's ramp in the curve colours the Audio timeline draws it in,
      // and its handle in the Audio timeline's own.
      rising: t.curve.first,
      falling: t.curve.length > 1 ? t.curve[1] : t.curve.first,
      handle: t.textSecondary,
      handleFill: t.surface1,
      wave: t.waveform,
    );
    final style = WaveformStyle(
      multiwave: true,
      sqrtScale: true,
      fromBottom: ui.workspace.interface.waveformsFromBottom,
    );
    final repaint = Listenable.merge([_drag, _hover, _hLane, _pictures]);
    final divider = cutPictureCount(_tracks) * _trackHeight;
    return RazorOverlay(
      active: razor,
      snapX: (x) => axis.xOf(_razorFrameAt(x)),
      mark: t.textPrimary,
      outline: t.surface0,
      child: Stack(
        children: [
          Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              TimelineRuler(
                comp: comp,
                axis: axis,
                fps: fps,
                height: t.density.ruler,
                work: work,
                onSeek: (f) =>
                    ui.scrubTo(f.clamp(0, frames == 0 ? 0 : frames - 1)),
                onWorkArea: (span) {
                  comp.setWorkArea(span: span);
                  _afterWrite();
                },
                onWorkPreview: (span) => _workPreview.value = span,
                onMarkersChanged: _afterWrite,
                snapTargets: _snapTargets,
                magnet: _snap,
                cache: TimelineCacheBar(
                    comp: comp, axis: axis, revision: _cacheRevision!),
              ),
              Expanded(
                child: LayoutBuilder(
                  builder: (context, box) {
                    _laneHeight = box.maxHeight;
                    return SingleChildScrollView(
                      controller: _vLane,
                      child: ConstrainedBox(
                        constraints: BoxConstraints(minHeight: box.maxHeight),
                        child: Listener(
                          onPointerSignal: (event) {
                            if (event is! PointerScrollEvent) return;
                            final keymap = ui.keymap;
                            if (!keymap.wheelHeld(BridgeWheelAction.zoomTime) &&
                                !keymap.wheelHeld(
                                    BridgeWheelAction.scrollSideways)) {
                              return;
                            }
                            GestureBinding.instance.pointerSignalResolver
                                .register(event, (resolved) {
                              if (resolved is PointerScrollEvent) {
                                _wheel(resolved, resolved.localPosition.dx,
                                    axis);
                              }
                            });
                          },
                          child: Stack(
                            // The box a released drag and a dropped file are
                            // measured in: x along the axis, y down from the
                            // top of the first track.
                            key: _laneContent,
                            children: [
                              WorkAreaGround(
                                key: const ValueKey<String>('ctl-lane-ground'),
                                preview: _workPreview,
                                committed: work,
                                axis: axis,
                                inside: Color.alphaBlend(
                                    t.animated.withValues(
                                        alpha: workAreaLaneFillAlpha),
                                    t.surface1),
                                outside: t.timelineOutOfRange,
                                edge: workAreaEdgeColour(t),
                              ),
                              if (_tracks.isEmpty)
                                Positioned.fill(
                                  child: Center(
                                    child: Text(l10n.cutNoTracks,
                                        key: const ValueKey('ctl-empty'),
                                        style: t.small),
                                  ),
                                ),
                              LazyBlocks(
                                key: const ValueKey<String>('ctl-lane-blocks'),
                                controller: _vLane,
                                heights: heights,
                                viewport: box.maxHeight,
                                builder: (context, i) => RepaintBoundary(
                                  child: CustomPaint(
                                    key: ValueKey<String>(
                                        'ctl-lane-${_tracks[i].id}'),
                                    size: Size(axis.width, _trackHeight),
                                    painter: CutTrackPainter(
                                      track: _tracks[i],
                                      index: i,
                                      tracks: _tracks,
                                      axis: axis,
                                      fps: fps,
                                      hScroll: _hLane,
                                      selected: _selected,
                                      drag: _drag,
                                      hover: _hover,
                                      thumbs: _thumbs,
                                      peaks: _peaks,
                                      compGlyph: _compGlyph,
                                      nameOf: _nameOf,
                                      handles: _toolOf(ui) == CutTool.select,
                                      colours: (
                                        label: t.labelColour(
                                            _tracks[i].entry.info.label),
                                        edge: colours.edge,
                                        picked: colours.picked,
                                        cross: colours.cross,
                                        lift: colours.lift,
                                        rising: colours.rising,
                                        falling: colours.falling,
                                        handle: colours.handle,
                                        handleFill: colours.handleFill,
                                        wave: colours.wave,
                                      ),
                                      style: style,
                                      radius: clipRadius(t),
                                      repaint: repaint,
                                    ),
                                  ),
                                ),
                              ),
                              Positioned.fill(
                                child: IgnorePointer(
                                  child: CustomPaint(
                                    painter: RowDividerPainter(
                                      step: _trackHeight,
                                      colour: rowSeamColour(t),
                                    ),
                                  ),
                                ),
                              ),
                              if (_tracks.any((track) => track.sound))
                                _divider(t, divider),
                              // The box a marquee sweeps, over the clips.
                              ValueListenableBuilder<Rect?>(
                                valueListenable: _marquee,
                                builder: (context, rect, _) => rect == null
                                    ? const SizedBox.shrink()
                                    : Positioned.fromRect(
                                        rect: rect, child: const MarqueeBox()),
                              ),
                              // What the drag landed on, marked while it
                              // holds it: the hairline the Timeline draws
                              // through a caught target, down every track.
                              ValueListenableBuilder<SnapTarget?>(
                                valueListenable: _caught,
                                builder: (context, caught, _) => caught == null
                                    ? const SizedBox.shrink()
                                    : Positioned(
                                        key: const ValueKey<String>(
                                            'ctl-snap-caught'),
                                        left: axis.xOf(caught.frame) - 0.5,
                                        top: 0,
                                        bottom: 0,
                                        width: 1,
                                        child: IgnorePointer(
                                            child:
                                                ColoredBox(color: t.accent)),
                                      ),
                              ),
                              // The readout beside the pointer while a clip
                              // is in hand: how far it has gone, signed, in
                              // the pill the Timeline's drags carry.
                              ValueListenableBuilder<
                                  ({Offset at, int row, String text})?>(
                                valueListenable: _hint,
                                builder: (context, hint, _) {
                                  if (hint == null) {
                                    return const SizedBox.shrink();
                                  }
                                  final x = hint.at.dx;
                                  final pill = hint.text.length * 5.0 + 8;
                                  return Positioned(
                                    key: const ValueKey<String>(
                                        'ctl-drag-hint'),
                                    left: x + 8 + pill > axis.width
                                        ? x - 8 - pill
                                        : x + 8,
                                    top: hint.row * _trackHeight + 1,
                                    child: HintPill(text: hint.text),
                                  );
                                },
                              ),
                              // The one pointer over the lanes: picks, drags,
                              // trims, sweeps and cuts, through the pure hit
                              // tests. A raw listener, so the scroll views
                              // around it cannot win the drag away. Over a
                              // fade handle the cursor is the Audio
                              // timeline's for the same handle.
                              Positioned.fill(
                                child: ValueListenableBuilder<CutHover?>(
                                  valueListenable: _hover,
                                  builder: (context, hover, child) =>
                                      MouseRegion(
                                    cursor: razor
                                        ? SystemMouseCursors.none
                                        : hover?.fade != null
                                            ? SystemMouseCursors
                                                .resizeLeftRight
                                            : MouseCursor.defer,
                                    onHover: (e) =>
                                        _hoverAt(e.localPosition, ui),
                                    onExit: (_) => _hover.value = null,
                                    child: child,
                                  ),
                                  child: Listener(
                                    key: const ValueKey('ctl-lane-pointer'),
                                    behavior: HitTestBehavior.opaque,
                                    onPointerDown: (e) => _down(e, ui),
                                    onPointerMove: _move,
                                    onPointerUp: (e) => _up(e, ui),
                                    onPointerCancel: (_) => _cancel(),
                                  ),
                                ),
                              ),
                            ],
                          ),
                        ),
                      ),
                    );
                  },
                ),
              ),
            ],
          ),
          PlayheadOverlay(playhead: ui.playheadFrame, xOf: axis.xOf),
        ],
      ),
    );
  }
}
