// The Audio timeline panel (docs/impl/audio-timeline.md §5): the sound table.
//
// Two halves over one time axis, like the layer Timeline - an outline of
// tracks on the left, the lanes on the right - but the rows are what can be
// heard rather than what is in the stack, and a track stands two lane rows
// tall, or as many as its own bottom edge has been dragged to. It takes the
// Timeline's place in the Audio arrangement and keeps its own zoom, scroll,
// twirls and lane modes, so the two tables can stand on screen together
// without moving each other about.
//
// **What is here.** The tracks, the faded picture rows with their Detach audio
// button, the lane-mode chip, the twirl on to Volume and the track's audio
// effects, the layer's own wave or spectrogram, and the clips on a converted
// track with their gain lines, trims, slides, cross-track moves, drops, razor
// and menu. Fades and clip effects come after this, in the
// packages the note orders.
//
// Everything drawn rides in on the read model; the bridge is crossed from
// gestures and from the claimed-key peaks fetch alone.

import 'dart:math';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show exportFrb;
import '../state/comp_model.dart';
import '../state/comp_time.dart';
import '../state/dock.dart';
import '../state/drag_payloads.dart';
import '../state/timecode.dart';
import '../state/timeline_columns.dart';
import '../state/tools.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import '../widgets/marquee.dart';
import '../widgets/smooth_zoom.dart';
import '../widgets/time_readout.dart';
import '../widgets/zoom_anchored_scroll.dart';
import 'audio_timeline_clips_frb.dart';
import 'audio_timeline_fades_frb.dart';
import 'audio_timeline_rows_frb.dart';
import 'clip_fade_popover.dart';
import 'clip_fades.dart';
import 'easing_curve.dart' show EasingCurve;
import 'effect_controls_panel_frb.dart' show showAddEffectMenu;
import 'graph_channels.dart' show GraphChannel, graphChannels;
import 'graph_clipboard.dart'
    show copySelectedKeys, graphKeyClipboard, pasteKeysAtPlayhead;
import 'graph_edits.dart' show applyEasingToSelection;
import 'graph_maths.dart' show rationalSeconds;
import 'key_block.dart' show KeyStretch;
import 'layer_fold_frb.dart';
import 'placeholder.dart';
import 'spectral_lane_frb.dart';
import 'timeline_bar_frb.dart' show BarGrab;
import 'timeline_extras_frb.dart';
import 'timeline_key_lane_frb.dart';
import 'timeline_lane_area_frb.dart' show SelectedKey, commitKeyGesture;
import 'timeline_lane_bottom_bar_frb.dart';
import 'timeline_layer_rows_frb.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_navigator.dart';
import 'timeline_outline_frb.dart' show GutterScrollbar;
import 'timeline_razor.dart';
import 'timeline_snap.dart';
import 'timeline_toolbar_frb.dart' show timelineChromeControl;
import 'volume_band_frb.dart';
import 'waveform_frb.dart';

/// How wide the outline stands, and so where the seam between the halves is.
/// The board's own 300 across a 1440 drawing; squeezed narrower than that the
/// column gives its room up to the lanes until the panel's floor takes over
/// and the whole thing slides ([panelMinWidth]).
const double audioTimelineOutlineWidth = 300;

/// The narrowest sliver of lane the seam leaves, however tight the panel is: a
/// table of names with no time in it is not this panel.
const double _laneFloor = 60;

/// How narrow the chrome strip's search box may be squeezed before the
/// readouts beside it give up their room instead. A well under this is one you
/// cannot read a track's name in, and a long comp's readouts will squeeze it to
/// nothing given the chance.
const double audioTimelineSearchFloor = 90;

/// Whether the chrome strip can carry the frame count and the comp's length
/// beside the clock: only while the search box keeps its floor with the two of
/// them and their gaps in the row as well. Pure, so the rule can be read off
/// the numbers rather than off a screenshot.
bool audioTimelineChromeCarriesCount({
  required double strip,
  required double clock,
  required double count,
  required double length,
}) =>
    strip - clock - outlineGap - audioTimelineSearchFloor >= count + length + 8;

/// How many frames full zoom-in shows across the lanes - the layer Timeline's
/// own number, so the two tables magnify to the same place.
const int _framesAtFullZoom = 20;

class AudioTimelinePanelFrb extends StatefulWidget {
  const AudioTimelinePanelFrb({super.key});

  @override
  State<AudioTimelinePanelFrb> createState() => _AudioTimelinePanelFrbState();
}

class _AudioTimelinePanelFrbState extends State<AudioTimelinePanelFrb>
    with SingleTickerProviderStateMixin {
  /// What is twirled open: track ids, and the paths of the groups under them.
  /// The panel's own set, not the Timeline's - each table remembers its own
  /// twirls - though the paths are the same strings, so a fold path means the
  /// same thing in both.
  final Set<String> _open = {};

  /// Which picture each track's lane draws, by track id. Wave to begin with,
  /// and the panel's own state: the layer Timeline's three-mode store is not
  /// shared, so a choice here does not change a lane there.
  final Map<String, LaneMode> _laneMode = {};

  /// How many lane rows each track stands on, by track id, and how far a drag
  /// on a row's bottom edge has travelled since the last whole row came off
  /// it. The carry starts again at each press, so no drag spends the travel
  /// another one banked. Two rows unless an edge has been dragged, and the
  /// panel's own state: a height is how this table is being read rather than
  /// anything about the comp, so the next mount opens at two again.
  final Map<String, int> _trackRows = {};
  double _rowCarry = 0;

  /// Which layers carry sound and which have a picture, by id. Both come from
  /// probing the source, so both are asked once per layer and remembered - a
  /// build may never probe.
  final Map<String, bool> _hasAudio = {};
  final Map<String, bool> _hasPicture = {};

  /// Each track's waveform peaks or spectrogram over the stretch the lanes are
  /// showing, and what each was fetched for. Equal keys mean the answer in hand
  /// is still the right one, so nothing is asked twice; a track holds one
  /// picture or the other, never both.
  final Map<String, BridgeAudioPeaks> _peaks = {};
  final Map<String, String> _peakKeys = {};
  final Map<String, BridgeSpectrogram> _spectra = {};
  final Map<String, String> _spectraKeys = {};

  /// Each track's Volume scalar, read once per document revision and carried
  /// down onto the fold rows - it is not in the read model, and a build may not
  /// go and ask for it.
  Map<String, BridgeScalar> _volumeDb = {};
  BigInt? _readsRevision;

  /// The two halves' vertical scrolls, kept in step: they are one table, and a
  /// name that does not line up with its lane is worse than no table at all.
  final ScrollController _vOutline = ScrollController();
  final ScrollController _vLane = ScrollController();
  bool _syncingScroll = false;

  /// The lanes' horizontal scroll, anchored so a zoom holds a frame still while
  /// the content grows underneath it.
  final ZoomAnchoredScrollController _hLane = ZoomAnchoredScrollController();

  late final SmoothZoom _zoomMotion;
  double _zoomAnchorFrame = 0;
  double _zoomAnchorViewportX = 0;
  bool _zoomAnchorHeld = false;
  bool _pullingBackZoom = false;
  AnimationLevel _animationLevel = AnimationLevel.all;

  /// The lanes' viewport width and the comp's length as the last layout knew
  /// them: what a peak window is worked out from, and what the zoom's ceiling
  /// is measured against.
  double _laneViewport = 0;
  int _laneFrames = 1;

  double get _zoom => _zoomMotion.value;
  double get _maxZoom => max(1.0, _laneFrames / _framesAtFullZoom.toDouble());

  /// The clip drag in flight on each track's clip strip, by track id. The
  /// strip writes it and the fade layer stacked over the strip reads it, so the
  /// ramps and the corner handles travel with the box rather than waiting for
  /// the release. One per track, because a drag on one says nothing about
  /// another.
  final Map<String, ValueNotifier<AudioClipDrag?>> _clipDrag = {};

  ValueNotifier<AudioClipDrag?> _clipDragOf(String track) =>
      _clipDrag.putIfAbsent(track, () => ValueNotifier<AudioClipDrag?>(null));

  /// The keyframes in hand on the twirl rows' lanes, and the drag carrying
  /// them. The panel's own, and only the fold rows read it.
  final ValueNotifier<Set<String>> _laneKeys = ValueNotifier<Set<String>>({});
  final ValueNotifier<KeyStretch?> _keyStretch =
      ValueNotifier<KeyStretch?>(null);

  /// Which property rows are picked, so a row draws itself lit. A plain list:
  /// this panel has no graph to colour and no range selection yet.
  List<String> _selectedProperties = const [];

  /// The work area, held between document revisions - reading it is several
  /// bridge calls and only an edit can change the answer - and the span staged
  /// while an edge is being dragged.
  ({int start, int end, bool whole})? _workArea;
  BigInt? _workRevision;
  CompositionReference? _workComp;
  final ValueNotifier<({int start, int end, bool whole})?> _workPreview =
      ValueNotifier<({int start, int end, bool whole})?>(null);

  /// When the render cache may have changed. Merged once, not per build: a
  /// fresh `Listenable` every rebuild makes the cache bar unsubscribe and
  /// resubscribe sixty times a second through a zoom flight.
  Listenable? _cacheRevision;

  LumitUiState? _ui;

  /// The comp this panel has already marked mixed, so it is marked once
  /// however many edits follow the first.
  String? _markedComp;

  /// The chrome strip's search box, and what it says. The tracks are narrowed
  /// to the names carrying it, which is what the layer Timeline's own search
  /// does to its rows.
  final TextEditingController _searchField = TextEditingController();
  String _search = '';

  /// The tracks the last build drew, so a scroll can refresh their peaks
  /// without a rebuild to hand them over.
  List<AudioTrackRow> _lastTracks = const [];

  /// The track being renamed in the outline, and what has been typed into it.
  /// Held here rather than in the row because Enter is the panel's key: the
  /// row draws whichever of the two the panel hands it.
  String? _renaming;
  TextEditingController? _rename;

  /// The picked clip: which track it is on and which clip it is. One at a
  /// time, and the panel's own - nothing about a picked clip is in the
  /// document. Delete takes it while this panel holds the keys.
  ({String track, String clip})? _selectedClip;

  /// The lanes' scrolled content, so a drag or a drop that let go somewhere on
  /// the table can be turned into a track and a frame.
  final GlobalKey _laneContent = GlobalKey();

  /// The toolbar, subscribed to for the razor: the armed tool sits on its own
  /// notifier, so watching the shell's state does not hear about it.
  ToolsState? _boundTools;

  /// Escape's registration, and the claims this panel displaced while it holds
  /// the keys - put back untouched the moment another panel takes them.
  VoidCallback? _escapeRelease;
  bool _claimed = false;
  bool Function()? _heldDelete;
  bool Function()? _heldCopy;
  bool Function()? _heldPaste;
  ValueChanged<EasingCurve>? _heldEasing;

  @override
  void initState() {
    super.initState();
    // Escape gives the picked clip up, below any drag in flight - a gesture
    // being abandoned is the inner thing and answers first.
    _escapeRelease = EscapeLadder.register(EscapeRung.selection, _escapeClaim);
    _zoomMotion = SmoothZoom(vsync: this, initial: 1, min: 1, max: 64)
      ..addListener(_onZoomTick);
    _vOutline.addListener(() => _followScroll(_vOutline, _vLane));
    _vLane.addListener(() => _followScroll(_vLane, _vOutline));
    // Scrolling sideways moves which stretch of audio the lanes show, and a
    // summary is only as detailed as the window it was taken over. Nothing else
    // about the panel changes, so this listens rather than the panel rebuilding
    // on every scrolled pixel.
    _hLane.addListener(_onLaneScroll);
    // Kept rather than looked up again: `dispose` runs after the element is
    // deactivated, where an ancestor lookup is no longer safe.
    _ui = Provider.of<LumitUiState>(context, listen: false);
    _cacheRevision = Listenable.merge([_ui!.frameArrived, _ui!.cacheChanged]);
    _ui!.activePane.addListener(_onActivePanel);
    _onActivePanel();
    _searchField.addListener(() => setState(() => _search = _searchField.text));
    HardwareKeyboard.instance.addHandler(_onKey);
  }

  @override
  void dispose() {
    _escapeRelease?.call();
    _boundTools?.removeListener(_onToolChanged);
    _ui?.activePane.removeListener(_onActivePanel);
    HardwareKeyboard.instance.removeHandler(_onKey);
    _rename?.dispose();
    _releaseKeys();
    _zoomMotion.dispose();
    _vOutline.dispose();
    _vLane.dispose();
    _hLane.dispose();
    _searchField.dispose();
    _laneKeys.dispose();
    _keyStretch.dispose();
    _workPreview.dispose();
    for (final drag in _clipDrag.values) {
      drag.dispose();
    }
    super.dispose();
  }

  void _onToolChanged() {
    if (mounted) setState(() {});
  }

  /// Mark the fronted comp mixed, once. A comp already marked is left alone,
  /// so nothing is written for a comp that has been here before.
  void _markMixed() {
    final comp = _ui?.selectedComp;
    if (comp == null || !mounted) return;
    final id = comp.internalid.toString();
    if (id == _markedComp) return;
    _markedComp = id;
    try {
      if (comp.soundMix()) return;
      comp.setSoundMix(mixed: true);
    } catch (_) {
      // A comp that will not take the mark is not worth an error thrown out
      // of a pointer handler.
    }
  }

  /// Where every edit made in this panel ends: the comp is marked mixed on
  /// the first of them (docs/impl/audio-timeline.md §2), so the layer
  /// Timeline knows to fold its Audio layers behind the Sound mix row, and
  /// the read model is freshened. Looking at a comp writes nothing.
  ///
  /// ponytail: the mark is its own undo step after that first edit, because
  /// this runs once the edit's own undo group has closed. The upgrade is
  /// marking inside the group, which means every write road here opening one.
  void _afterWrite() {
    _markMixed();
    _ui?.model.refresh();
  }

  void _bindTools(LumitUiState ui) {
    if (identical(_boundTools, ui.tools)) return;
    _boundTools?.removeListener(_onToolChanged);
    _boundTools = ui.tools..addListener(_onToolChanged);
  }

  bool _razorArmed(LumitUiState ui) => ui.tools.tool.group == ToolGroup.razor;

  /// Escape with a clip picked lets it go, and nothing else.
  bool _escapeClaim() {
    if (!mounted || _selectedClip == null) return false;
    setState(() => _selectedClip = null);
    return true;
  }

  // ------------------------------------------------------------- the rename

  /// This panel's one keyboard command: **Enter names the picked track**, the
  /// binding the keymap already carries for a layer.
  ///
  /// Registered on the hardware keyboard, because a panel holds no focus, and
  /// answered only while this is the focused panel - the layer Timeline and the
  /// Project panel answer the same key for their own selections, and two
  /// renames on one press is a mess. A dialogue, or a field already being
  /// typed into, keeps its keys.
  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent || !mounted || _renaming != null) return false;
    if (lumitModalOpen) return false;
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return false;
    }
    final ui = _ui;
    if (ui == null || ui.activePanel != Panel.audioTimeline) return false;
    if (ui.keymap.actionFor(BridgeKeyContext.timeline, event) !=
        'layer.rename') {
      return false;
    }
    // One track, and one that is on this table: the selection can hold a layer
    // this panel does not list.
    final picked = ui.selectedLayers.value;
    if (picked.length != 1) return false;
    final id = picked.single.internallayerId.toString();
    for (final track in _lastTracks) {
      if (track.id != id || track.entry.info.switches.locked) continue;
      setState(() {
        _renaming = id;
        _rename = TextEditingController(text: track.entry.info.name);
      });
      return true;
    }
    return false;
  }

  /// Escape, or a rename that has been written: the editor shuts.
  void _cancelRename() {
    if (!mounted || _rename == null) return;
    setState(() {
      _rename?.dispose();
      _rename = null;
      _renaming = null;
    });
  }

  /// Enter, or a click anywhere else: what was typed becomes the layer's name,
  /// through the call the layer Timeline's own rename makes.
  void _commitRename(AudioTrackRow track) {
    if (!mounted || _rename == null) return;
    final text = _rename!.text.trim();
    _cancelRename();
    if (text.isEmpty || text == track.entry.info.name) return;
    track.entry.layer.rename(name: text);
    _afterWrite();
  }

  // ------------------------------------------------------------- the claims

  /// Take the shell's single-slot key claims while this panel is the focused
  /// one, and give them straight back when another panel takes the keys.
  ///
  /// The slots hold one callback each and are set by whichever panel mounted
  /// last, so with both timelines on screen the second to mount silently won
  /// every one of them. Following `activePanel` is the arbitration: what stood
  /// before is held and put back untouched, so the layer Timeline needs no
  /// change at all and gets its own keys back the moment it is clicked in.
  void _onActivePanel() {
    if (_ui?.activePanel == Panel.audioTimeline) {
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
    _heldPaste = ui.pasteClaim;
    _heldEasing = ui.easingApply.value;
    ui.deleteClaim = _deleteClaim;
    ui.copyClaim = _copyKeys;
    ui.pasteClaim = _pasteKeys;
    ui.easingApply.value = _applyEasing;
  }

  void _releaseKeys() {
    final ui = _ui;
    if (ui == null || !_claimed) return;
    _claimed = false;
    // Only what is still ours: a third panel that has taken a slot since keeps
    // it, which is what makes this give-and-take rather than a scramble.
    if (ui.deleteClaim == _deleteClaim) ui.deleteClaim = _heldDelete;
    if (ui.copyClaim == _copyKeys) ui.copyClaim = _heldCopy;
    if (ui.pasteClaim == _pasteKeys) ui.pasteClaim = _heldPaste;
    if (ui.easingApply.value == _applyEasing) {
      ui.easingApply.value = _heldEasing;
    }
  }

  /// Delete: the picked clip.
  bool _deleteClaim() {
    final picked = _selectedClip;
    final ui = _ui;
    if (!mounted || ui == null || picked == null) return false;
    for (final track in _lastTracks) {
      if (track.id != picked.track) continue;
      for (final clip in track.entry.info.clips) {
        if (clip.id.toString() != picked.clip) continue;
        track.entry.layer.deleteClip(clip: clip.id);
        setState(() => _selectedClip = null);
        _afterWrite();
        return true;
      }
    }
    return false;
  }

  /// The channels the picked keyframes sit on, resolved afresh against the read
  /// model - an edit replaces a property's whole animation, so a channel held
  /// across one carries the curve as it was.
  ///
  /// A **clip's** rows resolve to nothing: their paths root under the clip
  /// rather than under a layer, and the shared resolver reads a layer id. A
  /// group header's rows have always answered the same way.
  List<GraphChannel> _keyChannels(LumitUiState ui) => graphChannels(
        layers: ui.model.layers,
        selected: {
          for (final id in _laneKeys.value)
            if (id.lastIndexOf('#') > 0) id.substring(0, id.lastIndexOf('#')),
        }.toList(),
      );

  /// The lane selection said in channel terms: a diamond stands for its row, so
  /// `row#i` fans out to each channel of that path.
  Set<String> _keysOnChannels(List<GraphChannel> channels) {
    final out = <String>{};
    for (final id in _laneKeys.value) {
      final hash = id.lastIndexOf('#');
      if (hash <= 0) continue;
      final path = id.substring(0, hash);
      final index = id.substring(hash + 1);
      for (final channel in channels) {
        if (channel.path == path) out.add('${channel.id}#$index');
      }
    }
    return out;
  }

  /// Copy takes the picked keyframes, and says whether it took any - a copy
  /// that captured nothing must not swallow the chord and leave the last one on
  /// the clipboard for Paste to put down again.
  bool _copyKeys() {
    final ui = _ui;
    final comp = ui?.selectedComp;
    if (!mounted || ui == null || comp == null) return false;
    final channels = _keyChannels(ui);
    final selection = _keysOnChannels(channels);
    if (selection.isEmpty) return false;
    return copySelectedKeys(
      comp: comp,
      channels: channels,
      selectedKeys: selection,
      fps: ui.model.fps,
    );
  }

  /// Paste puts them down at the playhead, on the rows the selection sits on.
  bool _pasteKeys() {
    final ui = _ui;
    if (!mounted || ui == null || graphKeyClipboard.isEmpty) return false;
    final channels = _keyChannels(ui);
    if (channels.isEmpty) return false;
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    pasteKeysAtPlayhead(
      channels: channels,
      playheadFrame: ui.playheadFrame.value,
      fps: ui.model.fps,
      fpsNum: fpsNum,
      fpsDen: fpsDen,
      project: Provider.of<LumitState>(context, listen: false).project,
    ).then((pasted) {
      if (pasted && mounted) _afterWrite();
    });
    return true;
  }

  /// A shape sent from the Easing panel, onto every span the picked keyframes
  /// cover - one press, one undo step.
  void _applyEasing(EasingCurve curve) {
    final ui = _ui;
    if (!mounted || ui == null) return;
    final channels = _keyChannels(ui);
    final selection = _keysOnChannels(channels);
    if (selection.isEmpty) return;
    asOneUndoStep(
      Provider.of<LumitState>(context, listen: false).project,
      () => applyEasingToSelection(
          channels: channels, selectedKeys: selection, curve: curve),
    );
    _afterWrite();
  }

  void _followScroll(ScrollController from, ScrollController to) {
    if (_syncingScroll || !from.hasClients || !to.hasClients) return;
    if ((to.offset - from.offset).abs() < 0.5) return;
    _syncingScroll = true;
    to.jumpTo(from.offset.clamp(0.0, to.position.maxScrollExtent));
    _syncingScroll = false;
  }

  LaneMode _modeOf(String id) => _laneMode[id] ?? LaneMode.wave;

  int _rowsOf(String id) => _trackRows[id] ?? audioTrackMinRows;

  /// The drag on a track's bottom edge, taken a whole lane row at a time so
  /// the outline and the lanes step together and the wave never lands on half
  /// a row. What the pointer has travelled short of the next row is carried,
  /// so a slow drag arrives where a fast one does.
  void _resizeTrack(AudioTrackRow track, double dy) {
    final step = track.rowHeight;
    if (step <= 0) return;
    _rowCarry += dy;
    final by = (_rowCarry / step).truncate();
    if (by == 0) return;
    _rowCarry -= by * step;
    final was = _rowsOf(track.id);
    final rows = (was + by).clamp(audioTrackMinRows, audioTrackMaxRows);
    if (rows == was) return;
    setState(() => _trackRows[track.id] = rows);
  }

  /// The chip: Wave and Spectral, and nothing else. The stack is a picture of
  /// three bands, which is a layer Timeline lane's business.
  void _cycleLaneMode(String id) {
    setState(() => _laneMode[id] =
        _modeOf(id) == LaneMode.wave ? LaneMode.spectral : LaneMode.wave);
  }

  void _toggleOpen(String path) {
    setState(() {
      if (!_open.remove(path)) _open.add(path);
    });
  }

  /// Fill in any layer's has-audio and has-picture answers we do not have, off
  /// the build. Both mean probing the source, so both are asked once per layer
  /// and remembered.
  void _refreshProbes(List<BridgeLayerEntry> layers) {
    for (final entry in layers) {
      final id = entry.layer.internallayerId.toString();
      if (_hasAudio.containsKey(id)) continue;
      // Claim the slot first, so a rebuild mid-probe does not probe twice.
      _hasAudio[id] = false;
      _hasPicture[id] = entry.layer.hasPicture();
      entry.layer.hasAudio().then((has) {
        if (!mounted || _hasAudio[id] == has) return;
        setState(() {
          _hasAudio[id] = has;
          // A track has a Volume scalar to read, and the document has not
          // moved: forget the revision so the next build reads it.
          _readsRevision = null;
        });
      });
    }
  }

  /// The per-track answers the fold rows carry, read once per document
  /// revision. Volume is on the read model, so this is a walk rather than a
  /// crossing - but it still belongs out of the row builds.
  void _refreshReads(CompModel model) {
    final revision = model.revision;
    if (revision != null && revision == _readsRevision) return;
    _readsRevision = revision;
    _volumeDb = {
      for (final entry in model.layers)
        if (_hasAudio[entry.layer.internallayerId.toString()] ?? false)
          entry.layer.internallayerId.toString(): entry.info.volumeDb,
    };
  }

  /// How waveforms draw. The Wave lane draws the three-band stack, the same
  /// three stops the Spectral lane blends, so the two pictures the chip
  /// cycles between share one range of colour. Where the wave sits is the one
  /// setting left here - Settings ▸ Interface ▸ Editing - because the chip
  /// says wave or spectrogram and the Settings multiwave toggle has nothing
  /// to decide.
  WaveformStyle get _waveformStyle => WaveformStyle(
        multiwave: true,
        // On a square-root scale, which is this panel's own: a mix is read
        // here, and a quiet take drawn straight through is a line along the
        // middle of a box the eye cannot tell from silence.
        sqrtScale: true,
        fromBottom: Provider.of<LumitUiState>(context, listen: false)
            .workspace
            .interface
            .waveformsFromBottom,
      );

  /// Fetch each track's picture over the stretch of audio the lanes are showing
  /// right now, so the detail follows the zoom rather than stretching one
  /// coarse summary.
  ///
  /// Called from the build *and* from the lanes' scroll, because scrolling
  /// moves the window without changing anything the panel rebuilds for. The
  /// request rounds itself off, so an ordinary scroll asks nothing new.
  void _refreshPeaks(List<AudioTrackRow> tracks) {
    final ui = _ui;
    if (ui == null) return;
    final frames = ui.model.durationFrames;
    final fps = ui.model.fps;
    final width = _laneViewport * _zoom;
    if (frames <= 0 || fps <= 0 || width <= 0 || _laneViewport <= 0) return;
    final maxOffset = max(0.0, width - _laneViewport);
    final offset =
        _hLane.hasClients ? _hLane.offset.clamp(0.0, maxOffset) : 0.0;
    final secondsPerPixel = frames / fps / width;
    final viewStart = offset * secondsPerPixel;
    final viewEnd = (offset + _laneViewport) * secondsPerPixel;

    final live = <String>{};
    for (final track in tracks) {
      final id = track.id;
      // A converted track draws its clips' own pictures, which are fetched per
      // clip once there are clips to fetch for.
      if (track.entry.info.clips.isNotEmpty) continue;
      live.add(id);
      final startOffset = rationalSeconds(track.entry.info.span.startOffset);
      final request = WaveformRequest.forView(
        startSeconds: viewStart - startOffset,
        endSeconds: viewEnd - startOffset,
        pixels: _laneViewport,
      );
      if (request == null) continue;
      // A retimed layer's buckets are taken through its Retime map, so
      // reshaping the map changes the answer without moving the window. Only a
      // retimed layer pays for that.
      final retimed =
          track.entry.info.retime == null ? '' : '|${ui.model.heldRevision}';
      final key = '${request.key}$retimed';
      if (_modeOf(id) == LaneMode.spectral) {
        _peaks.remove(id);
        _peakKeys.remove(id);
        if (_spectraKeys[id] == key) continue;
        _spectraKeys[id] = key;
        track.entry.layer
            .audioSpectrogram(
          startSeconds: request.startSeconds,
          endSeconds: request.endSeconds,
          columns: request.buckets,
        )
            .then((grid) {
          if (!mounted || _spectraKeys[id] != key) return;
          setState(() => _spectra[id] = grid);
        });
        continue;
      }
      _spectra.remove(id);
      _spectraKeys.remove(id);
      // Claimed before the fetch starts, so a rebuild mid-decode does not ask
      // twice for the same window.
      if (_peakKeys[id] == key) continue;
      _peakKeys[id] = key;
      track.entry.layer
          .audioPeaks(
        startSeconds: request.startSeconds,
        endSeconds: request.endSeconds,
        buckets: request.buckets,
        multiwave: true,
      )
          .then((peaks) {
        // A later window may already have been asked for while this one was
        // decoding; the newest ask wins.
        if (!mounted || _peakKeys[id] != key) return;
        setState(() => _peaks[id] = peaks);
      });
    }
    // A track that has left the list keeps nothing: the window it was fetched
    // for is stale by the time it comes back, and the memory is a whole track's
    // summary.
    _peaks.removeWhere((id, _) => !live.contains(id));
    _peakKeys.removeWhere((id, _) => !live.contains(id));
    _spectra.removeWhere((id, _) => !live.contains(id));
    _spectraKeys.removeWhere((id, _) => !live.contains(id));
  }

  /// The lanes scrolled: the visible window moved, so the pictures may want a
  /// finer summary of somewhere else. Nothing is rebuilt here - the fetch calls
  /// `setState` only when an answer actually arrives.
  void _onLaneScroll() {
    if (_peakKeys.isEmpty && _spectraKeys.isEmpty) return;
    _refreshPeaks(_lastTracks);
  }

  // ---------------------------------------------------------------- the zoom

  void _onZoomTick() {
    if (_laneFrames <= 0) return;
    _hLane.hold(ZoomAnchor(
      frame: _zoomAnchorFrame,
      viewportX: _zoomAnchorViewportX,
      frames: _laneFrames,
      pad: TimelineAxis.pad,
    ));
  }

  /// Point the flight's anchor at the playhead - held where it is if it is on
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

  /// The navigator asked for a window: `start` frames from the left, `span`
  /// frames across. The window's left edge is the anchor and is held for the
  /// length of the gesture, so dragging its right-hand end zooms about its
  /// left-hand one.
  void _navigateTo(double start, double span) {
    if (_laneFrames <= 0 || span <= 0) return;
    _zoomAnchorFrame = start;
    _zoomAnchorViewportX = 0;
    _zoomAnchorHeld = true;
    final want = (_laneFrames / span).clamp(1.0, _maxZoom);
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

  /// Bring a zoom that is past the composition's ceiling back to it, once this
  /// frame has been painted - a zoom notifies its listeners, and doing that
  /// inside a build is `setState` during build.
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

  void _panLanes(Offset delta) {
    _scrollBy(_hLane, -delta.dx);
    _scrollBy(_vLane, -delta.dy);
  }

  void _wheel(PointerScrollEvent event, double contentX, TimelineAxis axis) {
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed) {
      _zoomAnchorViewportX = contentX - (_hLane.hasClients ? _hLane.offset : 0);
      _zoomAnchorFrame = axis.frameAtExact(contentX);
      _zoomMotion.nudge(
        event.scrollDelta.dy < 0 ? 1.2 : 1 / 1.2,
        duration: animationDuration(_animationLevel),
      );
      return;
    }
    if (keys.isShiftPressed) _scrollBy(_hLane, event.scrollDelta.dy);
  }

  // ------------------------------------------------------------ the fold keys

  /// How far along the comp a row's keys are drawn from their own zero.
  ///
  /// Zero for a track's own rows, whose keys are already in comp time. A
  /// **clip's** parameter is keyed from the clip's start, as its Retime is, so
  /// its keys are drawn a clip's start along and written back that far short -
  /// `startFrame` is the clip's place with the layer's own zero already added,
  /// which is exactly the walk the note asks for.
  int _clipShiftOf(AudioTrackRow track, LayerFoldRow row) {
    if (row is! FoldEffectParamRow || row.clip == null) return 0;
    for (final clip in track.entry.info.clips) {
      if (clip.id == row.clip) return clip.startFrame.toInt();
    }
    return 0;
  }

  /// Every keyframe on the twirl rows, with where it sits - the one walk the
  /// marquee, the drag commit and the selection all read, so the three cannot
  /// disagree about which key is where.
  ///
  /// [drawn] is where a clip's keys are measured: on the comp's clock, which is
  /// where the marquee and the snap look for them, or on the clip's own, which
  /// is what a commit writes back.
  List<SelectedKey> _keyPlaces(List<AudioTrackRow> tracks, double fps,
      {bool drawn = true}) {
    final out = <SelectedKey>[];
    var y = 0.0;
    for (final track in tracks) {
      final step = track.rowHeight;
      // The track's own band comes first; the fold rows follow it.
      y += track.laneHeight;
      for (final row in track.drawnRows) {
        final rowId = foldRowPath(track.id, row);
        final keys = laneKeysOf(row);
        final shift = drawn ? _clipShiftOf(track, row) : 0;
        for (var i = 0; i < keys.length; i++) {
          out.add(SelectedKey(
            entry: track.entry,
            row: row,
            rowId: rowId,
            index: i,
            frame: laneKeyFrame(keys[i], fps) + shift,
            top: y,
            height: step,
          ));
        }
        y += step;
      }
    }
    return out;
  }

  void _selectKey(String id, bool additive) {
    final next = <String>{..._laneKeys.value};
    if (additive) {
      if (!next.remove(id)) next.add(id);
    } else {
      next
        ..clear()
        ..add(id);
    }
    _laneKeys.value = next;
  }

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context);
    final comp = ui.selectedComp;
    if (comp == null) {
      return PlaceholderPanel(
        icon: LumitIcon.audio,
        title: l10n.panelAudioTimeline,
        hint: l10n.selectACompositionFirst,
      );
    }
    // Everything this panel draws comes from the read model: zero bridge calls
    // per rebuild.
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
    final (fpsNum, fpsDen) = ui.model.fpsExact;

    _bindTools(ui);
    _refreshProbes(ui.model.layers);
    _refreshReads(ui.model);
    // Which layers are tracks, and which of them are read-only picture rows.
    // The same rule the layer Timeline reads from the other side.
    final view = timelineViewLayers(
      layers: ui.model.layers,
      audioTimeline: true,
      mixOpen: true,
      hasAudio: _hasAudio,
      hasPicture: _hasPicture,
    );
    // The search box narrows the list to the tracks whose names carry what
    // was typed, as the layer Timeline narrows its rows. Everything below
    // reads this list, so nothing else has to know about the box.
    final needle = _search.trim().toLowerCase();
    final tracks = audioTimelineTracks(
      layers: [
        for (final entry in view.shown)
          if (needle.isEmpty || entry.info.name.toLowerCase().contains(needle))
            entry,
      ],
      dimmed: view.dimmed,
      open: _open,
      rowHeight: t.density.laneRow,
      trackRows: _trackRows,
      volumeDb: _volumeDb,
    );
    _lastTracks = tracks;
    _refreshPeaks(tracks);
    final heights = [for (final track in tracks) track.height];

    final revision = ui.model.revision;
    if (_workArea == null || revision != _workRevision || comp != _workComp) {
      _workRevision = revision;
      _workComp = comp;
      _workArea = workAreaFrames(comp);
    }
    final work = _workPreview.value ?? _workArea!;
    // Gathered once for the whole table, not once per lane.
    final snap = snapTargetsOf(
      layers: [for (final track in tracks) track.entry],
      compMarkers: markersOf(comp),
      keyRows: [
        for (final track in tracks)
          for (final row in track.allRows)
            (
              rowId: foldRowPath(track.id, row),
              // Where the diamonds are drawn, which is where a drag can land on
              // them: a clip's keys are its own clock's, a clip's start along.
              frames: laneKeysOf(row)
                  .map((k) => laneKeyFrame(k, fps) + _clipShiftOf(track, row)),
            ),
      ],
      playheadFrame: ui.playheadFrame.value,
      work: work,
      fps: fps,
    );

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        CompTabsFrb(
          state: Provider.of<LumitState>(context, listen: false),
          uiState: ui,
          onExport: () => exportFrb(context),
          title: l10n.panelAudioTimeline,
        ),
        Expanded(
          // Footage dropped on a track becomes a clip on it at the pointer,
          // overlapping what it lands on; dropped on empty ground it becomes an
          // Audio layer, which is a track of one clip. A composition is not a
          // sound and is ignored: nesting one belongs to the layer Timeline.
          child: DragTarget<Object>(
            onWillAcceptWithDetails: (details) =>
                details.data is FootageDragData,
            onAcceptWithDetails: (details) async {
              if (details.data case FootageDragData(:final footage)) {
                await _dropFootage(comp, footage, details.offset);
              }
            },
            builder: (context, candidate, _) => Container(
              // A live outline while something is over it, so the drop is
              // visibly going to land rather than being taken on faith.
              foregroundDecoration: candidate.isEmpty
                  ? null
                  : BoxDecoration(
                      border: Border.all(color: t.accent, width: 2)),
              child: LayoutBuilder(
                builder: (context, box) {
                  final outlineWidth = min(
                    audioTimelineOutlineWidth,
                    max(120.0, box.maxWidth - scrollGutterWidth - _laneFloor),
                  );
                  final laneViewport =
                      (box.maxWidth - outlineWidth - scrollGutterWidth)
                          .clamp(1.0, 1e6);
                  _laneFrames = frames;
                  _zoomMotion.max = _maxZoom;
                  _pullZoomBackToCeiling();
                  // How wide the lanes are is how many buckets a picture wants.
                  // Measured here because this is where it is known; acted on
                  // after the frame, since a build must not start one.
                  if (_laneViewport != laneViewport) {
                    _laneViewport = laneViewport;
                    WidgetsBinding.instance.addPostFrameCallback((_) {
                      if (mounted) _refreshPeaks(_lastTracks);
                    });
                  }
                  // A two-finger trackpad scroll arrives as a pan gesture
                  // rather than as the wheel's pointer signal, so it is allowed
                  // here and nowhere else - a click-drag is still the marquee's.
                  return ScrollConfiguration(
                    behavior: ScrollConfiguration.of(context).copyWith(
                        dragDevices: const {PointerDeviceKind.trackpad},
                        scrollbars: false),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        _outlineHalf(t, ui, comp,
                            tracks: tracks,
                            heights: heights,
                            width: outlineWidth,
                            playhead: ui.playheadFrame.value),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              // The whole comp as a strip, over the lane area
                              // alone. Outside the zoom's builder below: it
                              // listens to the zoom, the scroll and the
                              // playhead for itself.
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
                                // **Only this half rebuilds when the zoom
                                // moves.** Nothing left of the seam depends on
                                // it, and rebuilding the outline once per
                                // animation frame is what makes a zoom crawl.
                                child: ListenableBuilder(
                                  listenable: _zoomMotion,
                                  builder: (context, _) => _laneHalf(
                                    t,
                                    ui,
                                    comp,
                                    axis: TimelineAxis(
                                        frames: frames,
                                        width: laneViewport * _zoom),
                                    tracks: tracks,
                                    heights: heights,
                                    work: work,
                                    snap: snap,
                                    frames: frames,
                                    fps: fps,
                                    fpsNum: fpsNum,
                                    fpsDen: fpsDen,
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

  // ------------------------------------------------------------- the outline

  /// The board's chrome strip over the column header: the playhead's time and
  /// frame, the comp's length, and the search that narrows the list. The
  /// layer Timeline's own pieces at its own height, less LAYERS and GRAPH -
  /// this panel has one shape.
  Widget _chromeStrip(LumitTheme t, LumitUiState ui) {
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    final lastFrame = ui.model.durationFrames - 1;
    final clockFace = t.mono.copyWith(fontSize: 11, color: t.textPrimary);
    final countFace = t.mono.copyWith(fontSize: 10, color: t.textMuted);
    final length = '/${ui.model.durationFrames}';
    // What each piece takes: a readout's slot is its digits plus its own
    // inset, and a well's hairline adds two either side of that.
    final clockWidth = monoSlotWidth(clockFace, timecodeChars(fpsNum, fpsDen)) +
        readoutPadding.horizontal +
        2;
    final countWidth = monoSlotWidth(
            countFace, 2 + '${lastFrame < 0 ? 0 : lastFrame}'.length) +
        readoutPadding.horizontal +
        2;
    return Container(
      height: TimelineNavigator.band + t.density.timelineChromeRow,
      color: t.surface1,
      padding: const EdgeInsets.only(left: 10, right: 8),
      child: LayoutBuilder(
        builder: (context, box) {
          // The ladder a narrow outline goes down: the well keeps its floor,
          // and the frame count and the comp's length go together as soon as
          // what is left will not hold them. The clock is what stays, because
          // a strip that cannot say the time is not worth its row.
          final whole = audioTimelineChromeCarriesCount(
            strip: box.maxWidth,
            clock: clockWidth,
            count: countWidth,
            length: monoSlotWidth(countFace, length.length),
          );
          return Row(
            children: [
              // Both readouts stand in slots wide enough for the longest thing
              // they can say and both can be typed into, as the Timeline's are.
              // Only they listen to the playhead: the rest of this half stands
              // still while it runs.
              ValueListenableBuilder<int>(
                valueListenable: ui.playheadFrame,
                builder: (context, frame, _) => Row(
                  children: [
                    timelineChromeControl(
                        t,
                        TimeReadout(
                          key: const ValueKey('atl-timecode'),
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
                    if (whole) const SizedBox(width: 4),
                    if (whole)
                      timelineChromeControl(
                          t,
                          TimeReadout(
                            key: const ValueKey('atl-frame'),
                            frame: frame,
                            format: (f) => 'F$f',
                            widthChars:
                                2 + '${lastFrame < 0 ? 0 : lastFrame}'.length,
                            style: countFace,
                            parse: _frameOfTyped,
                            onCommit: ui.scrubTo,
                            minFrame: 0,
                            maxFrame: lastFrame,
                            tooltip: l10n.tipFrameNumber,
                            well: true,
                            editFormat: (f) => '$f',
                          )),
                  ],
                ),
              ),
              // How many frames there are in all, after the frame the playhead is
              // on. Outside the listener, because a comp's length does not move
              // as the playhead does.
              if (whole) const SizedBox(width: 4),
              if (whole) Text(length, style: countFace),
              const SizedBox(width: outlineGap),
              // The well takes what is left of the row and is the first to give
              // it back, so a narrow outline still reads its clock.
              Expanded(
                child: timelineChromeControl(
                  t,
                  SizedBox(
                    height: layerSearchWellHeight,
                    child: HouseTextField(
                      key: const ValueKey('atl-search'),
                      controller: _searchField,
                      width: 1e9,
                      padding: const EdgeInsets.symmetric(horizontal: 6),
                      hint: l10n.searchTracks,
                      leading: glyph.LumitIcon(LumitIcons.search,
                          size: layerSearchWellHeight - 4, colour: t.textMuted),
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

  /// A typed frame number, with or without the letter the readout wears.
  static int? _frameOfTyped(String text) =>
      int.tryParse(text.trim().replaceFirst(RegExp('^[fF]'), ''));

  Widget _outlineHalf(
    LumitTheme t,
    LumitUiState ui,
    CompositionReference comp, {
    required List<AudioTrackRow> tracks,
    required List<double> heights,
    required double width,
    required int playhead,
  }) =>
      SizedBox(
        width: width,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // Level with the lane side's navigator strip and ruler, so the two
            // halves spend the same height above their first track: the chrome
            // strip stands as tall as the navigator's band and the chrome row
            // together, then the board's column header faces the ruler's lower
            // row.
            _chromeStrip(t, ui),
            const AudioTrackHeaderRow(),
            Expanded(
              child: Stack(
                children: [
                  LayoutBuilder(
                    builder: (context, box) => SingleChildScrollView(
                      controller: _vOutline,
                      child: ConstrainedBox(
                        constraints: BoxConstraints(minHeight: box.maxHeight),
                        // Only the blocks in view are built; the rest are two
                        // blanks holding the stack open, so the outline costs
                        // what is on screen rather than what the comp has.
                        child: LazyBlocks(
                          key: const ValueKey<String>('atl-outline-blocks'),
                          controller: _vOutline,
                          heights: heights,
                          viewport: box.maxHeight,
                          builder: (context, i) => _outlineBlock(
                              t, ui, comp, tracks[i], i,
                              playhead: playhead),
                        ),
                      ),
                    ),
                  ),
                  // The row seams the lane half rules, over the same rows and
                  // the same empty ground: the two halves are one table, and a
                  // line that stopped at the seam would say they were two.
                  // Pinned to the rows' own viewport and carried by the
                  // scroll, which is how the layer Timeline draws its own.
                  Positioned.fill(
                    key: const ValueKey<String>('atl-outline-seams'),
                    child: IgnorePointer(
                      child: AnimatedBuilder(
                        animation: _vOutline,
                        builder: (context, _) => CustomPaint(
                          painter: RowDividerPainter(
                            step: t.density.laneRow,
                            colour: t.hairline,
                            phase: -((positionOf(_vOutline)?.pixels ?? 0) %
                                t.density.laneRow),
                            // A track is one row of the table, several lane
                            // rows tall, so the seams inside its wave are left
                            // out here as they are on the lanes; the grid
                            // repeats from this overlay's own top edge, so
                            // each blank is carried up by however far the rows
                            // have scrolled.
                            blanks: [
                              for (final b in _trackBlanks(tracks))
                                (
                                  b.$1 - (positionOf(_vOutline)?.pixels ?? 0),
                                  b.$2 - (positionOf(_vOutline)?.pixels ?? 0),
                                ),
                            ],
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
            // The room the lane half's bottom bar takes, so both halves give
            // their rows the same viewport and scroll the same distance.
            SizedBox(height: t.density.secondaryRow),
          ],
        ),
      );

  Widget _outlineBlock(
    LumitTheme t,
    LumitUiState ui,
    CompositionReference comp,
    AudioTrackRow track,
    int index, {
    required int playhead,
  }) =>
      Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // The pick rides on the shell's own list, so choosing a track
          // repaints this row and its neighbour and rebuilds no panel.
          ValueListenableBuilder<List<LayerReference>>(
            valueListenable: ui.selectedLayers,
            builder: (context, picked, _) => AudioTrackOutlineRow(
              key: ValueKey<String>('atl-row-${track.id}'),
              comp: comp,
              track: track,
              index: index,
              laneMode: _modeOf(track.id),
              selected: picked.any((l) =>
                  l.internallayerId == track.entry.layer.internallayerId),
              rename: _renaming == track.id ? _rename : null,
              onToggleOpen: () => _toggleOpen(track.id),
              onCycleLaneMode: () => _cycleLaneMode(track.id),
              onDetach: () => _detachAudio(track.entry),
              onSelect: () => ui.setSelection([track.entry.layer]),
              onRenameCommit: () => _commitRename(track),
              onRenameCancel: _cancelRename,
              onChanged: _afterWrite,
              onResize: (dy) => _resizeTrack(track, dy),
              onResizeStart: () => _rowCarry = 0,
            ),
          ),
          for (final row in track.drawnRows)
            FoldRow(
              key: ValueKey<String>('atl-prop-${foldRowPath(track.id, row)}'),
              comp: comp,
              layer: track.entry.layer,
              row: row,
              // No column groups here, so the value wells sit against the
              // outline's own right edge and a fold row indents from the twirl.
              valueColumn: const ValueColumn(90, outlineRowTrailing),
              timingsColumn: const ValueColumn(0, 0),
              baseIndent: outlineGap + switchCellWidth * 3 + outlineGap,
              path: foldRowPath(track.id, row),
              selectedProperties: _selectedProperties,
              graphColours: const {},
              onSelectProperty: _pickProperty,
              onEditProperty: _pickProperty,
              playheadFrame: playhead,
              onSeek: ui.scrubTo,
              onToggle: _toggleOpen,
              onChanged: _afterWrite,
              locked: track.entry.info.switches.locked,
              // A clip's effect heading wears the bypass tick, because a clip
              // is nobody's subject in the Effect controls panel and this is
              // the only place its stack is listed.
              onSetEnabled: (path, on) => _bypassFromPath(track, path, on),
              // And the track's own Effects heading wears the add glyph: the
              // rack is the track's, not any clip's, and a clip fills its own
              // from the button on its header.
              onAddEffect: foldRowPath(track.id, row) == effectsPath(track.id)
                  ? (at) => _addTrackEffect(track, at)
                  : null,
            ),
        ],
      );

  void _pickProperty(String path) =>
      setState(() => _selectedProperties = [path]);

  /// *Detach audio* on a faded picture row: its sound goes onto an Audio layer
  /// of its own and the picture row is muted, which takes it off this list and
  /// puts the new track in its place.
  Future<void> _detachAudio(BridgeLayerEntry entry) async {
    try {
      await entry.layer.detachAudio();
    } catch (_) {
      if (!mounted) return;
      Provider.of<LumitState>(context, listen: false)
          .postNotice(l10n.detachAudioNoSound);
      return;
    }
    if (!mounted) return;
    _afterWrite();
  }

  // ------------------------------------------------------- clips and gestures

  /// The axis the lanes were last drawn with - what a drop or a released drag
  /// is measured against. Rebuilt rather than held, because both numbers it is
  /// made of are already kept for the zoom.
  TimelineAxis get _laneAxis =>
      TimelineAxis(frames: _laneFrames, width: _laneViewport * _zoom);

  /// Where [global] falls on the lanes' own scrolled content: x measured along
  /// the axis, y measured down from the top of the first track.
  Offset? _pointOnLanes(Offset global) {
    final box = _laneContent.currentContext?.findRenderObject();
    return box is RenderBox ? box.globalToLocal(global) : null;
  }

  /// Footage let go on the table: a clip on the track it landed on, or a new
  /// Audio layer when it landed on none.
  Future<void> _dropFootage(CompositionReference comp,
      List<FootageReference> footage, Offset global) async {
    final at = _pointOnLanes(global);
    final track = at == null ? null : audioTrackAt(_lastTracks, at.dy);
    final entry = track == null ? null : _lastTracks[track].entry;
    if (entry != null && audioTrackTakesClips(entry.info)) {
      final frame = _laneAxis.frameAt(at!.dx);
      asOneUndoStep(
        Provider.of<LumitState>(context, listen: false).project,
        () {
          // A track of one clip that has never been cut is still a track, so
          // the drop converts it the way the first clip gesture on it would.
          // A row that is a Sequence layer already refuses, and simply takes
          // the clip as it is.
          if (entry.info.clips.isEmpty) _makeSequenced(entry.layer);
          for (final f in footage) {
            // Overlapping, because an overlap on an audio track is a
            // crossfade - a drop keeps what it lands on rather than cutting a
            // hole in it.
            entry.layer.addClip(footage: f, atFrame: frame, overlap: true);
          }
        },
      );
    } else {
      for (final f in footage) {
        await comp.addAudioLayer(footage: f);
      }
    }
    if (mounted) _afterWrite();
  }

  /// Turn an unconverted Audio layer into the one clip it is, and say whether
  /// the conversion went through.
  ///
  /// The engine refuses a retimed layer, and refuses a row that is a Sequence
  /// layer already - which a track whose last clip was deleted still is, bare
  /// grab bar and all. A refusal is the gesture writing nothing rather than an
  /// error thrown out of a pointer handler.
  bool _makeSequenced(LayerReference layer) {
    try {
      layer.convertToSequenced();
      return true;
    } catch (_) {
      return false;
    }
  }

  /// A clip gesture, let go: a trim, a slide along the track, or a move on to
  /// another track - or, below the last one, on to a track of the clip's own.
  ///
  /// On an **unconverted** Audio layer the clip does not exist yet, so the
  /// conversion and the edit go in one undo group: one Ctrl+Z puts the layer
  /// back as it was.
  void _commitClipEdit(AudioTrackRow track, AudioClipEdit edit) {
    final layer = track.entry.layer;
    final at = _pointOnLanes(edit.at);
    final index = at == null ? null : audioTrackAt(_lastTracks, at.dy);
    final onward = index == null || _lastTracks[index].id == track.id
        ? null
        : _lastTracks[index].entry.layer;
    // Below the last track: a track of the clip's own, which is how clips are
    // spread out again without a button for an empty one.
    final toNewTrack = edit.grab == BarGrab.move &&
        index == null &&
        at != null &&
        audioBelowTracks(_lastTracks, at.dy);
    // A move that goes nowhere writes nothing - and on an unconverted layer
    // that matters, because the conversion would be the whole of the edit.
    if (edit.grab == BarGrab.move &&
        edit.shift == 0 &&
        onward == null &&
        !toNewTrack) {
      return;
    }
    asOneUndoStep(
      Provider.of<LumitState>(context, listen: false).project,
      () {
        var clip = edit.clip;
        if (clip == null) {
          if (!_makeSequenced(layer)) return;
          final made = layer.getClips();
          if (made.isEmpty) return;
          clip = made.first;
        }
        switch (edit.grab) {
          case BarGrab.move:
            final to = clip.startFrame + edit.shift;
            if (onward != null || toNewTrack) {
              layer.moveClip(
                  clip: clip.id, target: onward, toFrame: to, overlap: true);
            } else if (edit.shift != 0) {
              layer.slideClip(clip: clip.id, toFrame: to, overlap: true);
            }
          case BarGrab.trimIn:
            layer.trimClip(
              clip: clip.id,
              startFrame: clip.startFrame + edit.shift,
              endFrame: clip.endFrame,
            );
          case BarGrab.trimOut:
            layer.trimClip(
              clip: clip.id,
              startFrame: clip.startFrame,
              endFrame: clip.endFrame + edit.shift,
            );
        }
      },
    );
    _afterWrite();
  }

  // ------------------------------------------------- track and clip effects

  /// The Effects heading's glyph: the catalogue narrowed to what a track can
  /// be heard through, added to the **track's own** stack. The road the Effect
  /// controls panel's add takes, aimed at this layer.
  Future<void> _addTrackEffect(AudioTrackRow track, BuildContext at) async {
    await showAddEffectMenu(at, (name) {
      track.entry.layer.addEffect(name: name);
      _afterStackEdit();
    }, category: 'audio');
  }

  /// The clip's own fx switch: its whole stack bypassed, or given back.
  void _toggleClipFx(AudioTrackRow track, BridgeClip clip) {
    track.entry.layer.setClipFx(clip: clip.id, on_: !clip.fx);
    _afterStackEdit();
  }

  /// The add-effect button: the catalogue narrowed to the plugins a clip can
  /// be heard through, dropped from the button itself.
  Future<void> _addClipEffect(
      AudioTrackRow track, BridgeClip clip, BuildContext at) async {
    await showAddEffectMenu(at, (name) {
      track.entry.layer.addClipEffect(clip: clip.id, name: name);
      _afterStackEdit();
    }, category: 'audio');
  }

  /// One effect on a clip switched off, or on, from the heading's own path -
  /// `c:<clip>/effects/<effect>`, which carries both ids and so needs no
  /// lookup table beside it. The instance is read fresh at click time, because
  /// the read model carries drawing data and not handles.
  void _bypassFromPath(AudioTrackRow track, String path, bool on) {
    final effectId = effectIdOfPath(path);
    if (effectId == null) return;
    for (final clip in track.entry.info.clips) {
      if (path != effectPath(clipFoldPrefix(clip.id), effectId)) continue;
      for (final instance in track.entry.layer.getClipEffects(clip: clip.id)) {
        if (instance.id().toString() != effectId) continue;
        track.entry.layer.setEffectEnabled(effect: instance, enabled: on);
        _afterStackEdit();
        return;
      }
    }
  }

  /// What every stack edit ends with, a track's rack and a clip's alike: the
  /// read model freshened, and the mixer told to build its jobs again - a
  /// chain that has changed is a different sound, and nothing else asks for
  /// it.
  void _afterStackEdit() {
    _afterWrite();
    _ui?.selectedComp?.audioPrepare();
  }

  /// A clip's twirl: the drop-down rows under the track, keyed by the clip's
  /// own prefix so several clips can stand open at once.
  void _toggleClipOpen(BridgeClip clip) => _toggleOpen(clipFoldPrefix(clip.id));

  /// One shape written on one clip, keeping the length it already has. A clip
  /// with no fade yet gets a second, which is a fade you can see and then drag
  /// by its corner.
  void _setFadeShape(
    AudioTrackRow track,
    BridgeClip clip,
    BridgeClipFadeShape shape, {
    required bool into,
  }) {
    final held = into ? clip.fadeIn : clip.fadeOut;
    final fade = BridgeClipFade(
        seconds: held.seconds > 0 ? held.seconds : 1, shape: shape);
    track.entry.layer.setClipFade(
      clip: clip.id,
      fadeIn: into ? fade : null,
      fadeOut: into ? null : fade,
    );
  }

  /// The shapes of a join, written to the clips they belong to: the rising one
  /// to the clip whose start it is, the falling one to the clip whose end it
  /// is. A lone fade has one of the two, and the side with no clip is skipped.
  void _writeFadeShapes(
    AudioTrackRow track,
    BridgeClip clip, {
    required bool into,
    BridgeClipFadeShape? outgoing,
    BridgeClipFadeShape? incoming,
  }) {
    final clips = track.entry.info.clips;
    final partner = clipFadePartner(clips, clip, into: into);
    final rising = into ? clip : partner;
    final falling = into ? partner : clip;
    asOneUndoStep(
      Provider.of<LumitState>(context, listen: false).project,
      () {
        if (incoming != null && rising != null) {
          _setFadeShape(track, rising, incoming, into: true);
        }
        if (outgoing != null && falling != null) {
          _setFadeShape(track, falling, outgoing, into: false);
        }
      },
    );
    _afterWrite();
  }

  /// A corner let go: the fade at that end is as long as the drag left it, and
  /// zero takes it away. On an **unconverted** Audio layer the clip does not
  /// exist yet, so the conversion and the fade go in one undo step.
  void _writeFade(AudioTrackRow track, AudioClipFade fade) {
    final layer = track.entry.layer;
    asOneUndoStep(
      Provider.of<LumitState>(context, listen: false).project,
      () {
        var clip = fade.clip;
        if (clip == null) {
          if (!_makeSequenced(layer)) return;
          final made = layer.getClips();
          if (made.isEmpty) return;
          clip = made.first;
        }
        // The shape is the clip's own, which is Fast until somebody says
        // otherwise: a corner drag says how long, not what curve.
        final written = BridgeClipFade(
          seconds: fade.seconds,
          shape: (fade.into ? clip.fadeIn : clip.fadeOut).shape,
        );
        layer.setClipFade(
          clip: clip.id,
          fadeIn: fade.into ? written : null,
          fadeOut: fade.into ? null : written,
        );
      },
    );
    _afterWrite();
  }

  /// The fade menu: a right click on a fade, or anywhere in an overlap. The
  /// five shapes by name, then the editor.
  ///
  /// A preset lands on both sides of an overlap, because a crossfade is a pair
  /// and Fast against Fast is what keeps the join as loud as either clip was.
  Future<void> _fadeMenu(
      AudioTrackRow track, BridgeClip clip, bool into, Offset at) async {
    await showMenuAt<void>(
      context: context,
      position: at,
      width: 160,
      rows: (close) => [
        for (final preset in clipFadeShapes)
          MenuRow(
            key: ValueKey<String>('atl-fade-shape-${preset.id}-${clip.id}'),
            onPressed: () {
              close(null);
              _writeFadeShapes(track, clip,
                  into: into, outgoing: preset.shape, incoming: preset.shape);
            },
            child: Text(clipFadeShapeName(preset.id)),
          ),
        MenuRow(
          key: ValueKey<String>('atl-fade-custom-${clip.id}'),
          onPressed: () {
            close(null);
            final partner =
                clipFadePartner(track.entry.info.clips, clip, into: into);
            showClipFadePopover(
              context: context,
              position: at,
              outgoing: (into ? partner : clip)?.fadeOut.shape,
              incoming: (into ? clip : partner)?.fadeIn.shape,
              onApply: (outgoing, incoming) => _writeFadeShapes(track, clip,
                  into: into, outgoing: outgoing, incoming: incoming),
            );
          },
          child: Text(l10n.clipFadeCustom),
        ),
      ],
    );
  }

  /// What can be done to one clip: its two fades by shape, the split at the
  /// playhead, and the delete.
  Future<void> _clipMenu(
      AudioTrackRow track, BridgeClip clip, Offset at) async {
    final ui = _ui;
    Widget shapes(VoidCallback dismiss, {required bool into}) => FloatSurface(
          width: 140,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final preset in clipFadeShapes)
                MenuRow(
                  key: ValueKey<String>(
                      'atl-fade-${into ? 'in' : 'out'}-${preset.id}-${clip.id}'),
                  onPressed: () {
                    dismiss();
                    _setFadeShape(track, clip, preset.shape, into: into);
                    _afterWrite();
                  },
                  child: Text(clipFadeShapeName(preset.id)),
                ),
            ],
          ),
        );

    await showMenuAt<void>(
      context: context,
      position: at,
      width: 190,
      rows: (close) => [
        SubmenuRow(
          key: ValueKey<String>('atl-clip-fade-in-${clip.id}'),
          closeParent: () => close(null),
          submenu: (dismiss) => shapes(dismiss, into: true),
          child: Text(l10n.clipFadeIn),
        ),
        SubmenuRow(
          key: ValueKey<String>('atl-clip-fade-out-${clip.id}'),
          closeParent: () => close(null),
          submenu: (dismiss) => shapes(dismiss, into: false),
          child: Text(l10n.clipFadeOut),
        ),
        MenuRow(
          key: ValueKey<String>('atl-clip-split-${clip.id}'),
          onPressed: () {
            close(null);
            track.entry.layer.cutClipAt(frame: ui?.playheadFrame.value ?? 0);
            _afterWrite();
          },
          child: Text(l10n.clipSplitAtPlayhead),
        ),
        MenuRow(
          key: ValueKey<String>('atl-clip-delete-${clip.id}'),
          onPressed: () {
            close(null);
            // A gap, not a closed row: what follows keeps the beat it was cut
            // to.
            track.entry.layer.deleteClip(clip: clip.id);
            setState(() => _selectedClip = null);
            _afterWrite();
          },
          child: Text(l10n.clipDelete),
        ),
      ],
    );
  }

  /// The razor, aimed at the row that was clicked - or, with Shift, at every
  /// track that spans the cut, which is what it does in the layer Timeline.
  void _razorCut(BridgeLayerEntry clicked, int frame) {
    final targets = razorTargets(
      [for (final track in _lastTracks) track.entry],
      frame,
      clicked: clicked,
      allLayers: HardwareKeyboard.instance.isShiftPressed,
    );
    final made = razorCut(targets, frame);
    if (!made.cut) return;
    // The half after the cut is the one you go on working with, so the cut
    // hands the selection to it, as the layer Timeline's razor does.
    if (made.halves.isNotEmpty) _ui?.setSelection(made.halves);
    _afterWrite();
  }

  // ---------------------------------------------------------------- the lanes

  Widget _laneHalf(
    LumitTheme t,
    LumitUiState ui,
    CompositionReference comp, {
    required TimelineAxis axis,
    required List<AudioTrackRow> tracks,
    required List<double> heights,
    required ({int start, int end, bool whole}) work,
    required List<SnapTarget> snap,
    required int frames,
    required double fps,
    required int fpsNum,
    required int fpsDen,
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
                          tracks: tracks,
                          heights: heights,
                          work: work,
                          snap: snap,
                          frames: frames,
                          fps: fps,
                          fpsNum: fpsNum,
                          fpsDen: fpsDen),
                    ),
                  ),
                ),
                // The lanes' thumb, pinned to the viewport's right edge rather
                // than riding the scrolled content.
                SizedBox(
                  width: scrollGutterWidth,
                  child: Column(
                    children: [
                      // Level with the ruler beside it. The navigator's band is
                      // above this half already, and the strip leaves the
                      // gutter's width free for it.
                      Container(
                          height: t.density.ruler, color: t.timelineOutOfRange),
                      Expanded(child: GutterScrollbar(controller: _vLane)),
                    ],
                  ),
                ),
              ],
            ),
          ),
          // The board's own bottom bar: the zoom slider and the scrollbar, and
          // nothing else. No magnet - a clip gesture suspends the snap with
          // Ctrl rather than with a button.
          LaneBottomBar(
            zoom: _zoomMotion.target,
            maxZoom: _maxZoom,
            hScroll: _hLane,
            onZoom: _setZoom,
            onZoomLive: (z) => _setZoom(z, fly: false),
            onZoomDragStart: _zoomDragStart,
            onZoomDragEnd: _zoomDragEnd,
          ),
        ],
      );

  Widget _laneArea(
    LumitTheme t,
    LumitUiState ui,
    CompositionReference comp, {
    required TimelineAxis axis,
    required List<AudioTrackRow> tracks,
    required List<double> heights,
    required ({int start, int end, bool whole}) work,
    required List<SnapTarget> snap,
    required int frames,
    required double fps,
    required int fpsNum,
    required int fpsDen,
  }) {
    // Where a razor cut lands, as a frame - the one answer the blade's line and
    // the cut itself both read, so the mark cannot stand anywhere but where the
    // edge bites.
    double razorFrameAt(double x) => snapFrame(
          frame: axis.frameAtExact(x),
          targets: snap,
          perFrame: axis.perFrame,
          magnet: !snapSuspended(
              controlPressed: HardwareKeyboard.instance.isControlPressed),
        ).frame.roundToDouble();
    final razor = _razorArmed(ui);
    return RazorOverlay(
      active: razor,
      snapX: (x) => axis.xOf(razorFrameAt(x)),
      mark: t.textPrimary,
      outline: t.surface0,
      child: Stack(
        children: [
          Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              // The navigator's band is spent above this builder, and the
              // outline reserves the band and this ruler in one strip - so both
              // halves spend the same height above their first track.
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
                snapTargets: snap,
                cache: TimelineCacheBar(
                    comp: comp, axis: axis, revision: _cacheRevision!),
              ),
              Expanded(
                child: LayoutBuilder(
                  builder: (context, box) => SingleChildScrollView(
                    controller: _vLane,
                    child: ConstrainedBox(
                      constraints: BoxConstraints(minHeight: box.maxHeight),
                      // Innermost, so the pointer-signal resolver hands it the
                      // wheel before the scrollables do.
                      child: Listener(
                        onPointerMove: (event) {
                          if (event.buttons == kMiddleMouseButton) {
                            _panLanes(event.delta);
                          }
                        },
                        onPointerSignal: (event) {
                          if (event is! PointerScrollEvent) return;
                          final keys = HardwareKeyboard.instance;
                          if (!keys.isControlPressed && !keys.isShiftPressed) {
                            return;
                          }
                          GestureBinding.instance.pointerSignalResolver
                              .register(event, (resolved) {
                            if (resolved is PointerScrollEvent) {
                              _wheel(resolved, resolved.localPosition.dx, axis);
                            }
                          });
                        },
                        child: Stack(
                          // Keyed, because this is the box a released drag and
                          // a dropped file are measured in: x along the axis, y
                          // down from the top of the first track.
                          key: _laneContent,
                          children: [
                            WorkAreaGround(
                              key: const ValueKey<String>('atl-lane-ground'),
                              preview: _workPreview,
                              committed: work,
                              axis: axis,
                              inside: Color.alphaBlend(
                                  t.animated
                                      .withValues(alpha: workAreaLaneFillAlpha),
                                  t.surface1),
                              outside: t.timelineOutOfRange,
                              edge: workAreaEdgeColour(t),
                            ),
                            // Behind the lanes: dragging empty ground boxes up
                            // keyframes; the bands and diamonds above still win
                            // their own gestures.
                            Positioned.fill(
                              child: MarqueeSelect(
                                key: const ValueKey('atl-lane-marquee'),
                                onSelect: (rect, additive) =>
                                    _laneKeys.value = {
                                  if (additive) ..._laneKeys.value,
                                  ..._keysIn(rect, tracks, axis, fps),
                                },
                                // A click on empty ground gives up the keys and
                                // the picked clip together: it means "nothing
                                // is selected", not "nothing of one kind is".
                                onClear: () {
                                  _laneKeys.value = {};
                                  if (_selectedClip != null) {
                                    setState(() => _selectedClip = null);
                                  }
                                },
                              ),
                            ),
                            LazyBlocks(
                              // Named so a budget test can read the lanes' own
                              // paint count: a playhead-only change must leave
                              // it alone.
                              key: const ValueKey<String>('atl-lane-blocks'),
                              controller: _vLane,
                              heights: heights,
                              viewport: box.maxHeight,
                              builder: (context, i) => _laneBlock(
                                  t, ui, tracks[i],
                                  axis: axis,
                                  snap: snap,
                                  fps: fps,
                                  fpsNum: fpsNum,
                                  fpsDen: fpsDen,
                                  razor: razor,
                                  razorFrameAt: razorFrameAt),
                            ),
                            // The row hairlines, over everything and touching
                            // nothing: they run the full width so the eye can
                            // track a track across the table, and they are
                            // drawn rather than given to each row as a border
                            // because a decorated box absorbs pointers.
                            Positioned.fill(
                              child: IgnorePointer(
                                child: AnimatedBuilder(
                                  animation: _vLane,
                                  builder: (context, _) => CustomPaint(
                                    painter: RowDividerPainter(
                                      step: t.density.laneRow,
                                      colour: t.hairline,
                                      // A track is one row of the table,
                                      // several lane rows tall, so the seams
                                      // that would fall inside its wave are
                                      // left out; the ones bounding it stay.
                                      blanks: _trackBlanks(tracks),
                                      // Only the fraction: rounding is
                                      // invariant under whole-pixel shifts, so
                                      // a whole-pixel scroll repaints nothing.
                                      origin:
                                          -((positionOf(_vLane)?.pixels ?? 0) %
                                              1.0),
                                    ),
                                  ),
                                ),
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
          // The playhead rides above every lane, and it is the only thing here
          // that redraws when it moves - on its own layer, so that is true of
          // the painting and not only of the rebuilding.
          PlayheadOverlay(playhead: ui.playheadFrame, xOf: axis.xOf),
        ],
      ),
    );
  }

  /// Each track's own band, so the seam painter rules between tracks and not
  /// through them. The twirl's rows are outside the blank and keep their
  /// hairlines, which is what makes an open track read as a list.
  List<(double, double)> _trackBlanks(List<AudioTrackRow> tracks) {
    final out = <(double, double)>[];
    var y = 0.0;
    for (final track in tracks) {
      out.add((y, y + track.laneHeight));
      y += track.height;
    }
    return out;
  }

  /// Every keyframe the box caught, walking the same rows the lanes draw.
  Set<String> _keysIn(
      Rect rect, List<AudioTrackRow> tracks, TimelineAxis axis, double fps) {
    final caught = <String>{};
    for (final place in _keyPlaces(tracks, fps)) {
      if (place.top + place.height < rect.top || place.top > rect.bottom) {
        continue;
      }
      final x = axis.xOf(place.frame);
      if (x >= rect.left && x <= rect.right) {
        caught.add('${place.rowId}#${place.index}');
      }
    }
    return caught;
  }

  /// The release of a lane key's drag: every key it carried written where it
  /// travelled, one undo step. Done here because the selection reaches across
  /// rows and a lane knows only its own.
  void _moveHeldKeys(KeyStretch moved, List<AudioTrackRow> tracks, double fps,
      int fpsNum, int fpsDen) {
    if (commitKeyGesture(
      places: [
        // On each row's own clock: the travel is the same number in both, and
        // a clip's key is written where the clip counts from.
        for (final place in _keyPlaces(tracks, fps, drawn: false))
          if (_laneKeys.value.contains('${place.rowId}#${place.index}')) place,
      ],
      moved: moved,
      whole: !snapSuspended(
          controlPressed: HardwareKeyboard.instance.isControlPressed),
      fpsNum: fpsNum,
      fpsDen: fpsDen,
      project: Provider.of<LumitState>(context, listen: false).project,
    )) {
      _afterWrite();
    }
  }

  Widget _laneBlock(
    LumitTheme t,
    LumitUiState ui,
    AudioTrackRow track, {
    required TimelineAxis axis,
    required List<SnapTarget> snap,
    required double fps,
    required int fpsNum,
    required int fpsDen,
    required bool razor,
    required double Function(double) razorFrameAt,
  }) =>
      Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SizedBox(
            height: track.laneHeight,
            child: dimmedIf(
                track.dimmed,
                _trackLane(t, track,
                    axis: axis,
                    snap: snap,
                    fps: fps,
                    razor: razor,
                    razorFrameAt: razorFrameAt)),
          ),
          for (final row in track.drawnRows)
            SizedBox(
              height: track.rowHeight,
              child: _foldLane(ui, track, row,
                  axis: axis,
                  snap: snap,
                  fps: fps,
                  fpsNum: fpsNum,
                  fpsDen: fpsDen),
            ),
        ],
      );

  /// A track's own lane: its wave or spectrogram across its bar, with the level
  /// as a number at the right.
  ///
  /// A **converted** track - one that holds clips - draws the clips instead,
  /// each with its own picture and its own gain line. The track's Volume is not
  /// on the lane at all: it keeps the row under the twirl, and the gain line on
  /// each clip took the band's place (docs/impl/audio-timeline.md §5).
  Widget _trackLane(
    LumitTheme t,
    AudioTrackRow track, {
    required TimelineAxis axis,
    required List<SnapTarget> snap,
    required double fps,
    required bool razor,
    required double Function(double) razorFrameAt,
  }) {
    final info = track.entry.info;
    final left = axis.xOf(info.inFrame.toInt());
    final right = axis.xOf(info.outFrame.toInt());
    final height = track.laneHeight;
    final clips = AudioClipStrip(
      key: ValueKey<String>('atl-clips-${track.id}'),
      track: track,
      axis: axis,
      fps: fps,
      height: height,
      mode: _modeOf(track.id),
      style: _waveformStyle,
      hScroll: _hLane,
      snapTargets: snap,
      preview: _clipDragOf(track.id),
      selected: _selectedClip?.track == track.id ? _selectedClip?.clip : null,
      openClips: {
        for (final clip in info.clips)
          if (_open.contains(clipFoldPrefix(clip.id))) clip.id.toString(),
      },
      onToggleFx: (clip, _) => _toggleClipFx(track, clip),
      onAddEffect: (clip, at) => _addClipEffect(track, clip, at),
      onToggleOpen: (clip, _) => _toggleClipOpen(clip),
      razor: razor,
      onRazor: (frame) => _razorCut(track.entry, frame),
      razorFrameAt: razorFrameAt,
      onSelect: (clip) => setState(
          () => _selectedClip = (track: track.id, clip: clip.id.toString())),
      onMenu: (clip, at) => _clipMenu(track, clip, at),
      onFadeMenu: (clip, into, at) => _fadeMenu(track, clip, into, at),
      onCommit: (edit) => _commitClipEdit(track, edit),
      onChanged: _afterWrite,
    );
    // The ramps and their corner handles, over the clips: each tops out at its
    // clip's own gain line.
    final fades = AudioFadeLayer(
      key: ValueKey<String>('atl-fade-layer-${track.id}'),
      track: track,
      axis: axis,
      fps: fps,
      height: height,
      snapTargets: snap,
      preview: _clipDragOf(track.id),
      razor: razor,
      onFade: (fade) => _writeFade(track, fade),
      onCommit: (edit) => _commitClipEdit(track, edit),
    );
    if (info.clips.isNotEmpty) return Stack(children: [clips, fades]);
    final startOffset = rationalSeconds(info.span.startOffset);
    final secondsPerPixel =
        axis.perFrame <= 0 || fps <= 0 ? 0.0 : 1 / (axis.perFrame * fps);
    // Canvas x 0 is the axis's left padding, comp time 0 sits a padding's width
    // in, and the source's own clock runs from there less wherever the layer
    // starts it.
    final origin = -startOffset - TimelineAxis.pad * secondsPerPixel;
    return Stack(
      children: [
        if (_modeOf(track.id) == LaneMode.spectral)
          RepaintBoundary(
            key: ValueKey<String>('atl-spectral-${track.id}'),
            child: SizedBox(
              width: axis.width,
              height: height,
              child: SpectralLane(
                grid: _spectra[track.id],
                originSeconds: origin,
                secondsPerPixel: secondsPerPixel,
                left: left,
                right: right,
                height: height,
              ),
            ),
          )
        else
          CustomPaint(
            key: ValueKey<String>('atl-wave-${track.id}'),
            size: Size(axis.width, height),
            painter: WaveformPainter(
              peaks: _peaks[track.id],
              originSeconds: origin,
              secondsPerPixel: secondsPerPixel,
              left: left,
              right: right,
              colours: t.waveform,
              style: _waveformStyle,
              height: height,
            ),
          ),
        // No clips yet, so no boxes - but the bar takes the same trims and
        // slides, and the first of them converts the layer into the one clip it
        // has always been.
        clips,
        // The level as a number at the right of the lane, the way the board
        // draws it. A keyed level is drawn on its own row under the twirl, so
        // there is no one number to say.
        if (_levelOf(info.volumeDb) case final reading?)
          Positioned(
            right: 6,
            top: 2,
            child: Text(reading,
                style: t.mono.copyWith(fontSize: 8, color: t.textMuted)),
          ),
        // The corners of the layer's own bar: the first fade dragged on to one
        // converts it, as the first trim does.
        fades,
      ],
    );
  }

  /// The track's level as a readout, or null while it is keyed.
  String? _levelOf(BridgeScalar scalar) => switch (scalar) {
        BridgeScalar_Static(:final field0) => field0 <= volumeBandFloorDb
            ? l10n.volumeNegInf
            : '${field0.toStringAsFixed(1)} dB',
        _ => null,
      };

  /// One twirl row's lane: its diamonds, or empty room for a heading.
  Widget _foldLane(
    LumitUiState ui,
    AudioTrackRow track,
    LayerFoldRow row, {
    required TimelineAxis axis,
    required List<SnapTarget> snap,
    required double fps,
    required int fpsNum,
    required int fpsDen,
  }) {
    final keys = laneKeysOf(row);
    if (keys.isEmpty) return const SizedBox.shrink();
    final rowId = foldRowPath(track.id, row);
    return ValueListenableBuilder<Set<String>>(
      valueListenable: _laneKeys,
      builder: (context, selected, _) => KeyLane(
        key: ValueKey<String>('atl-keys-$rowId'),
        entry: track.entry,
        row: row,
        rowId: rowId,
        keys: keys,
        axis: axis,
        fps: fps,
        fpsNum: fpsNum,
        fpsDen: fpsDen,
        magnet: true,
        // A clip's parameter is keyed from the clip's start, so its diamonds
        // are drawn that far along - the same shift a moved bar gives its keys.
        barShift: _clipShiftOf(track, row),
        snapTargets: snap,
        selectedKeys: selected,
        stretch: _keyStretch,
        // ponytail: no key menu here, so a right click on a diamond does
        // nothing. The ceiling is the interpolation rows and Delete key the
        // Timeline's menu offers; the upgrade is lifting that menu out of
        // timeline_panel_frb.dart, which is where its five commands live.
        onKeyMenu: (index, position) {},
        onSelectKey: (index, additive) => _selectKey('$rowId#$index', additive),
        onMoveKeys: (moved) =>
            _moveHeldKeys(moved, _lastTracks, fps, fpsNum, fpsDen),
        onChanged: ui.model.refresh,
      ),
    );
  }
}
