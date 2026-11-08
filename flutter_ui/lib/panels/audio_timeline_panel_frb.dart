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
// button, the lane picture's switch, the twirl on to Volume and the track's audio
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
import 'package:lumit_flutter/state/keymap.dart';
import 'package:lumit_flutter/src/rust/api/cut.dart'
    show BridgeCutMove, BridgeCutPaste, BridgeCutResult;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/streams.dart'
    show BridgeAudioStreams;
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show exportFrb, insertEmptyLayer;
import '../state/comp_model.dart';
import '../state/comp_time.dart';
import '../state/dock.dart';
import '../state/drag_payloads.dart';
import '../state/timecode.dart';
import '../state/timeline_columns.dart';
import '../state/tools.dart';
import '../theme/motion.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import '../widgets/marquee.dart';
import '../widgets/smooth_zoom.dart';
import '../widgets/time_readout.dart';
import '../widgets/zoom_anchored_scroll.dart';
import 'audio_routes_frb.dart';
import 'audio_streams_menu_frb.dart';
import 'audio_timeline_clips_frb.dart';
import 'audio_timeline_fades_frb.dart';
import 'audio_timeline_rows_frb.dart';
import 'clip_fade_popover.dart';
import 'clip_fades.dart';
import 'cut_timeline_panel_frb.dart' show openClipInSource;
import 'cut_timeline_tracks.dart'
    show cutClipsCopiedIn, cutClipsHeld, cutDuplicateFrames;
import 'easing_curve.dart' show ApplyEasing, EaseSides, EasingCurve;
import 'effect_controls_panel_frb.dart' show showAddEffectMenu;
import 'graph_channels.dart' show GraphChannel, graphChannels;
import 'graph_clipboard.dart'
    show copySelectedKeys, graphKeyClipboard, pasteKeysAtPlayhead;
import 'graph_edits.dart' show applyEasingToSelection, deleteKeysFromChannels;
import 'graph_maths.dart' show rationalSeconds;
import 'key_block.dart' show KeyStretch;
import 'layer_fold_frb.dart';
import '../state/share.dart';
import '../widgets/share_marks.dart';
import 'normalise_commands.dart';
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
import 'retime_commands.dart' show askToFreezeClips;
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
/// readouts beside it give up their room instead. A well under this cannot hold
/// its own hint on one line, and a long comp's readouts will squeeze it to
/// nothing given the chance.
const double audioTimelineSearchFloor = 120;

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

  /// The volume and pan lines across tracks, kept as the Cut timeline keeps
  /// its own.
  final EnvelopesShown _envelopes = EnvelopesShown();

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
  int? _readsRevision;

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
  MotionSpec _zoomFlight = MotionSpec.still;

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

  /// The lane marquee's box while it is being dragged.
  final ValueNotifier<Rect?> _laneMarquee = ValueNotifier<Rect?>(null);

  /// Which property rows are picked, so a row draws itself lit. A plain list:
  /// this panel has no graph to colour and no range selection yet.
  List<String> _selectedProperties = const [];

  /// The shared project's people, whose tracks and rows are marked here.
  /// Its roster only moves while a project is shared.
  ShareState? _share;

  /// Whether the project was shared the last time anyone was heard from.
  bool _sharing = false;

  /// Someone came, went or selected something else: the rows are drawn
  /// again. Nothing happens here while the project is not shared.
  void _onRoster() {
    if (!mounted) return;
    final now = _share?.active ?? false;
    if (now || _sharing) setState(() => _sharing = now);
    _shareKeys();
  }

  /// Tell the others in a shared project which keyframes are selected in
  /// these lanes. Nothing is gathered when the project is not shared.
  void _shareKeys() {
    if (!(_share?.active ?? false)) return;
    _share!.keys(Set<String>.of(_laneKeys.value), from: 'audio');
  }

  /// The work area, held between document revisions - reading it is several
  /// bridge calls and only an edit can change the answer - and the span staged
  /// while an edge is being dragged.
  ({int start, int end, bool whole})? _workArea;
  int? _workRevision;
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

  /// The picked clips, by id. The panel's own - nothing about a pick is in the
  /// document. Delete, Cut and Copy take them while this panel holds the keys.
  Set<String> _pickedClips = const {};

  /// Whether the last thing copied here was clips rather than keys, so Paste
  /// puts down what the person last had in hand.
  bool _copiedClips = false;

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
  bool Function()? _heldCut;
  bool Function()? _heldCopy;
  bool Function()? _heldPaste;
  ApplyEasing? _heldEasing;

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
    _share = Provider.of<LumitState>(context, listen: false).share
      ..roster.addListener(_onRoster);
    _sharing = _share!.active;
    _laneKeys.addListener(_shareKeys);
    _cacheRevision = Listenable.merge([_ui!.frameArrived, _ui!.cacheChanged]);
    _ui!.activePane.addListener(_onActivePanel);
    _onActivePanel();
    _searchField.addListener(() => setState(() => _search = _searchField.text));
    HardwareKeyboard.instance.addHandler(_onKey);
  }

  @override
  void dispose() {
    _share?.roster.removeListener(_onRoster);
    // With the panel gone nothing in it is selected, for the others either.
    _share?.keys(const {}, from: 'audio');
    _escapeRelease?.call();
    _boundTools?.removeListener(_onToolChanged);
    _ui?.activePane.removeListener(_onActivePanel);
    HardwareKeyboard.instance.removeHandler(_onKey);
    _rename?.dispose();
    _releaseKeys(closing: true);
    _zoomMotion.dispose();
    _vOutline.dispose();
    _vLane.dispose();
    _hLane.dispose();
    _searchField.dispose();
    _laneKeys.dispose();
    _keyStretch.dispose();
    _laneMarquee.dispose();
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
    if (!mounted || _pickedClips.isEmpty) return false;
    setState(() => _pickedClips = const {});
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
    // A clip here is the clip it is in the Cut timeline, so the chords its
    // menu shows mean the same.
    final comp = ui.selectedComp;
    final action = ui.keymap.actionFor(BridgeKeyContext.cut, event);
    switch (action) {
      case 'cut.envelope.volume' || 'cut.envelope.pan':
        _toggleEnvelope(
            action == 'cut.envelope.volume' ? BandKind.volume : BandKind.pan,
            _keyTracks(ui));
        return true;
      case 'cut.tracks.minimise' when _trackRows.isNotEmpty:
        setState(_trackRows.clear);
        return true;
      case 'cut.add.edit':
        _splitAtPlayhead(ui);
        return true;
      case 'cut.delete.ripple':
        if (_pickedClips.isEmpty) return false;
        _rippleDeletePicked(ui);
        return true;
      case 'cut.group' when _pickedClips.length > 1:
        _groupClips(join: true);
        return true;
      case 'cut.duplicate' when comp != null && _pickedClips.isNotEmpty:
        final picked = _pickedOnTracks();
        if (picked.isEmpty) return false;
        _duplicateClips(comp, picked);
        return true;
      case 'cut.ungroup' when _pickedClips.isNotEmpty:
        _groupClips(join: false);
        return true;
      case 'cut.paste.attributes'
          when _clipsHeld && _pickedClips.isNotEmpty && comp != null:
        final ids = [for (final (_, c) in _pickedOnTracks()) c.id];
        _afterCut(_cut(() => comp.cutPasteAttributes(clips: ids)));
        return true;
      case final String trim
          when trim.startsWith('cut.trim.') &&
              _pickedClips.isNotEmpty &&
              comp != null:
        _trimClips(comp, _pickedOnTracks(), ui.playheadFrame.value,
            start: trim.contains('.start'), ripple: trim.endsWith('.ripple'));
        return true;
    }
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
      _beginRename(track);
      return true;
    }
    return false;
  }

  /// The tracks a key means: the picked ones, else every one.
  List<AudioTrackRow> _keyTracks(LumitUiState ui) {
    final tracks = [for (final t in _lastTracks) if (!t.dimmed) t];
    final picked = [
      for (final t in tracks)
        if (ui.selectedLayerIds.contains(t.entry.layer.internallayerId)) t,
    ];
    return picked.isNotEmpty ? picked : tracks;
  }

  /// Show [kind]'s line on [tracks], or hide it when they all show it.
  void _toggleEnvelope(BandKind kind, List<AudioTrackRow> tracks) {
    if (tracks.isEmpty) return;
    setState(() =>
        _envelopes.toggle([for (final t in tracks) (t.entry, t.id)], kind));
  }

  /// Move [track]'s layer to where the track [by] places away stands.
  void _moveTrack(AudioTrackRow track, int by) {
    final at = _lastTracks.indexOf(track) + by;
    final ui = _ui;
    if (ui == null || at < 0 || at >= _lastTracks.length) return;
    final id = _lastTracks[at].entry.layer.internallayerId;
    final index =
        ui.model.heldLayers.indexWhere((e) => e.layer.internallayerId == id);
    if (index < 0) return;
    try {
      track.entry.layer.reorder(newIndex: index);
    } catch (_) {
      // A locked layer refuses, and quietly.
    }
    _afterStackEdit();
  }

  /// Turn a track's name into its field: Enter on a picked track, or the menu.
  void _beginRename(AudioTrackRow track) {
    setState(() {
      _renaming = track.id;
      _rename = TextEditingController(text: track.entry.info.name);
    });
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
    _heldCut = ui.cutClaim;
    _heldCopy = ui.copyClaim;
    _heldPaste = ui.pasteClaim;
    _heldEasing = ui.easingApply.value;
    ui.deleteClaim = _deleteClaim;
    ui.cutClaim = _cutClaim;
    ui.copyClaim = _copyClaim;
    ui.pasteClaim = _pasteClaim;
    ui.easingApply.value = _applyEasing;
  }

  void _releaseKeys({bool closing = false}) {
    final ui = _ui;
    if (ui == null || !_claimed) return;
    _claimed = false;
    // Only what is still ours: a third panel that has taken a slot since keeps
    // it, which is what makes this give-and-take rather than a scramble.
    if (ui.deleteClaim == _deleteClaim) ui.deleteClaim = _heldDelete;
    if (ui.cutClaim == _cutClaim) ui.cutClaim = _heldCut;
    if (ui.copyClaim == _copyClaim) ui.copyClaim = _heldCopy;
    if (ui.pasteClaim == _pasteClaim) ui.pasteClaim = _heldPaste;
    void ease() {
      if (ui.easingApply.value == _applyEasing) {
        ui.easingApply.value = _heldEasing;
      }
    }

    // Closing, after the frame: the Easing panel listens, and can't be asked
    // to rebuild while the tree is being taken down.
    closing
        ? WidgetsBinding.instance.addPostFrameCallback((_) => ease())
        : ease();
  }

  /// Whether the keys are this panel's to answer. The Timeline asks a claim it
  /// took over before its own, so a claim held from before must stand down.
  bool get _keysAreOurs => _ui?.activePanel == Panel.audioTimeline;

  /// Whether the last pick was a track's head, the one time Delete may take
  /// the whole track.
  bool _headPicked = false;

  /// Delete: the picked keyframes, or else the picked clips. Keys first, as
  /// they sit on the clip and picking one must not cost the clip. With
  /// neither the key is taken and nothing goes, unless the track was picked
  /// by its head: a clip let go of must not turn Delete on its track.
  bool _deleteClaim() {
    final ui = _ui;
    if (!mounted || ui == null || !_keysAreOurs) return false;
    if (_laneKeys.value.isNotEmpty && _deleteKeys()) return true;
    if (_pickedClips.isEmpty) return !_headPicked;
    final picked = _pickedOnTracks();
    if (picked.isEmpty) return false;
    _deleteClips(picked);
    return true;
  }

  /// The picked clips that are still on a track, each with its track.
  List<(AudioTrackRow, BridgeClip)> _pickedOnTracks() => [
        for (final track in _lastTracks)
          for (final clip in track.entry.info.clips)
            if (_pickedClips.contains(clip.id.toString())) (track, clip),
      ];

  /// Clips deleted as one undo step, and a row with them when they were the
  /// last ones there: an empty row is a silent bar with nothing on it to take
  /// hold of.
  void _deleteClips(List<(AudioTrackRow, BridgeClip)> clips) {
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
      for (final track in {for (final (track, _) in clips) track}) {
        final going = [
          for (final (on, clip) in clips)
            if (identical(on, track)) clip,
        ];
        if (going.length == track.entry.info.clips.length) {
          track.entry.layer.delete();
          continue;
        }
        for (final clip in going) {
          track.entry.layer.deleteClip(clip: clip.id);
        }
      }
    });
    setState(() => _pickedClips = const {});
    _afterWrite();
  }

  /// The clips a clip menu acts on: everything picked when the clip is among
  /// the picks, and otherwise the clip alone, which the menu then picks.
  List<(AudioTrackRow, BridgeClip)> _menuClips(
      AudioTrackRow track, BridgeClip clip) {
    if (!_pickedClips.contains(clip.id.toString())) {
      setState(() => _pickedClips = {clip.id.toString()});
      return [(track, clip)];
    }
    return _pickedOnTracks();
  }

  /// Copy the picked clips to the clipboard Cut's clips go on, and say whether
  /// anything was taken.
  bool _copyClips() {
    final ui = _ui;
    final comp = ui?.selectedComp;
    if (ui == null || comp == null) return false;
    final ids = [for (final (_, clip) in _pickedOnTracks()) clip.id];
    if (ids.isEmpty) return false;
    final result = _cut(() => comp.cutCopy(clips: ids, linked: false));
    ui.reportCut(result);
    if (result != BridgeCutResult.done) return false;
    _copiedClips = true;
    cutClipsCopiedIn = Provider.of<LumitState>(context, listen: false).project;
    return true;
  }

  /// Whether a paste would have clips to land, wherever they were copied.
  bool get _clipsHeld =>
      cutClipsHeld(Provider.of<LumitState>(context, listen: false).project);

  /// Put the copied clips down at the playhead: on [onto], or on the track of
  /// the pick. Says whether there was anything to put down.
  bool _pasteClips({LayerReference? onto}) {
    final ui = _ui;
    final comp = ui?.selectedComp;
    if (ui == null || comp == null) return false;
    final picked = _pickedOnTracks();
    final BridgeCutPaste paste;
    try {
      paste = comp.cutPaste(
          atFrame: ui.playheadFrame.value,
          target: onto ??
              (picked.isEmpty
                  ? ui.selectedLayers.value.firstOrNull
                  : picked.first.$1.entry.layer));
    } catch (_) {
      return false;
    }
    if (paste.result == BridgeCutResult.nothing) return false;
    ui.reportCut(paste.result);
    if (paste.result == BridgeCutResult.done) {
      _afterWrite();
      ui.scrubTo(paste.endFrame);
    }
    return true;
  }

  /// A Cut call, with the one throw it can make, a clip that went away.
  BridgeCutResult _cut(BridgeCutResult Function() call) {
    try {
      return call();
    } catch (_) {
      return BridgeCutResult.nothing;
    }
  }

  /// A Cut call's answer: said in the status line, and the table freshened when
  /// it wrote.
  void _afterCut(BridgeCutResult result) {
    final ui = _ui;
    if (ui == null || !mounted) return;
    ui.reportCut(result);
    if (result == BridgeCutResult.done) _afterWrite();
  }

  /// Cut: the picked clips copied and gone. Keys answer further down the shell's
  /// ladder, through Copy then Delete, so this only speaks for clips.
  bool _cutClaim() {
    if (!mounted || !_keysAreOurs || _laneKeys.value.isNotEmpty) return false;
    if (!_copyClips()) return false;
    _deleteClips(_pickedOnTracks());
    return true;
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
      final index = int.tryParse(id.substring(hash + 1));
      if (index == null) continue;
      for (final channel in channels) {
        if (channel.path != path) continue;
        if (channel.ownKeyIndex(index) case final own?) {
          out.add('${channel.id}#$own');
        }
      }
    }
    return out;
  }

  /// Copy takes the picked keyframes, or else the picked clips, and says
  /// whether it took any - a copy that captured nothing must not swallow the
  /// chord and leave the last one on the clipboard for Paste to put down again.
  bool _copyClaim() {
    final ui = _ui;
    final comp = ui?.selectedComp;
    if (!mounted || ui == null || comp == null || !_keysAreOurs) return false;
    if (_keysOnChannels(_keyChannels(ui)).isEmpty) return _copyClips();
    return _copyKeys(ui);
  }

  /// Copy the picked keyframes, and say whether it took any.
  bool _copyKeys(LumitUiState ui) {
    final comp = ui.selectedComp;
    if (comp == null) return false;
    final channels = _keyChannels(ui);
    final took = copySelectedKeys(
      comp: comp,
      channels: channels,
      selectedKeys: _keysOnChannels(channels),
      fps: ui.model.fps,
    );
    if (took) _copiedClips = false;
    return took;
  }

  /// Paste puts the keys down at the playhead, on the rows the selection sits
  /// on, or the clips if clips were the last thing copied here.
  bool _pasteClaim() {
    final ui = _ui;
    if (!mounted || ui == null || !_keysAreOurs) return false;
    if (_copiedClips) return _pasteClips();
    return _pasteKeys(ui);
  }

  bool _pasteKeys(LumitUiState ui) {
    if (graphKeyClipboard.isEmpty) return false;
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

  /// Delete the picked keyframes, as one undo step.
  bool _deleteKeys() {
    final ui = _ui;
    if (!mounted || ui == null) return false;
    final channels = _keyChannels(ui);
    final selection = _keysOnChannels(channels);
    if (selection.isEmpty) return false;
    asOneUndoStep(
      Provider.of<LumitState>(context, listen: false).project,
      () => deleteKeysFromChannels(channels: channels, selectedKeys: selection),
    );
    _laneKeys.value = {};
    _afterWrite();
    return true;
  }

  /// The right-click menu on a lane keyframe, the one every key has. A key that
  /// is not picked is picked first, so the menu acts on what was clicked, and
  /// on a picked one it acts on all of them.
  void _keyMenu(String id, Offset position) {
    if (!_laneKeys.value.contains(id)) _selectKey(id, false);
    final ui = _ui;
    if (ui == null) return;
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    showMenuAt<void>(
      context: context,
      position: position,
      width: keyMenuWidth,
      rows: (close) => channelKeyMenuRows(
        context,
        close: () => close(null),
        keyPrefix: 'atl-key-menu',
        channels: () => _keyChannels(ui),
        picked: _keysOnChannels,
        count: _laneKeys.value.length,
        onChanged: _afterWrite,
        project: Provider.of<LumitState>(context, listen: false).project,
        fps: ui.model.fps,
        fpsNum: fpsNum,
        fpsDen: fpsDen,
        onCopy: () => _copyKeys(ui),
        onPaste: () => _pasteKeys(ui),
        onDelete: _deleteKeys,
      ),
    );
  }

  /// A shape sent from the Easing panel, onto every span the picked keyframes
  /// cover - one press, one undo step.
  void _applyEasing(EasingCurve curve, EaseSides sides) {
    final ui = _ui;
    if (!mounted || ui == null) return;
    final channels = _keyChannels(ui);
    final selection = _keysOnChannels(channels);
    if (selection.isEmpty) return;
    asOneUndoStep(
      Provider.of<LumitState>(context, listen: false).project,
      () => applyEasingToSelection(
          channels: channels,
          selectedKeys: selection,
          curve: curve,
          sides: sides),
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
      // An Audio layer is sound by what it is, even before anything is on
      // it, so an empty one made to put clips on is listed too.
      if (entry.info.kind == BridgeLayerKind.audio ||
          entry.info.kind == BridgeLayerKind.bus) {
        _hasAudio[id] = true;
        _hasPicture[id] = false;
        continue;
      }
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
    // Once the zoom lands the pictures are asked for again, after the layout
    // that puts the scroll where the zoom left it.
    if (!_zoomMotion.moving) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _refreshPeaks(_lastTracks);
      });
    }
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
    _zoomMotion.goTo(z, motion: fly ? _zoomFlight : MotionSpec.still);
  }

  void _zoomDragStart() {
    _anchorOnPlayhead();
    _zoomAnchorHeld = true;
  }

  void _zoomDragEnd() => _zoomAnchorHeld = false;

  /// The navigator asked for a window: `start` frames from the left, `span`
  /// frames across. The window's left edge is the anchor and is held for the
  /// length of the gesture, so dragging its right-hand end zooms about its
  /// left-hand one. A pan only scrolls (see the Timeline panel's own).
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
    final keymap = Provider.of<LumitUiState>(context, listen: false).keymap;
    if (keymap.wheelHeld(BridgeWheelAction.zoomTime)) {
      _zoomAnchorViewportX = contentX - (_hLane.hasClients ? _hLane.offset : 0);
      _zoomAnchorFrame = axis.frameAtExact(contentX);
      _zoomMotion.nudge(
        wheelDelta(event) < 0 ? 1.2 : 1 / 1.2,
        motion: _zoomFlight,
      );
      return;
    }
    if (keymap.wheelHeld(BridgeWheelAction.scrollSideways)) {
      _scrollBy(_hLane, wheelDelta(event));
    }
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
    _zoomFlight = scope.motion.zoom;
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
      busNames: {
        for (final l in ui.model.layers)
          if (l.info.kind == BridgeLayerKind.bus)
            l.layer.internallayerId.toString(): l.info.name,
      },
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
      playhead: ui.playheadFrame,
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
            builder: (context, candidate, _) => AnimatedContainer(
              duration: scope.motion.dropZone.duration,
              curve: scope.motion.dropZone.curve,
              // A live outline while something is over it, so the drop is
              // visibly going to land rather than being taken on faith.
              // Always a decoration, clear until then: adding one only
              // then rebuilt the lanes under it from nothing.
              foregroundDecoration: BoxDecoration(
                  border: Border.all(
                      color: candidate.isEmpty
                          ? t.accent.withValues(alpha: 0)
                          : t.accent,
                      width: 2)),
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
                        // macOS bounces past the ends by default, and the
                        // other half stops there, so the rows come apart.
                        physics: const ClampingScrollPhysics(),
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
      height: t.density.navigatorBand + t.density.timelineChromeRow,
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
            GestureDetector(
              onSecondaryTapUp: (e) => _headerMenu(e.globalPosition),
              child: const AudioTrackHeaderRow(),
            ),
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
                        child: Listener(
                          behavior: HitTestBehavior.translucent,
                          onPointerDown: (e) => _groundDown(e, heights),
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
                            colour: rowSeamColour(t),
                            phase: -((positionOf(_vOutline)?.pixels ?? 0) %
                                t.density.laneRow),
                            // A track is one row of the table, several lane
                            // rows tall, so the seams inside its wave are left
                            // out here as they are on the lanes; the grid
                            // repeats from this overlay's own top edge, so
                            // each blank is carried up by however far the rows
                            // have scrolled.
                            blanks: [
                              for (final b in _seamBlanks(tracks))
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
  }) {
    // What the others in a shared project have in hand, by track and by row.
    // Empty, and nothing walked, outside one.
    final theirs = _sharing ? _share!.inHand(comp) : const <String, List<int>>{};
    final onTrack = theirs[track.id] ?? const <int>[];
    final head = ValueListenableBuilder<List<LayerReference>>(
            valueListenable: ui.selectedLayers,
            builder: (context, picked, _) => GestureDetector(
              // Opaque, because a faded picture row is deaf inside and its
              // menu is still the way to work on it.
              behavior: HitTestBehavior.opaque,
              onSecondaryTapUp: (e) => _headMenu(track, e.globalPosition),
              child: AudioTrackOutlineRow(
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
              onSelect: (buttons) => _pickHead(ui, track, buttons),
              onRenameCommit: () => _commitRename(track),
              onRenameCancel: _cancelRename,
              onChanged: _afterWrite,
              onResize: (dy) => _resizeTrack(track, dy),
              onResizeStart: () => _rowCarry = 0,
              onResizeReset: () =>
                  setState(() => _trackRows.remove(track.id)),
            ),
            ),
          );
    return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // The pick rides on the shell's own list, so choosing a track
          // repaints this row and its neighbour and rebuilds no panel.
          if (onTrack.isEmpty)
            head
          else
            DecoratedBox(
              position: DecorationPosition.foreground,
              decoration:
                  TheirBar([for (final c in onTrack) t.personColour(c)]),
              child: head,
            ),
          // What the twirl opens comes up into place, as an Effect controls
          // section does; the lane half plays the same entrance beside it.
          OpenReveal(
            key: ValueKey<String>('atl-open-${track.id}'),
            open: track.drawnRows.isNotEmpty,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
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
              others: theirs[foldRowPath(track.id, row)] ?? const [],
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
            ),
          ),
        ],
      );
  }

  /// A press on a track's head: Ctrl or Shift adds it to the pick, a right
  /// press keeps a pick it is already in, so the menu can act on all of them,
  /// and anything else picks it alone.
  void _pickHead(LumitUiState ui, AudioTrackRow track, int buttons) {
    _headPicked = true;
    final layer = track.entry.layer;
    final isIn = ui.selectedLayerIds.contains(layer.internallayerId);
    if (HardwareKeyboard.instance.isControlPressed ||
        HardwareKeyboard.instance.isShiftPressed) {
      ui.setSelection([
        for (final l in ui.selectedLayers.value)
          if (l.internallayerId != layer.internallayerId) l,
        if (!isIn) layer,
      ]);
      return;
    }
    if (buttons == kSecondaryMouseButton && isIn) return;
    ui.setSelection([layer]);
  }

  /// A row of the menu that chooses a lane picture, with its own small drawing.
  Widget _laneRow(String key, String label, bool spectral, bool ticked,
          VoidCallback act) =>
      MenuRow(
        key: ValueKey<String>(key),
        onPressed: act,
        child: Row(children: [
          menuTick(ticked),
          CustomPaint(
            size: const Size.square(iconSize),
            painter: LaneGlyphPainter(
                spectral: spectral,
                colour: ThemeScope.of(context).theme.textSecondary),
          ),
          const SizedBox(width: 6),
          Expanded(child: Text(label)),
        ]),
      );

  /// The Lane row: which picture a track draws, on the tracks given.
  Widget _laneSubmenu(List<AudioTrackRow> on, VoidCallback closeParent) {
    final wave = on.every((t) => _modeOf(t.id) == LaneMode.wave);
    final spectral = on.every((t) => _modeOf(t.id) == LaneMode.spectral);
    void pick(LaneMode mode, VoidCallback dismiss) {
      dismiss();
      setState(() {
        for (final t in on) {
          _laneMode[t.id] = mode;
        }
      });
    }

    return SubmenuRow(
      key: const ValueKey<String>('atl-menu-lane'),
      closeParent: closeParent,
      submenu: (dismiss) => FloatSurface(
        width: 150,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _laneRow('atl-menu-lane-wave', l10n.laneModeWave, false, wave,
                () => pick(LaneMode.wave, dismiss)),
            _laneRow('atl-menu-lane-spectral', l10n.laneModeSpectral, true,
                spectral, () => pick(LaneMode.spectral, dismiss)),
          ],
        ),
      ),
      child: MenuLabel(l10n.columnLane, ticked: false),
    );
  }

  /// The track head's right-click: the sound layer's own menu, with the rows
  /// only a track has beside it. Picked tracks all answer.
  void _headMenu(AudioTrackRow track, Offset at) {
    final ui = _ui;
    final comp = ui?.selectedComp;
    if (ui == null || comp == null) return;
    final layers = ui.model.layers;
    final buses = [
      for (final l in layers)
        if (l.info.kind == BridgeLayerKind.bus) l,
    ];
    final targets = track.dimmed
        ? [track.entry]
        : audioMenuTargets(
            track.entry,
            [for (final t in _lastTracks) if (!t.dimmed) t.entry],
            ui.selectedLayerIds);
    final targetIds = {for (final t in targets) t.layer.internallayerId};
    final tracks = [
      for (final t in _lastTracks)
        if (targetIds.contains(t.entry.layer.internallayerId)) t,
    ];
    showMenuAt<void>(
      context: context,
      position: at,
      width: 200,
      rows: (close) => [
        ...audioLayerRows(
          theme: ThemeScope.of(context).theme,
          comp: comp,
          app: Provider.of<LumitState>(context, listen: false),
          entry: track.entry,
          targets: targets,
          buses: buses,
          frame: ui.playheadFrame.value,
          normaliseTargetDb: ui.workspace.normaliseTargetDb,
          closeParent: () => close(null),
          onRename: () => _beginRename(track),
          onChanged: _afterStackEdit,
          pictureRow: track.dimmed,
          extra: soundHeadRows(
            close: () => close(null),
            envelopeShown: (kind) =>
                _envelopes.of(track.entry, track.id, kind),
            toggleEnvelope: (kind) => _toggleEnvelope(kind, tracks),
            lane: track.entry.info.kind == BridgeLayerKind.bus || track.dimmed
                ? null
                : _laneSubmenu(tracks, () => close(null)),
            select: track.entry.info.clips.isEmpty
                ? null
                : () {
                    setState(() => _pickedClips = {
                          for (final c in track.entry.info.clips)
                            c.id.toString(),
                        });
                    ui.setSelection([track.entry.layer]);
                  },
            moveUp: _lastTracks.first == track
                ? null
                : () => _moveTrack(track, -1),
            moveDown: _lastTracks.last == track
                ? null
                : () => _moveTrack(track, 1),
            // The usual height is the least one here, so making every
            // track as short as it goes puts them all back to it.
            minimise:
                _trackRows.isEmpty ? null : () => setState(_trackRows.clear),
            resetHeight: tracks.any((t) => _trackRows.containsKey(t.id))
                ? () => setState(() {
                      for (final t in tracks) {
                        _trackRows.remove(t.id);
                      }
                    })
                : null,
          ),
        ),
      ],
    );
  }

  /// The column header's right-click: what can be said of the whole table.
  void _headerMenu(Offset at) {
    final tracks = [
      for (final t in _lastTracks)
        if (t.entry.info.kind != BridgeLayerKind.bus && !t.dimmed) t,
    ];
    showMenuAt<void>(
      context: context,
      position: at,
      width: 180,
      rows: (close) => [
        if (tracks.isNotEmpty) _laneSubmenu(tracks, () => close(null)),
        MenuRow(
          key: const ValueKey<String>('atl-menu-reset-heights'),
          enabled: _trackRows.isNotEmpty,
          onPressed: () {
            close(null);
            setState(_trackRows.clear);
          },
          child: MenuLabel(l10n.audioTracksResetHeights,
              ticked: false, enabled: _trackRows.isNotEmpty),
        ),
      ],
    );
  }

  void _pickProperty(String path) {
    setState(() => _selectedProperties = [path]);
    // This panel's picks are its own, so the others in a shared project are
    // told of the row the way a panel with no selection tells them.
    _ui?.touchProperty(path);
  }

  /// *Detach audio* on a faded picture row: its sound goes onto an Audio layer
  /// of its own and the picture row is muted, which takes it off this list and
  /// puts the new track in its place.
  Future<void> _detachAudio(BridgeLayerEntry entry) async {
    await detachLayerAudio(
        Provider.of<LumitState>(context, listen: false), entry);
    if (mounted) _afterWrite();
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
        await comp.addAudioLayer(footage: f, arrival: _ui?.streamsArrival);
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
    // Below the last track: a track of the clip's own.
    final toNewTrack = edit.grab == BarGrab.move &&
        index == null &&
        at != null &&
        audioBelowTracks(_lastTracks, at.dy);
    // A move that goes nowhere writes nothing - and on an unconverted layer
    // that matters, because the conversion would be the whole of the edit.
    if (edit.grab == BarGrab.move &&
        edit.shift == 0 &&
        edit.sub == 0 &&
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
            final fps = _ui?.model.fps ?? 0;
            final ticks = fps > 0 ? (edit.sub / fps * 48000).round() : 0;
            if (onward != null || toNewTrack) {
              layer.moveClip(
                  clip: clip.id, target: onward, toFrame: to, overlap: true);
              // On to another track and between two frames: the move puts it
              // on the frame, and the part of a frame follows on its new
              // layer, inside the same undo step.
              if (onward != null && ticks != 0) {
                onward.slideClip(
                    clip: clip.id, toFrame: to, overlap: true, nudgeTicks: ticks);
              }
            } else if (edit.shift != 0 || ticks != 0) {
              layer.slideClip(
                  clip: clip.id, toFrame: to, overlap: true, nudgeTicks: ticks);
            }
          case BarGrab.trimIn || BarGrab.trimOut:
            final into = edit.grab == BarGrab.trimIn;
            final start = clip.startFrame + (into ? edit.shift : 0);
            final end = clip.endFrame + (into ? 0 : edit.shift);
            // Shift held: the clip plays faster or slower to fill its new
            // length, rather than showing more or less of its sound.
            if (edit.stretch) {
              layer.stretchClip(
                  clip: clip.id, startFrame: start, endFrame: end);
            } else {
              layer.trimClip(
                  clip: clip.id, startFrame: start, endFrame: end);
            }
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
      for (final instance in track.entry.layer
          .getClipEffects(clip: clip.id, onCompClock: false)) {
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

  /// A right click on the ground under the last track. [lanes] says the press
  /// came through the lane half, where it may also be on the gap between two
  /// clips.
  void _groundDown(PointerDownEvent e, List<double> heights,
      {bool lanes = false}) {
    final ui = _ui;
    final comp = ui?.selectedComp;
    if (ui == null || comp == null || e.buttons != kSecondaryMouseButton) {
      return;
    }
    final y = e.localPosition.dy;
    final frame = lanes ? _laneAxis.frameAt(e.localPosition.dx) : null;
    if (y >= heights.fold<double>(0, (a, b) => a + b)) {
      _groundMenu(ui, comp, e.position, frame);
      return;
    }
    if (!lanes) return;
    final index = audioTrackAt(_lastTracks, y);
    if (index == null) return;
    final track = _lastTracks[index];
    final top = _lastTracks.take(index).fold<double>(0, (a, t) => a + t.height);
    if (y - top >= track.laneHeight) return;
    final info = track.entry.info;
    // The clips' own right-click answers on a clip, and an uncut layer's bar is
    // one clip until its first cut.
    final x = e.localPosition.dx;
    final onBar = info.clips.isEmpty &&
        x >= _laneAxis.xOf(info.inFrame.toInt()) &&
        x <= _laneAxis.xOf(info.outFrame.toInt());
    if (onBar || audioClipGrabAt(info.clips, _laneAxis, x) != null) return;
    _gapMenu(ui, comp, track, e.position, frame!);
  }

  /// The ground under the last track: a new track, and what the table offers
  /// that is not about any one of them.
  void _groundMenu(LumitUiState ui, CompositionReference comp, Offset at,
      int? frame) {
    showMenuAt<void>(
      context: context,
      position: at,
      width: 230,
      // The Cut timeline's order: paste, insert, select, then time.
      rows: (close) => [
        MenuRow(
          key: const ValueKey<String>('atl-menu-ground-paste'),
          enabled: _clipsHeld,
          onPressed: () {
            close(null);
            _pasteClips();
          },
          child: MenuLabel(l10n.menuPaste,
              action: 'edit.paste', enabled: _clipsHeld, ticked: false),
        ),
        const MenuDivider(),
        for (final (action, label) in [
          ('layer.new.audio', l10n.insertAudioLayer),
          ('layer.new.bus', l10n.insertBus),
        ])
          MenuRow(
            key: ValueKey<String>('insert-$action'),
            onPressed: () {
              close(null);
              insertEmptyLayer(ui, comp, action);
              _afterWrite();
            },
            child: MenuLabel(label, action: action, ticked: false),
          ),
        const MenuDivider(),
        _selectAllRow(() => close(null)),
        if (frame != null) ...[
          const MenuDivider(),
          _addMarkerRow(comp, frame, () => close(null)),
        ],
      ],
    );
  }

  /// A gap on a track: what can go there, or be closed up around it.
  void _gapMenu(LumitUiState ui, CompositionReference comp,
      AudioTrackRow track, Offset at, int frame) {
    final clips = track.entry.info.clips;
    // Something has to come after the gap for it to close on.
    final canClose = clips.any((c) => c.startFrame > frame);
    showMenuAt<void>(
      context: context,
      position: at,
      width: 200,
      rows: (close) => [
        MenuRow(
          key: const ValueKey<String>('atl-menu-gap-paste'),
          enabled: _clipsHeld,
          onPressed: () {
            close(null);
            _pasteClips(onto: track.entry.layer);
          },
          child: MenuLabel(l10n.menuPaste,
              action: 'edit.paste', enabled: _clipsHeld, ticked: false),
        ),
        MenuRow(
          key: const ValueKey<String>('atl-menu-gap-close'),
          enabled: canClose,
          onPressed: () {
            close(null);
            _afterCut(_cut(() =>
                comp.cutCloseGap(layer: track.entry.layer, atFrame: frame)));
          },
          child: MenuLabel(l10n.cutCloseGap, enabled: canClose, ticked: false),
        ),
        const MenuDivider(),
        _selectAllRow(() => close(null)),
        const MenuDivider(),
        _addMarkerRow(comp, frame, () => close(null)),
      ],
    );
  }

  Widget _selectAllRow(VoidCallback close) => MenuRow(
        key: const ValueKey<String>('atl-menu-select-all'),
        onPressed: () {
          close();
          setState(() => _pickedClips = {
                for (final t in _lastTracks)
                  for (final c in t.entry.info.clips) c.id.toString(),
              });
        },
        child: MenuLabel(l10n.menuSelectAll,
            action: 'edit.select.all', ticked: false),
      );

  Widget _addMarkerRow(
          CompositionReference comp, int frame, VoidCallback close) =>
      MenuRow(
        key: const ValueKey<String>('atl-menu-add-marker'),
        onPressed: () {
          close();
          addMarkerFrb(comp, frame: frame);
          _afterWrite();
        },
        child: MenuLabel(l10n.menuAddMarkerHere,
            action: 'marker.add', ticked: false),
      );

  /// The fade menu: a right click on a fade, or anywhere in an overlap. The
  /// five shapes by name, then the editor.
  ///
  /// A preset lands on both sides of an overlap, because a crossfade is a pair
  /// and Fast against Fast is what keeps the join as loud as either clip was.
  Future<void> _fadeMenu(
      AudioTrackRow track, BridgeClip clip, bool into, Offset at) async {
    final held = into ? clip.fadeIn : clip.fadeOut;
    final partner = clipFadePartner(track.entry.info.clips, clip, into: into);
    await showMenuAt<void>(
      context: context,
      position: at,
      width: 180,
      rows: (close) => [
        for (final preset in clipFadeShapes)
          MenuRow(
            key: ValueKey<String>('atl-fade-shape-${preset.id}-${clip.id}'),
            onPressed: () {
              close(null);
              _writeFadeShapes(track, clip,
                  into: into, outgoing: preset.shape, incoming: preset.shape);
            },
            child: fadeShapeLabel(preset, falling: !into, ticked: preset.shape == held.shape),
          ),
        const MenuDivider(),
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
          child: MenuLabel(l10n.clipFadeCustom, ticked: false),
        ),
        // An overlap is its fade: it goes when the clips stop overlapping.
        if (partner == null && held.seconds > 0)
          MenuRow(
            key: ValueKey<String>('atl-fade-remove-${clip.id}'),
            onPressed: () {
              close(null);
              _writeFade(track, (clip: clip, into: into, seconds: 0));
            },
            child: MenuLabel(
                into ? l10n.cutRemoveFadeIn : l10n.cutRemoveFadeOut,
                ticked: false),
          ),
      ],
    );
  }

  /// Split or merge the audio streams of one clip, and say so in the status
  /// line where there was nothing to do.
  void _clipStreams(CompositionReference comp, BridgeClip clip,
      {required bool split}) {
    var done = false;
    try {
      final result = split
          ? comp.cutSplitAudioStreams(clips: [clip.id])
          : comp.cutMergeAudioStreams(clips: [clip.id]);
      done = result == BridgeCutResult.done;
    } catch (_) {}
    if (!mounted) return;
    if (!done) {
      Provider.of<LumitState>(context, listen: false).postNotice(split
          ? l10n.audioStreamsNothingToSplit
          : l10n.audioStreamsNothingToMerge);
      return;
    }
    _afterWrite();
  }

  /// Write on the clip a menu row is about. An uncut layer's bar has no clip
  /// yet, so the first thing done to it from the menu makes the one clip it has
  /// always been, inside the same undo step.
  void _onClip(AudioTrackRow track, BridgeClip? clip,
      void Function(BridgeClip clip) write) {
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
      var held = clip;
      if (held == null) {
        if (!_makeSequenced(track.entry.layer)) return;
        final made = track.entry.layer.getClips();
        if (made.isEmpty) return;
        held = made.first;
      }
      write(held);
    });
    _afterWrite();
  }

  /// A clip's right-click, grouped as every menu is: what it does, the
  /// clipboard, how it sits with the others, its colour, and Delete last. Rows
  /// on a pick of several act on all of them. On an uncut layer's bar [clip] is
  /// null, and only the rows that make sense of one sound without a clip show.
  Future<void> _clipMenu(
      AudioTrackRow track, BridgeClip? clip, Offset at) async {
    final ui = _ui;
    final comp = ui?.selectedComp;
    if (ui == null || comp == null) return;
    final layer = track.entry.layer;
    final info = track.entry.info;
    final picked = clip == null ? null : _menuClips(track, clip);
    final ids = [
      for (final (_, c) in picked ?? const <(AudioTrackRow, BridgeClip)>[]) c.id,
    ];
    final playhead = ui.playheadFrame.value;
    final first = clip?.startFrame.toInt() ?? info.inFrame.toInt();
    final last = clip?.endFrame.toInt() ?? info.outFrame.toInt();
    final canSplit = first < playhead && playhead < last;
    BridgeClipFade? held(bool into) => into ? clip?.fadeIn : clip?.fadeOut;
    // Whether the track's sound keeps its pitch when it plays faster or
    // slower. The whole layer's, so every clip on it follows.
    final follows = info.pitchFollowsSpeed;
    // The audio streams of the clip's file, for the rows that choose among
    // them. The track draws the clip's wave, so its file has been opened.
    BridgeAudioStreams? streams;
    if (clip != null) {
      try {
        streams = layer.clipAudioStreams(clip: clip.id);
      } catch (_) {}
    }
    final several = streams != null && streams.streams.length > 1;
    Widget shapes(VoidCallback dismiss, {required bool into}) => FloatSurface(
          width: 150,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final preset in clipFadeShapes)
                MenuRow(
                  key: ValueKey<String>(
                      'atl-fade-${into ? 'in' : 'out'}-${preset.id}-${clip?.id ?? track.id}'),
                  onPressed: () {
                    dismiss();
                    _onClip(track, clip, (held) {
                      for (final (on, c) in picked ?? [(track, held)]) {
                        try {
                          _setFadeShape(on, c, preset.shape, into: into);
                        } catch (_) {
                          // A locked track refuses, and the rest still take it.
                        }
                      }
                    });
                  },
                  child: fadeShapeLabel(preset,
                      falling: !into,
                      ticked: (held(into)?.seconds ?? 0) > 0 &&
                          held(into)?.shape == preset.shape),
                ),
            ],
          ),
        );
    Widget row(String key, String label, VoidCallback act, void Function() close,
            {String? action, bool enabled = true, bool? ticked = false}) =>
        MenuRow(
          key: ValueKey<String>('atl-clip-$key-${clip?.id ?? track.id}'),
          enabled: enabled,
          onPressed: () {
            close();
            act();
          },
          child:
              MenuLabel(label, action: action, enabled: enabled, ticked: ticked),
        );

    await showMenuAt<void>(
      context: context,
      position: at,
      width: 210,
      rows: (closeWith) {
        void close() => closeWith(null);
        return [
          row('split', l10n.cutMenuSplit, () {
            _onClip(track, clip, (_) => layer.cutClipAt(frame: playhead));
          }, close, action: 'cut.add.edit', enabled: canSplit),
          if (clip != null)
            SubmenuRow(
              key: ValueKey<String>('atl-clip-trim-${clip.id}'),
              closeParent: close,
              submenu: (dismiss) => FloatSurface(
                width: 220,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    for (final (key, label, start, ripple) in [
                      ('start', l10n.cutTrimStart, true, false),
                      ('end', l10n.cutTrimEnd, false, false),
                      ('ripple-start', l10n.cutRippleTrimStart, true, true),
                      ('ripple-end', l10n.cutRippleTrimEnd, false, true),
                    ])
                      row('trim-$key', label, () {
                        _trimClips(comp, picked!, playhead,
                            start: start, ripple: ripple);
                      }, dismiss,
                          action: 'cut.trim.${start ? 'start' : 'end'}'
                              '${ripple ? '.ripple' : ''}',
                          enabled: canSplit),
                  ],
                ),
              ),
              child: MenuLabel(l10n.cutTrimMenu, ticked: false),
            ),
          SubmenuRow(
            key: ValueKey<String>('atl-clip-transition-${clip?.id ?? track.id}'),
            closeParent: close,
            submenu: (dismissAll) => FloatSurface(
              width: 150,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SubmenuRow(
                    key: ValueKey<String>(
                        'atl-clip-fade-in-${clip?.id ?? track.id}'),
                    closeParent: dismissAll,
                    submenu: (dismiss) => shapes(dismiss, into: true),
                    child: MenuLabel(l10n.clipFadeIn),
                  ),
                  SubmenuRow(
                    key: ValueKey<String>(
                        'atl-clip-fade-out-${clip?.id ?? track.id}'),
                    closeParent: dismissAll,
                    submenu: (dismiss) => shapes(dismiss, into: false),
                    child: MenuLabel(l10n.clipFadeOut),
                  ),
                ],
              ),
            ),
            child: MenuLabel(l10n.cutTransitionMenu, ticked: false),
          ),
          // A hold on the sound under the playhead: a length of silence, with
          // what follows moved later.
          if (clip != null)
            row('freeze', l10n.cutFreezeFrame, () {
              askToFreezeClips(context, [(layer: layer, clip: clip)], at);
            }, close, enabled: first <= playhead && playhead < last),
          ...soundClipRows(
            close: close,
            tickColumn: true,
            keepPitch: !follows,
            onKeepPitch: () {
              layer.setPitchFollowsSpeed(follows: !follows);
              _afterWrite();
            },
            onNormalise: () async {
              final app = Provider.of<LumitState>(context, listen: false);
              final target = ui.workspace.normaliseTargetDb;
              if (picked != null) {
                await normaliseClips(app, target,
                    [for (final (on, c) in picked) (on.entry.layer, c.id)],
                    then: _afterWrite);
              } else {
                await normaliseLayers(app, comp, playhead, target, [track.entry],
                    then: _afterWrite);
              }
            },
            onLoudness: picked == null
                ? null
                : (lufs) => normaliseClips(
                    Provider.of<LumitState>(context, listen: false),
                    lufs,
                    [for (final (on, c) in picked) (on.entry.layer, c.id)],
                    then: _afterWrite,
                    loudness: true),
            // An uncut layer's bar has no clip of its own to set back.
            resetGainShown: picked != null,
            onResetGain: picked != null && picked.any((p) => p.$2.gainDb != 0)
                ? () => resetClipGain(
                    Provider.of<LumitState>(context, listen: false),
                    [for (final (on, c) in picked) (on.entry.layer, c)],
                    then: _afterWrite)
                : null,
          ),
          if (clip != null) ...[
            if (clip.takes.length > 1)
              SubmenuRow(
                key: ValueKey<String>('atl-clip-takes-${clip.id}'),
                closeParent: close,
                submenu: (dismiss) => FloatSurface(
                  width: 220,
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      for (var n = 0; n < clip.takes.length; n++)
                        MenuRow(
                          key: ValueKey<String>('atl-clip-take-$n'),
                          onPressed: () {
                            dismiss();
                            if (n == 0) return;
                            _afterCut(_cut(
                                () => comp.cutSetTake(clip: clip.id, take: n)));
                          },
                          child: MenuLabel(clip.takes[n], ticked: n == 0),
                        ),
                      const MenuDivider(),
                      MenuRow(
                        key: const ValueKey<String>('atl-clip-take-delete'),
                        onPressed: () {
                          dismiss();
                          _afterCut(_cut(() =>
                              comp.cutDeleteTake(clip: clip.id, take: 0)));
                        },
                        child: MenuLabel(l10n.cutDeleteTake, ticked: false),
                      ),
                    ],
                  ),
                ),
                child: MenuLabel(l10n.cutTakeMenu, ticked: false),
              ),
            // A file with several audio streams: which of them this clip
            // plays, a clip to each on a track of its own, and back onto one.
            if (several) ...[
              audioStreamsSubmenu(
                key: ValueKey<String>('atl-clip-audio-streams-${clip.id}'),
                closeParent: close,
                streams: streams!,
                onChoose: (places) {
                  try {
                    layer.setClipAudioStreams(clip: clip.id, streams: places);
                  } catch (_) {}
                  _afterWrite();
                },
              ),
              row('split-streams', l10n.menuSplitAudioStreams,
                  () => _clipStreams(comp, clip, split: true), close),
              row('merge-streams', l10n.menuMergeAudioStreams,
                  () => _clipStreams(comp, clip, split: false), close),
            ],
            SubmenuRow(
              key: ValueKey<String>('atl-clip-add-audio-effect-${clip.id}'),
              closeParent: close,
              submenu: (dismiss) => audioEffectFlyout(dismiss, (name) {
                layer.addClipEffect(clip: clip.id, name: name);
                _afterStackEdit();
              }),
              child: MenuLabel(l10n.audioAddEffect, ticked: false),
            ),
          ],
          // The lane is this timeline's own, so it follows the rows Cut shares.
          _laneSubmenu([track], close),
          if (clip != null) ...[
            const MenuDivider(),
            // What opens or looks at the clip, as Cut has them.
            if ((picked?.length ?? 0) <= 1) ...[
              if (ui.currentViewerPane != null &&
                  !clip.sourceIsComp &&
                  !clip.sourceIsSolid)
                row('open-source', l10n.cutOpenInSource,
                    () => openClipInSource(ui, comp, layer, clip), close),
              row('effect-controls', l10n.effectControls,
                  () => _showClipEffects(ui, layer, clip), close),
            ],
            if ((picked?.length ?? 0) > 1)
              row('fold', l10n.cutNewCompositionFromSelection, () {
                CompositionReference? made;
                try {
                  made = comp.cutClipsToComposition(
                    clips: ids,
                    linked: false,
                    name: l10n.cutSelectionCompName(comp.getSettings().name),
                  );
                } catch (_) {
                  made = null;
                }
                if (made == null) ui.reportCut(BridgeCutResult.locked);
                _afterWrite();
              }, close)
            else if (!clip.sourceIsSolid)
              row(
                  'open-comp',
                  clip.sourceIsComp
                      ? l10n.cutOpenComposition
                      : l10n.cutNewCompositionFromClip, () {
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
                _afterWrite();
                ui.setSelectedComp(made);
              }, close),
            const MenuDivider(),
            row('cut', l10n.menuCut, () {
              if (_copyClips()) _deleteClips(_pickedOnTracks());
            }, close, action: 'edit.cut'),
            row('copy', l10n.menuCopy, () => _copyClips(), close,
                action: 'edit.copy'),
            row('paste', l10n.menuPaste, () => _pasteClips(onto: layer), close,
                action: 'edit.paste', enabled: _clipsHeld),
            row('duplicate', l10n.menuDuplicate,
                () => _duplicateClips(comp, picked!), close,
                action: 'cut.duplicate'),
            row('paste-attributes', l10n.cutPasteAttributes, () {
              _afterCut(_cut(() => comp.cutPasteAttributes(clips: ids)));
            }, close, action: 'cut.paste.attributes', enabled: _clipsHeld),
            const MenuDivider(),
            row('group', l10n.cutGroup, () => _groupClips(join: true), close,
                action: 'cut.group', enabled: ids.length > 1),
            if (clip.group != null)
              row('ungroup', l10n.cutUngroup, () => _groupClips(join: false),
                  close, action: 'cut.ungroup'),
            row('select-to-end', l10n.cutSelectToEnd, () {
              setState(() => _pickedClips = {
                    for (final c in info.clips)
                      if (c.startFrame >= clip.startFrame) c.id.toString(),
                  });
            }, close),
            row('select-track', l10n.cutSelectTrack, () {
              setState(() => _pickedClips = {
                    for (final c in info.clips) c.id.toString(),
                  });
            }, close),
          ],
          const MenuDivider(),
          SubmenuRow(
            key: ValueKey<String>('atl-clip-colour-menu-${clip?.id ?? track.id}'),
            closeParent: close,
            submenu: (dismiss) => clipColourMenu(
              ThemeScope.of(context).theme,
              keyPrefix: 'atl-clip-colour-menu',
              onPick: (label) {
                dismiss();
                _onClip(track, clip, (held) {
                  // On the whole pick, one undo step with the rest.
                  for (final (on, c) in picked ?? [(track, held)]) {
                    try {
                      on.entry.layer.setClipLabel(clip: c.id, label: label);
                    } catch (_) {}
                  }
                });
              },
            ),
            child: MenuLabel(l10n.clipColour, ticked: false),
          ),
          if (clip != null && clip.effects.isNotEmpty)
            row('bypass', l10n.cutBypassEffects, () {
              _toggleClipFx(track, clip);
            }, close, ticked: !clip.fx),
          const MenuDivider(),
          if (picked != null) ...[
            row('delete', l10n.clipDelete, () {
              // A gap, not a closed row: what follows keeps the beat it was
              // cut to.
              _deleteClips(picked);
            }, close, action: 'edit.delete.selection'),
            row('ripple-delete', l10n.cutRippleDelete,
                () => _rippleDeletePicked(ui), close,
                action: 'cut.delete.ripple'),
          ] else
            row('delete-layer', l10n.delete, () {
              layer.delete();
              _afterWrite();
            }, close, action: 'edit.delete.selection'),
        ];
      },
    );
  }

  /// The picked clips joined in a group that moves together in the Cut
  /// timeline, or let go of it. One undo step.
  void _groupClips({required bool join}) {
    final picked = _pickedOnTracks();
    final group = join ? UuidValue.fromString(const Uuid().v4()) : null;
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
      for (final (track, clip) in picked) {
        try {
          track.entry.layer.setClipGroup(clip: clip.id, group: group);
        } catch (_) {}
      }
    });
    _afterWrite();
  }

  /// Cut at the playhead: the tracks of the picked clips, or the picked tracks,
  /// or every track. A faded picture row has no sound here to cut.
  void _splitAtPlayhead(LumitUiState ui) {
    final tracks = [for (final t in _lastTracks) if (!t.dimmed) t];
    final holding = {for (final (track, _) in _pickedOnTracks()) track};
    final picked = holding.isNotEmpty
        ? holding
        : tracks.where(
            (t) => ui.selectedLayerIds.contains(t.entry.layer.internallayerId));
    final made = razorCut(
        razorTargets(
          [for (final t in picked.isEmpty ? tracks : picked) t.entry],
          ui.playheadFrame.value,
          clicked: null,
          allLayers: true,
        ),
        ui.playheadFrame.value);
    if (made.cut) _afterWrite();
  }

  /// The picked clips gone, and what follows each moved back to close the room.
  void _rippleDeletePicked(LumitUiState ui) {
    final comp = ui.selectedComp;
    if (comp == null) return;
    final ids = [for (final (_, clip) in _pickedOnTracks()) clip.id];
    _afterCut(
        _cut(() => comp.cutDelete(clips: ids, ripple: true, linked: false)));
    setState(() => _pickedClips = const {});
  }

  /// A copy of the picked clips, straight after them or past the end of their
  /// tracks where something is in the way.
  void _duplicateClips(CompositionReference comp,
      List<(AudioTrackRow, BridgeClip)> picked) {
    final clips = [for (final (_, clip) in picked) clip];
    _afterCut(_cut(() => comp.cutDuplicate(
          clips: [for (final clip in clips) clip.id],
          grabbed: clips.first.id,
          byFrames: cutDuplicateFrames(clips, [
            for (final track in {for (final (track, _) in picked) track})
              track.entry.info.clips,
          ]),
          target: null,
          linked: false,
          how: BridgeCutMove(
            overlap: _ui?.workspace.interface.cutAutoCrossfade ?? false,
            ripple: false,
            nudgeTicks: 0,
            transition: null,
          ),
        )));
  }

  /// One end of each picked clip the playhead is inside brought to the playhead,
  /// closing the room behind it with [ripple]. One undo step.
  /// The clip's own stack in Effect controls, brought up. Another layer
  /// picked lets the clip go, so the panel follows the pick again.
  void _showClipEffects(
      LumitUiState ui, LayerReference layer, BridgeClip clip) {
    ui.setSelection([layer]);
    ui.cutClip.value = (layer: layer, clip: clip.id);
    void release() {
      if (ui.selectedLayer.value?.internallayerId == layer.internallayerId) {
        return;
      }
      ui.selectedLayers.removeListener(release);
      if (ui.cutClip.value?.clip == clip.id) ui.cutClip.value = null;
    }

    ui.selectedLayers.addListener(release);
    setPanelVisible(ui.split, Panel.effectControls, true);
    activatePanelTab(ui.split, Panel.effectControls);
    ui.workspace.touch();
  }

  void _trimClips(CompositionReference comp,
      List<(AudioTrackRow, BridgeClip)> picked, int playhead,
      {required bool start, required bool ripple}) {
    var result = BridgeCutResult.nothing;
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
      for (final (_, clip) in picked) {
        if (clip.startFrame >= playhead || playhead >= clip.endFrame) continue;
        result = _cut(() => comp.cutTrim(
              clip: clip.id,
              startFrame: start ? playhead : clip.startFrame,
              endFrame: start ? clip.endFrame : playhead,
              ripple: ripple,
              linked: false,
              overlap: _ui?.workspace.interface.cutAutoCrossfade ?? false,
              transition: null,
            ));
      }
    });
    _afterCut(result);
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
                        onPointerDown: (e) => _groundDown(e, heights, lanes: true),
                        onPointerMove: (event) {
                          if (event.buttons == kMiddleMouseButton) {
                            _panLanes(event.delta);
                          }
                        },
                        onPointerSignal: (event) {
                          if (event is! PointerScrollEvent) return;
                          final keymap =
                              Provider.of<LumitUiState>(context, listen: false)
                                  .keymap;
                          if (!keymap.wheelHeld(BridgeWheelAction.zoomTime) &&
                              !keymap.wheelHeld(
                                  BridgeWheelAction.scrollSideways)) {
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
                                box: _laneMarquee,
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
                                  if (_pickedClips.isNotEmpty) {
                                    setState(() => _pickedClips = const {});
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
                                      colour: rowSeamColour(t),
                                      // A track is one row of the table,
                                      // several lane rows tall, so the seams
                                      // that would fall inside its wave are
                                      // left out; the ones bounding it stay.
                                      blanks: _seamBlanks(tracks),
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
                            // The marquee's box, over the lanes and the seams
                            // it is dragged across.
                            MarqueeOverlay(box: _laneMarquee),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
          // A comp with nothing to hear offers the first track, rather than
          // leaving an empty table to be puzzled over.
          if (tracks.isEmpty && _search.trim().isEmpty)
            Positioned.fill(
              child: Center(
                child: HouseButton(
                  key: const ValueKey<String>('atl-empty'),
                  small: true,
                  onPressed: () {
                    insertEmptyLayer(ui, comp, 'layer.new.audio');
                    _afterWrite();
                  },
                  child: Text(l10n.insertAudioLayer),
                ),
              ),
            ),
          // Each marker's line, from the ruler down through every track.
          MarkerLinesGround(comp: comp, axis: axis, hScroll: _hLane),
          // The playhead rides above every lane, and it is the only thing here
          // that redraws when it moves - on its own layer, so that is true of
          // the painting and not only of the rebuilding.
          PlayheadOverlay(playhead: ui.playheadFrame, xOf: axis.xOf),
        ],
      ),
    );
  }

  /// Each track's own band, so the seam painter rules between tracks and not
  /// through them, and the ground under the last one, which is not a row and
  /// wears no lines. The twirl's rows are outside the blank and keep their
  /// hairlines, which is what makes an open track read as a list.
  List<(double, double)> _seamBlanks(List<AudioTrackRow> tracks) {
    final out = <(double, double)>[];
    var y = 0.0;
    for (final track in tracks) {
      out.add((y, y + track.laneHeight));
      y += track.height;
    }
    return out..add((y, 1e7));
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
          OpenReveal(
            key: ValueKey<String>('atl-lane-open-${track.id}'),
            open: track.drawnRows.isNotEmpty,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
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
            ),
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
      quantise: _ui?.workspace.interface.quantiseSound ?? true,
      preview: _clipDragOf(track.id),
      selected: _pickedClips,
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
      // Picking a clip picks its track too, as a bar picks its layer in the
      // Timeline, so the Mixer and the Audio panel follow it.
      onSelect: (clip) {
        _headPicked = false;
        final id = clip.id.toString();
        final adds = HardwareKeyboard.instance.isControlPressed ||
            HardwareKeyboard.instance.isShiftPressed;
        setState(() => _pickedClips = adds
            ? (_pickedClips.contains(id)
                ? ({..._pickedClips}..remove(id))
                : {..._pickedClips, id})
            : {id});
        _ui?.setSelection([track.entry.layer]);
      },
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
    // The volume and pan lines over the lane, under the strip the fade
    // handles take so those stay in reach, as on a Cut sound track.
    final bandHeight = height - audioFadeHandleHeight - 2;
    final (fpsNum, fpsDen) = _ui!.model.fpsExact;
    final bands = [
      if (!track.dimmed && !razor && height > 36)
        for (final kind in BandKind.values)
          if (_envelopes.of(track.entry, track.id, kind))
            Positioned(
              key: ValueKey<String>('atl-${kind.name}-${track.id}'),
              left: 0,
              width: axis.width,
              top: audioFadeHandleHeight,
              height: bandHeight,
              child: RepaintBoundary(
                child: VolumeBand(
                  entry: track.entry,
                  axis: axis,
                  fps: fps,
                  fpsNum: fpsNum,
                  fpsDen: fpsDen,
                  barShift: 0,
                  rowHeight: bandHeight,
                  onChanged: _afterWrite,
                  kind: kind,
                  onHide: () => _toggleEnvelope(kind, [track]),
                ),
              ),
            ),
    ];
    if (info.clips.isNotEmpty) {
      return Stack(children: [clips, fades, ...bands]);
    }
    final startOffset = rationalSeconds(info.span.startOffset);
    final secondsPerPixel =
        axis.perFrame <= 0 || fps <= 0 ? 0.0 : 1 / (axis.perFrame * fps);
    // Canvas x 0 is the axis's left padding, comp time 0 sits a padding's width
    // in, and the source's own clock runs from there less wherever the layer
    // starts it.
    final origin = -startOffset - TimelineAxis.pad * secondsPerPixel;
    final label = t.labelColour(info.label);
    return Stack(
      children: [
        // The bar itself, boxed as a clip is: an uncut track is a track of
        // one clip, and should look like one before its first cut as after.
        Positioned(
          key: ValueKey<String>('atl-bar-${track.id}'),
          left: left,
          width: max(0.0, right - left),
          top: 2,
          bottom: 2,
          child: IgnorePointer(
            child: ValueListenableBuilder<List<LayerReference>>(
              valueListenable: _ui!.selectedLayers,
              builder: (context, picked, _) {
                final on = picked.any((l) =>
                    l.internallayerId == track.entry.layer.internallayerId);
                final mark = ThemeScope.of(context).motion.mark;
                // A bus holds no sound of its own, so its bar is the track's
                // length and no more: a tint, not a clip.
                final bus = info.kind == BridgeLayerKind.bus;
                final fill = on ? clipFillSelectedAlpha : clipFillAlpha;
                return AnimatedContainer(
                  duration: mark.duration,
                  curve: mark.curve,
                  decoration: BoxDecoration(
                    color: label.withValues(alpha: bus ? fill * 0.3 : fill),
                    border: Border.all(
                        color: on
                            ? (t.shape == ThemeShape.desk
                                ? t.accent
                                : t.textPrimary)
                            : t.surface0),
                    borderRadius: BorderRadius.circular(clipRadius(t)),
                  ),
                  alignment: Alignment.centerLeft,
                  child: Container(width: clipEdgeWidth, color: label),
                );
              },
            ),
          ),
        ),
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
        // The level as a number at the bar's own right end. A keyed level is
        // drawn on its own row under the twirl, so there is no one number to
        // say.
        if (_levelOf(info.volumeDb) case final reading?)
          Positioned(
            right: axis.width - right + 6,
            bottom: 4,
            child: Text(reading,
                style: t.mono.copyWith(fontSize: 9, color: t.textMuted)),
          ),
        // The corners of the layer's own bar: the first fade dragged on to one
        // converts it, as the first trim does.
        fades,
        ...bands,
      ],
    );
  }

  /// The track's level as a readout, or null while it is keyed.
  /// Unity says nothing, so it is left off the lane.
  String? _levelOf(BridgeScalar scalar) => switch (scalar) {
        BridgeScalar_Static(:final field0) when field0.abs() < 0.05 => null,
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
    // The keys of this lane someone else has selected, in their colours.
    final others = <int, List<Color>>{};
    if (_sharing) {
      final t = ThemeScope.of(context).theme;
      final held = _share!.keysInHand(ui.selectedComp);
      for (var i = 0; held.isNotEmpty && i < keys.length; i++) {
        if (held['$rowId#$i'] case final colours?) {
          others[i] = [for (final c in colours) t.personColour(c)];
        }
      }
    }
    return ValueListenableBuilder<Set<String>>(
      valueListenable: _laneKeys,
      builder: (context, selected, _) => KeyLane(
        key: ValueKey<String>('atl-keys-$rowId'),
        others: others,
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
        onKeyMenu: (index, position) => _keyMenu('$rowId#$index', position),
        onSelectKey: (index, additive) => _selectKey('$rowId#$index', additive),
        onMoveKeys: (moved) =>
            _moveHeldKeys(moved, _lastTracks, fps, fpsNum, fpsDen),
        onChanged: ui.model.refresh,
      ),
    );
  }
}
