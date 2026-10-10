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

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
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
/// travelled, which track and clip it took hold of, and what the release
/// will do with it.
typedef _Press = ({
  Offset from,
  Offset at,
  bool moved,
  int? track,
  BridgeClip? clip,
  BarGrab grab,
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

  /// The clips that share a link, by clip id: a picture clip and the clip
  /// carrying its sound, which select and travel together while the strip's
  /// Linked switch is on.
  Map<String, Set<String>> _mates = const {};

  /// The tool armed on the strip, other than the razor, which rides the
  /// shell's own tool state so the toolbar and its chord arm it too.
  CutTool _localTool = CutTool.select;

  /// The picked clips by id. The panel's own: nothing about a picked clip is
  /// in the document. Delete takes them while this panel holds the keys.
  Set<String> _selected = {};

  /// The gesture in flight, the drag the painters read off it, and the box a
  /// marquee is sweeping. The notifiers repaint the lanes without a rebuild.
  _Press? _press;
  final ValueNotifier<CutDrag?> _drag = ValueNotifier<CutDrag?>(null);
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
    _releaseKeys();
    _zoomMotion.dispose();
    _vOutline.dispose();
    _vLane.dispose();
    _hLane.dispose();
    _drag.dispose();
    _marquee.dispose();
    _pictures.dispose();
    _workPreview.dispose();
    for (final image in _thumbs.values) {
      image?.dispose();
    }
    for (final text in _names.values) {
      text.dispose();
    }
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

  /// Shift+Delete: the picked clips go and the room they leave closes.
  /// On the hardware keyboard, because a panel holds no focus, and answered
  /// only while this is the focused panel.
  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent || !mounted || lumitModalOpen) return false;
    final ui = _ui;
    if (ui == null || ui.activePanel != Panel.cutTimeline) return false;
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return false;
    }
    if (ui.keymap.actionFor(BridgeKeyContext.cut, event) !=
        'cut.delete.ripple') {
      return false;
    }
    return _deleteSelected(ripple: true);
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
    ui.deleteClaim = _deleteClaim;
  }

  void _releaseKeys() {
    final ui = _ui;
    if (ui == null || !_claimed) return;
    _claimed = false;
    if (ui.deleteClaim == _deleteClaim) ui.deleteClaim = _heldDelete;
  }

  /// Delete: the picked clips go, and leave a gap.
  bool _deleteClaim() {
    if (_ui?.activePanel != Panel.cutTimeline) return false;
    return _deleteSelected(ripple: false);
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
  void _refreshTracks(CompModel model) {
    final revision = model.revision;
    if (revision != null && revision == _tracksRevision) return;
    _tracksRevision = revision;
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
    final hit =
        track == null ? null : cutClipGrabAt(track, _laneAxis, at.dx);
    if (event.buttons == kSecondaryMouseButton) {
      if (track != null) _menu(ui, track, hit?.clip, at, event.position);
      return;
    }
    if (event.buttons != kPrimaryMouseButton) return;
    final keys = HardwareKeyboard.instance;
    final additive = keys.isShiftPressed || keys.isControlPressed;
    final tool = _toolOf(ui);
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
      marquee: hit == null && tool != CutTool.razor,
      additive: additive,
      tool: tool,
    );
    _escape.begin(() {
      _press = null;
      _drag.value = null;
      _marquee.value = null;
    });
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
      marquee: held.marquee,
      additive: held.additive,
      tool: held.tool,
    );
    if (!moved) return;
    if (held.marquee) {
      _marquee.value = Rect.fromPoints(held.from, at);
    } else if (held.clip != null) {
      _drag.value = _dragOf(_press!);
    }
  }

  void _up(PointerUpEvent event, LumitUiState ui) {
    final held = _press;
    final commit = _escape.end();
    _press = null;
    _drag.value = null;
    _marquee.value = null;
    if (held == null || !commit) return;
    if (held.tool == CutTool.razor) {
      if (held.moved || held.track == null) return;
      _razorCut(_tracks[held.track!].entry, _razorFrameAt(held.at.dx).round());
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
    _press = null;
    _drag.value = null;
    _marquee.value = null;
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
  /// caught on a target can still be pulled off it, and the tracks crossed.
  CutDrag _dragOf(_Press held) {
    final clip = held.clip!;
    final axis = _laneAxis;
    final from = clip.startFrame.toInt();
    final to = clip.endFrame.toInt();
    final tool = held.tool;
    final move = held.grab == BarGrab.move && tool != CutTool.slip;
    // The clips that travel: the selection for a move, the pressed clip
    // alone for an edge or a slip or a slide.
    final clips = move && _selected.contains(clip.id.toString())
        ? _selected
        : {clip.id.toString()};
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
    // A slip moves frames inside the box, so nothing on the axis is a place
    // for it to land.
    var shift = snappedDelta(
      rawFrames: axis.framesOfPx(held.at.dx - held.from.dx),
      perFrame: axis.perFrame,
      sources: switch (held.grab) {
        BarGrab.move => [from.toDouble(), to.toDouble()],
        BarGrab.trimIn => [from.toDouble()],
        BarGrab.trimOut => [to.toDouble()],
      },
      targets: _snapTargets.where((t) => !own.contains(t.frame)),
      magnet: _magnet && tool != CutTool.slip,
    ).delta;
    final track = held.track == null ? null : _tracks[held.track!];
    final next = track == null || tool != CutTool.roll
        ? null
        : held.grab == BarGrab.trimOut
            ? cutClipAfter(track, clip)
            : _clipBefore(track, clip);
    shift = cutClampShift(
        clip: clip, grab: held.grab, tool: tool, shift: shift, next: next);
    var trackShift = 0;
    if (move && tool != CutTool.slide && held.track != null) {
      final over = cutTrackAt(_tracks, held.at.dy, _trackHeight);
      trackShift = over == null ? 0 : over - held.track!;
    }
    return (
      tool: tool,
      clips: clips,
      grab: held.grab,
      shift: shift,
      trackShift: trackShift,
      editPoint: tool != CutTool.roll
          ? null
          : held.grab == BarGrab.trimOut
              ? to
              : from,
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
          (_, BarGrab.move) => comp.cutMove(
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
    await showMenuAt<void>(
      context: context,
      position: global,
      width: 160,
      rows: (close) => [
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
    _refreshTracks(ui.model);
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
              foregroundDecoration: candidate.isEmpty
                  ? null
                  : BoxDecoration(
                      border: Border.all(color: t.accent, width: 2)),
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
            child: Text(label,
                style: t.small
                    .copyWith(color: on ? t.textPrimary : t.textSecondary)),
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
      wave: t.waveform,
    );
    final style = WaveformStyle(
      multiwave: true,
      sqrtScale: true,
      fromBottom: ui.workspace.interface.waveformsFromBottom,
    );
    final repaint = Listenable.merge([_drag, _hLane, _pictures]);
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
                                      axis: axis,
                                      fps: fps,
                                      hScroll: _hLane,
                                      selected: _selected,
                                      drag: _drag,
                                      thumbs: _thumbs,
                                      peaks: _peaks,
                                      nameOf: _nameOf,
                                      colours: (
                                        label: t.labelColour(
                                            _tracks[i].entry.info.label),
                                        edge: colours.edge,
                                        picked: colours.picked,
                                        cross: colours.cross,
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
                              // The one pointer over the lanes: picks, drags,
                              // trims, sweeps and cuts, through the pure hit
                              // tests. A raw listener, so the scroll views
                              // around it cannot win the drag away.
                              Positioned.fill(
                                child: MouseRegion(
                                  cursor: razor
                                      ? SystemMouseCursors.none
                                      : MouseCursor.defer,
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
