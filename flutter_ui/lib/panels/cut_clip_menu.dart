part of 'cut_timeline_panel_frb.dart';

// The clip menu the Cut timeline and the Sequence view share, and the commands
// behind it: one list of rows over one set of commands, so a clip has the same
// menu in both. A view mixes this in and says where its tracks and its picked
// clips are.

/// A footage clip loaded into the source view, stood on the frame under the
/// playhead when that is inside the clip and on its first frame otherwise,
/// with the In and Out marks on the clip's own span. The Cut timeline and the
/// Audio timeline open a clip this way.
void openClipInSource(LumitUiState ui, CompositionReference comp,
    LayerReference layer, BridgeClip clip) {
  if (clip.sourceIsSolid || clip.sourceIsComp) return;
  final at = ui.playheadFrame.value;
  final start = clip.startFrame.toInt();
  final inside = at >= start && at < clip.endFrame.toInt();
  BridgeMatchFrame? hit;
  try {
    hit = comp.cutMatchFrame(frame: inside ? at : start, layer: layer);
  } catch (_) {
    hit = null;
  }
  if (hit == null) {
    ui.reportCut(BridgeCutResult.nothing);
    return;
  }
  ui.showSourceView();
  ui.openFootageViewAt(hit.footage, hit.sourceTime,
      markIn: clip.sourceIn, markOut: clip.sourceOut);
}

mixin CutClipMenu<W extends StatefulWidget> on State<W> {
  LumitUiState? get cutUi;

  /// Every track, the clips on each, which track each clip is on, and the
  /// clips linked to each clip, all as the read model has them.
  List<CutTrack> get cutTracks;
  Map<String, int> get cutTrackOf;
  Map<String, Set<String>> get cutMates;

  /// The picked clips by id. Setting it shows the pick.
  Set<String> get cutPicked;
  set cutPicked(Set<String> picked);

  /// Whether a pick of more than one clip shows in this view, for the rows
  /// that only pick.
  bool get cutPicksMany;

  /// Whether [clip] is drawn as sound, so its line is gain.
  bool cutHeard(CutTrack track, BridgeClip clip);

  /// The clips showing their speed line with the switch off, or null in a
  /// view that draws no speed lines of its own.
  Set<String>? get cutSpeedShown;

  /// Where a paste lands.
  LayerReference? cutPasteTarget(LumitUiState ui);

  InterfaceSettings get _settings => cutUi!.workspace.interface;

  bool get _autoRipple => _settings.cutAutoRipple;

  bool get _autoCrossfade => _settings.cutAutoCrossfade;

  bool get _speedLines => _settings.cutSpeedLines;

  /// How a move is made, as the strip's switches stand.
  BridgeCutMove _how({double sub = 0, bool copy = false}) => BridgeCutMove(
        overlap: _autoCrossfade,
        ripple: _autoRipple && !copy,
        nudgeTicks: (sub / max(1e-9, cutUi?.model.heldFps ?? 0) * 48000).round(),
        transition: _settings.cutDefaultTransition,
      );

  /// Where every edit made here ends: the read model freshened.
  void _afterWrite() => cutUi?.model.refresh();

  /// Where every Cut edit ends: a refusal is said on the status line and
  /// the document is untouched; a done one is told to everyone.
  void _afterCut(BridgeCutResult result) {
    final ui = cutUi;
    if (ui == null || !mounted) return;
    ui.reportCut(result);
    if (result != BridgeCutResult.done) return;
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
    _afterWrite();
  }

  bool get _linked => cutUi?.cutLinked.value ?? true;

  /// [id] and, while Linked is on, the clips linked to it.
  Set<String> _withMates(String id) =>
      _linked ? {id, ...?cutMates[id]} : {id};

  /// The tracks a playhead command works on: the picked clips' own, top
  /// first, else every track.
  List<CutTrack> _candidateTracks() {
    if (cutPicked.isEmpty) return cutTracks;
    final at = <int>{
      for (final id in cutPicked)
        if (cutTrackOf[id] case final i?) i,
    };
    return [for (final i in at.toList()..sort()) cutTracks[i]];
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
    if (cutPicked.isEmpty || tracks.length > 1) {
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
          overlap: _autoCrossfade,
          transition: _settings.cutDefaultTransition,
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
    final tracks = cutPicked.isEmpty ? _unlockedTracks() : _candidateTracks();
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
          layers: cutPicked.isEmpty
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
      [for (final track in cutTracks) if (!track.entry.info.switches.locked) track];

  /// One second in this composition's frames: the default transition.
  int _secondFrames(LumitUiState ui) => max(1, ui.model.fps.round());

  /// [edit] as one History step called [name].
  BridgeCutResult _named(String name, BridgeCutResult Function() edit) {
    var result = BridgeCutResult.nothing;
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project,
        () => result = edit(),
        name: name);
    return result;
  }

  /// A transition on one edge of [clip], or none when [frames] is zero.
  void _transition(LumitUiState ui, CompositionReference comp, BridgeClip clip,
      {required bool endEdge, required int frames}) {
    _afterCut(_cut(() => _named(
        'Transition',
        () => comp.cutTransition(
              clip: clip.id,
              endEdge: endEdge,
              frames: frames,
              linked: _linked,
            ))));
  }

  /// Make the overlap at one edge of [clip] the kind named, or the dissolve
  /// with none, making the overlap first where the edit point only meets. A
  /// kind's rows come up in Effect controls when [show] is set.
  void _setKind(CompositionReference comp, CutTrack track, BridgeClip clip,
      bool endEdge, String? kind, int frames, {bool show = false}) {
    _afterCut(_cut(() => _named(
        'Transition',
        () => comp.cutSetTransition(
              clip: clip.id,
              endEdge: endEdge,
              frames: frames,
              kind: kind,
              linked: _linked,
            ))));
    if (kind != null && show) {
      final other = cutClipAcross(track, clip, endEdge: endEdge);
      _showOverlap(track, endEdge ? other : clip);
    }
  }

  /// Bring the rows of the overlap at [incoming]'s start up in Effect controls.
  void _showOverlap(CutTrack track, BridgeClip? incoming) {
    final ui = cutUi;
    if (ui == null || incoming == null) return;
    ui.cutOverlap.value = (layer: track.entry.layer, clip: incoming.id);
    setPanelVisible(ui.split, Panel.effectControls, true);
    ui.frontPanel(Panel.effectControls);
  }

  /// The Transition rows for one edit point: the dissolve, then every kind, the
  /// one in force ticked.
  List<Widget> _kindRows(
    void Function(void) close,
    String key,
    CompositionReference comp,
    CutTrack track,
    BridgeClip clip,
    bool endEdge,
    int frames,
  ) {
    final other = cutClipAcross(track, clip, endEdge: endEdge);
    final overlaps = cutEdgeOverlaps(track, clip, endEdge: endEdge);
    final incoming = endEdge ? other : clip;
    final now = overlaps ? incoming?.transition : null;
    return [
      _row(close, '$key-dissolve', l10n.cutDissolve,
          () => _setKind(comp, track, clip, endEdge, null, frames),
          action: _settings.cutDefaultTransition == null
              ? 'cut.transition.default'
              : null,
          ticked: overlaps && now == null),
      for (final kind in transitionKinds)
        _row(
            close,
            '$key-${kind.matchName}',
            engineLabel(kind.label),
            () => _setKind(comp, track, clip, endEdge, kind.matchName, frames,
                show: true),
            action: _settings.cutDefaultTransition == kind.matchName
                ? 'cut.transition.default'
                : null,
            ticked: now == kind.matchName),
    ];
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
    // A multicam clip shows its angles rather than opening its comp.
    if (clip.angle != null) {
      ui.showAngles(cutMulticamOf(comp, clip));
      return;
    }
    if (clip.sourceIsComp) {
      _openAsComposition(ui, comp, clip);
      return;
    }
    openClipInSource(ui, comp, track.entry.layer, clip);
  }

  /// The picked clips and their linked sound moved into a new composition, and
  /// one clip left in their place, picked. The cut stays in front, as the point
  /// was to tidy it.
  void _foldPicked(LumitUiState ui, CompositionReference comp) {
    final CompositionReference? made;
    try {
      made = comp.cutClipsToComposition(
        clips: _selectedIds(),
        linked: _linked,
        name: l10n.cutSelectionCompName(comp.getSettings().name),
      );
    } catch (_) {
      return;
    }
    if (made == null) {
      ui.reportCut(BridgeCutResult.locked);
      return;
    }
    cutPicked = ({});
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
    _afterWrite();
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

  /// Mod+K: a cut at the playhead on the picked clips' tracks, or on every
  /// unlocked track when nothing is picked; with Shift always every track,
  /// which an empty list means to the engine.
  void _addEdit(LumitUiState ui, CompositionReference? comp,
      {required bool all}) {
    if (comp == null) return;
    var result = BridgeCutResult.nothing;
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
      result = _cut(() => comp.cutRazor(
            layers: all || cutPicked.isEmpty
                ? const []
                : [for (final track in _candidateTracks()) track.entry.layer],
            atFrame: ui.playheadFrame.value,
            linked: _linked,
          ));
    }, name: all ? 'Split all tracks' : 'Split');
    _afterCut(result);
  }

  /// The picked clips held by the engine for a paste. Says whether it took any.
  bool _copyPicked(LumitUiState ui, CompositionReference comp) {
    final result =
        _cut(() => comp.cutCopy(clips: _selectedIds(), linked: _linked));
    ui.reportCut(result);
    if (result != BridgeCutResult.done) return false;
    cutClipsCopiedIn = Provider.of<LumitState>(context, listen: false).project;
    return true;
  }

  /// Whether a paste would have clips to land.
  bool get _clipsHeld => cutClipsHeld(
      Provider.of<LumitState>(context, listen: false).project);

  /// Copy, or cut, from a menu: the keys' own, without asking whose they are.
  void _copyFromMenu(LumitUiState ui, CompositionReference comp,
      {required bool cut}) {
    if (cutPicked.isEmpty) return;
    if (_copyPicked(ui, comp) && cut) _deleteSelected(ripple: false);
  }

  bool _pasteAtPlayhead(LumitUiState ui, CompositionReference comp) =>
      _pasteAt(ui, comp, ui.playheadFrame.value, onto: cutPasteTarget(ui));

  /// The held clips landed at [frame] on [onto], and the playhead moved to
  /// their end, ready for the next.
  bool _pasteAt(LumitUiState ui, CompositionReference comp, int frame,
      {LayerReference? onto}) {
    if (onto != null && cutIsBusLayer(cutTracks, onto)) {
      cutRefuseOnBus(Provider.of<LumitState>(context, listen: false));
      return true;
    }
    final BridgeCutPaste paste;
    try {
      paste = comp.cutPaste(atFrame: frame, target: onto);
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
    final comp = cutUi?.selectedComp;
    if (!mounted || comp == null || cutPicked.isEmpty) return false;
    final result = _cut(() => comp.cutDelete(
          clips: _selectedIds(),
          ripple: ripple,
          linked: _linked,
        ));
    if (result == BridgeCutResult.done) cutPicked = ({});
    _afterCut(result);
    return true;
  }

  /// The picked clips' ids, as the engine wants them; a clip the document no
  /// longer holds is left out, since an unknown id throws.
  List<UuidValue> _selectedIds() => [
        for (final id in cutPicked)
          if (cutTrackOf[id] case final at?)
            if (cutTracks[at].clip(id) case final clip?) clip.id,
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

  /// A clip's button pressed: its effects in Effect controls, and for
  /// Pan/Crop a Crop and a Transform on the clip first where it has none.
  /// The Crop takes edges off the picture, and the Transform's position,
  /// scale and rotation pan, zoom and turn what is left.
  void _badge(LumitUiState ui, CutTrack track, BridgeClip clip, CutBadge kind) {
    // Pan/Crop opens its own window: the clip's frame with the area shown
    // boxed on it.
    if (kind == CutBadge.panCrop) {
      final comp = ui.selectedComp;
      if (comp == null) return;
      showCutPanCrop(context,
          comp: comp,
          layer: track.entry.layer,
          clip: clip,
          onChanged: () => _afterCut(BridgeCutResult.done));
      return;
    }
    if (!panelVisible(ui.split, Panel.effectControls)) {
      setPanelVisible(ui.split, Panel.effectControls, true);
    }
    activatePanelTab(ui.split, Panel.effectControls);
    ui.workspace.touch();
  }

  /// One row of a menu: shuts the menu, then runs. Every menu here is made of
  /// these, so each row shows its chord the same way.
  Widget _row(
    void Function(void) close,
    String key,
    String label,
    VoidCallback run, {
    String? action,
    bool enabled = true,
    bool? ticked,
  }) =>
      MenuRow(
        key: ValueKey<String>(key),
        enabled: enabled,
        onPressed: () {
          close(null);
          run();
        },
        child:
            MenuLabel(label, action: action, ticked: ticked, enabled: enabled),
      );

  /// A row of the clip menu: [_row] with room for a tick on every row, as the
  /// Audio timeline's clip menu has, so a ticked row keeps the one text edge.
  Widget _cell(
    void Function(void) close,
    String key,
    String label,
    VoidCallback run, {
    String? action,
    bool enabled = true,
    bool? ticked,
  }) =>
      _row(close, key, label, run,
          action: action, enabled: enabled, ticked: ticked ?? false);

  /// A row with more rows out beside it.
  Widget _flyout(
    void Function(void) close,
    String key,
    String label,
    List<Widget> rows, {
    double width = 210,
    bool? ticked = false,
  }) =>
      SubmenuRow(
        key: ValueKey<String>(key),
        closeParent: () => close(null),
        submenu: (_) => FloatSurface(
          width: width,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: rows,
          ),
        ),
        child: MenuLabel(label, ticked: ticked),
      );

  /// An audio effect on every picked clip on a sound track, as one undo step.
  void _addClipAudioEffect(String name) {
    final app = Provider.of<LumitState>(context, listen: false);
    asOneUndoStep(app.project, () {
      for (final id in cutPicked) {
        final at = cutTrackOf[id];
        if (at == null || !cutTracks[at].sound) continue;
        final clip = cutTracks[at].clip(id);
        if (clip == null) continue;
        try {
          cutTracks[at].entry.layer.addClipEffect(clip: clip.id, name: name);
        } catch (_) {
          // A locked track refuses, and the rest still take it.
        }
      }
    });
    app.notifyDocumentChanged();
    _afterWrite();
  }

  /// Every clip on [track] picked.
  void _selectTrack(CutTrack track) => cutPicked = ({
        for (final clip in track.clips) ..._withMates(clip.id.toString()),
      });

  /// Give the picked clips a colour of their own, or hand them back to their
  /// track's with null. One undo step for all of them.
  void _setClipLabel(int? label) {
    final app = Provider.of<LumitState>(context, listen: false);
    asOneUndoStep(app.project, () {
      for (final id in cutPicked) {
        final at = cutTrackOf[id];
        if (at == null) continue;
        final clip = cutTracks[at].clip(id);
        // The colour it already has makes no undo step.
        if (clip == null || clip.label == label) continue;
        try {
          cutTracks[at].entry.layer.setClipLabel(clip: clip.id, label: label);
        } catch (_) {
          // A locked track refuses, and the rest still take it.
        }
      }
    });
    app.notifyDocumentChanged();
    _afterWrite();
  }

  /// A clip's menu, acting on the selection it is in or on the clip alone: its
  /// own commands, the clipboard, then arranging, then how it looks, then its
  /// removal.
  Future<void> openCutClipMenu(LumitUiState ui, CompositionReference comp,
      CutTrack track, BridgeClip clip, double frame, Offset global,
      {List<Widget> Function(void Function(void) close)? extra}) {
    final id = clip.id.toString();
    if (!cutPicked.contains(id)) cutPicked = (_withMates(id));
    // The rows that open or look at one clip, which a wider pick can't mean.
    final one = _withMates(id).containsAll(cutPicked);
    final start = clip.startFrame.toInt();
    final end = clip.endFrame.toInt();
    final playhead = ui.playheadFrame.value;
    // At the playhead, so only while it is over this clip: a split elsewhere
    // would cut some other clip and leave this one whole.
    final inside = start < playhead && playhead < end;
    final picture = !cutHeard(track, clip);
    final second = _secondFrames(ui);
    final heard = track.entry.info.switches.audible;
    // A dissolve on the nearer edit point where a neighbour meets it, and a
    // fade at each end nothing meets, each taken off again where it is there.
    final endEdge = frame - start > end - frame;
    final across = cutClipAcross(track, clip, endEdge: endEdge) != null;
    final dissolved = across && cutEdgeOverlaps(track, clip, endEdge: endEdge);
    final before = cutClipAcross(track, clip, endEdge: false);
    final after = cutClipAfter(track, clip);
    final fadesIn = before == null;
    final fadesOut = cutClipAcross(track, clip, endEdge: true) == null;
    // A through edit, a cut with nothing changed across it: the one at the
    // nearer end where that is one, else the one at the other end.
    final atStart = before != null && cutThroughEdit(before, clip)
        ? (before, clip)
        : null;
    final atEnd =
        after != null && cutThroughEdit(clip, after) ? (clip, after) : null;
    final seam = endEdge ? atEnd ?? atStart : atStart ?? atEnd;
    final joins = seam != null;
    // The audio streams of the clip's file, for the rows that choose among
    // them. The item was probed when it was placed, so this opens nothing.
    BridgeAudioStreams? streams;
    try {
      streams = track.entry.layer.clipAudioStreams(clip: clip.id);
    } catch (_) {}
    final canDetach = track.kind == CutTrackKind.picture &&
        !clip.retimed &&
        !clip.sourceIsComp &&
        !clip.sourceIsSolid;

    // The picked clips that are heard, which the level rows act on.
    final heardPicks = [
      for (final pick in cutPicked)
        if (cutTrackOf[pick] case final at?)
          if (cutTracks[at].clip(pick) case final c?)
            if (cutTracks[at].entry.info.switches.audible && c.audible)
              (cutTracks[at].entry.layer, c),
    ];
    void afterLevel() {
      if (!mounted) return;
      Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
      _afterWrite();
    }

    /// The rows about the clip's sound: in the menu itself on a sound track,
    /// as the Audio timeline has them, and under a heading of their own on a
    /// picture track where there are several.
    List<Widget> soundRows(void Function(void) close) => [
          ...soundClipRows(
            close: () => close(null),
            tickColumn: true,
            keepPitch: !track.entry.info.pitchFollowsSpeed,
            onKeepPitch: track.sound
                ? () {
                    track.entry.layer.setPitchFollowsSpeed(
                        follows: !track.entry.info.pitchFollowsSpeed);
                    _afterCut(BridgeCutResult.done);
                  }
                : null,
            onNormalise: heard && clip.audible
                ? () => normaliseClips(
                    Provider.of<LumitState>(context, listen: false),
                    ui.workspace.normaliseTargetDb,
                    [for (final (layer, c) in heardPicks) (layer, c.id)],
                    then: afterLevel)
                : null,
            onLoudness: heard && clip.audible
                ? (lufs) => normaliseClips(
                    Provider.of<LumitState>(context, listen: false),
                    lufs,
                    [for (final (layer, c) in heardPicks) (layer, c.id)],
                    then: afterLevel,
                    loudness: true)
                : null,
            onTogether: heard && clip.audible && heardPicks.length > 1
                ? (lufs) => normaliseTogether(
                    Provider.of<LumitState>(context, listen: false),
                    lufs ?? ui.workspace.normaliseTargetDb,
                    clips: heardPicks,
                    then: afterLevel,
                    loudness: lufs != null)
                : null,
            resetGainShown: heard && clip.audible,
            onResetGain: heardPicks.any((p) => p.$2.gainDb != 0)
                ? () => resetClipGain(
                    Provider.of<LumitState>(context, listen: false),
                    heardPicks,
                    then: afterLevel)
                : null,
          ),
          // A picture clip's sound, taken off onto a sound track of its own,
          // or brought back from the clip it was taken onto.
          if (canDetach && heard && clip.audible)
            _cell(close, 'ctl-menu-detach-audio', l10n.cutDetachAudio,
                () => _sound(comp, detach: true))
          else if (canDetach && (cutMates[id]?.isNotEmpty ?? false))
            _cell(close, 'ctl-menu-attach-audio', l10n.cutAttachAudio,
                () => _sound(comp, detach: false)),
          // A file with several audio streams: which of them this clip plays,
          // a linked clip to each on a track of its own, and back onto one.
          // The list is for a clip that is heard. A silent picture clip has
          // its sound on the clip linked to it, and the two commands under the
          // list follow the link there.
          if (streams != null && streams.streams.length > 1) ...[
            if (heard && clip.audible)
              audioStreamsSubmenu(
                key: const ValueKey('ctl-menu-audio-streams'),
                closeParent: () => close(null),
                streams: streams,
                onChoose: (places) {
                  // Every picked clip that is heard, as the list is offered.
                  setPickedAudioStreams(
                    Provider.of<LumitState>(context, listen: false).project,
                    [
                      for (final pick in cutPicked)
                        if (cutTrackOf[pick] case final at?)
                          if (cutTracks[at].clip(pick) case final c?)
                            if (cutTracks[at].entry.info.switches.audible &&
                                c.audible)
                              (
                                streams: () => cutTracks[at]
                                    .entry
                                    .layer
                                    .clipAudioStreams(clip: c.id),
                                set: (List<int> places) => cutTracks[at]
                                    .entry
                                    .layer
                                    .setClipAudioStreams(
                                        clip: c.id, streams: places),
                              ),
                    ],
                    places,
                  );
                  if (!mounted) return;
                  Provider.of<LumitState>(context, listen: false)
                      .notifyDocumentChanged();
                  _afterWrite();
                },
              ),
            _cell(close, 'ctl-menu-split-streams', l10n.menuSplitAudioStreams,
                () => _streams(comp, split: true)),
            _cell(close, 'ctl-menu-merge-streams', l10n.menuMergeAudioStreams,
                () => _streams(comp, split: false)),
          ],
        ];

    return showMenuAt<void>(
      context: context,
      position: global,
      width: 230,
      rows: (close) {
        final sound = soundRows(close);
        return [
          // What an editor reaches for all day comes first: what can be done
          // to it along the time.
          _cell(close, 'ctl-menu-split', l10n.cutMenuSplit,
              () => _addEdit(ui, comp, all: false),
              action: 'cut.add.edit', enabled: inside),
          // The cut taken away again, where a neighbour carries straight on.
          if (joins)
            _cell(
                close,
                'ctl-menu-join',
                l10n.cutJoin,
                // One clip joins across that cut, several across the cuts
                // between them.
                () => _afterCut(_cut(() => comp.cutJoin(
                    clips: one ? [seam.$1.id, seam.$2.id] : _selectedIds(),
                    linked: _linked)))),
          _flyout(close, 'ctl-menu-trim', l10n.cutTrimMenu, [
            _cell(
                close,
                'ctl-menu-trim-start',
                l10n.cutTrimStart,
                () => _trimToPlayhead(ui, comp, start: true, ripple: false),
                action: 'cut.trim.start',
                enabled: inside),
            _cell(
                close,
                'ctl-menu-trim-end',
                l10n.cutTrimEnd,
                () => _trimToPlayhead(ui, comp, start: false, ripple: false),
                action: 'cut.trim.end',
                enabled: inside),
            _cell(
                close,
                'ctl-menu-ripple-trim-start',
                l10n.cutRippleTrimStart,
                () => _trimToPlayhead(ui, comp, start: true, ripple: true),
                action: 'cut.trim.start.ripple',
                enabled: inside),
            _cell(
                close,
                'ctl-menu-ripple-trim-end',
                l10n.cutRippleTrimEnd,
                () => _trimToPlayhead(ui, comp, start: false, ripple: true),
                action: 'cut.trim.end.ripple',
                enabled: inside),
          ], width: 250),
          if (across || fadesIn || fadesOut)
            _flyout(close, 'ctl-menu-transition', l10n.cutTransitionMenu, [
              // A sound track only crossfades. A picture track has every kind,
              // and the way back to a plain cut once there is an overlap.
              if (across && track.kind != CutTrackKind.picture)
                _cell(
                    close,
                    'ctl-menu-dissolve',
                    dissolved ? l10n.cutRemoveDissolve : l10n.cutAddDissolve,
                    () => _transition(ui, comp, clip,
                        endEdge: endEdge, frames: dissolved ? 0 : second),
                    action: 'cut.transition.default'),
              if (across && track.kind == CutTrackKind.picture) ...[
                ..._kindRows(close, 'ctl-menu-kind', comp, track, clip, endEdge,
                    second),
                if (dissolved)
                  _cell(
                      close,
                      'ctl-menu-transition-remove',
                      l10n.cutRemoveTransition,
                      () => _transition(ui, comp, clip,
                          endEdge: endEdge, frames: 0),
                      ticked: false),
                if (fadesIn || fadesOut) const MenuDivider(),
              ],
              if (fadesIn)
                _cell(
                    close,
                    'ctl-menu-fade-in',
                    clip.fadeIn.seconds > 0
                        ? l10n.cutRemoveFadeIn
                        : l10n.cutFadeIn,
                    () => _transitionPicked(ui, comp,
                        endEdge: false,
                        frames: clip.fadeIn.seconds > 0 ? 0 : second),
                    ticked: false),
              if (fadesOut)
                _cell(
                    close,
                    'ctl-menu-fade-out',
                    clip.fadeOut.seconds > 0
                        ? l10n.cutRemoveFadeOut
                        : l10n.cutFadeOut,
                    () => _transitionPicked(ui, comp,
                        endEdge: true,
                        frames: clip.fadeOut.seconds > 0 ? 0 : second),
                    ticked: false),
            ]),
          // The frame under the playhead held, for a length it asks.
          _cell(close, 'ctl-menu-freeze', l10n.cutFreezeFrame, () {
            askToFreezeClips(
                context, [(layer: track.entry.layer, clip: clip)], global);
          }, enabled: playhead >= start && playhead < end),
          // The picked picture clips' speed: one number for all of them, back
          // to normal, or one clip's line carried onto the rest.
          if (track.kind == CutTrackKind.picture)
            _flyout(close, 'ctl-menu-speed', l10n.cutSpeedMenu, [
              // The line on this clip alone, with the switch off. A retimed
              // clip always shows it.
              if (cutSpeedShown case final shown?)
                _cell(close, 'ctl-menu-speed-line', l10n.cutSpeedLineHere, () {
                  setState(() {
                    if (!shown.remove(id)) shown.add(id);
                  });
                },
                    enabled: !_speedLines && !clip.retimed,
                    ticked: _speedLines || clip.retimed || shown.contains(id)),
              _cell(close, 'ctl-menu-speed-set', l10n.menuClipSpeed,
                  _askSpeed,
                  ticked: false),
              _cell(close, 'ctl-menu-speed-reset', l10n.clipResetSpeed,
                  () => _setSpeed(100),
                  enabled: _pickedPicture().any((p) => p.$2.retimed),
                  ticked: false),
              const MenuDivider(),
              _cell(close, 'ctl-menu-speed-copy', l10n.cutCopySpeed,
                  () => speedLineClipboard = keysOf(clip.retime),
                  ticked: false),
              _cell(close, 'ctl-menu-speed-paste', l10n.cutPasteSpeed,
                  () => _pasteSpeed(ui),
                  enabled: speedLineClipboard != null, ticked: false),
            ]),
          if (track.sound) ...[
            ...sound,
            SubmenuRow(
              key: const ValueKey('ctl-menu-add-audio-effect'),
              closeParent: () => close(null),
              submenu: (dismiss) =>
                  audioEffectFlyout(dismiss, _addClipAudioEffect),
              child: MenuLabel(l10n.audioAddEffect, ticked: false),
            ),
          ] else if (sound.length == 1)
            sound.single
          else if (sound.isNotEmpty)
            _flyout(close, 'ctl-menu-sound', l10n.cutSoundMenu, sound,
                width: 240),
          // A picture clip's own blend mode, or its track's.
          if (track.kind == CutTrackKind.picture)
            SubmenuRow(
              key: const ValueKey('ctl-menu-blend'),
              closeParent: () => close(null),
              submenu: (_) => FloatSurface(
                width: 190,
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxHeight: 380),
                  child: SingleChildScrollView(
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        for (var i = -1; i < blendModeNames().length; i++)
                          MenuRow.option(
                            key: ValueKey<String>('ctl-menu-blend-$i'),
                            onPressed: () {
                              close(null);
                              _setBlend(i < 0 ? null : i);
                            },
                            child: MenuLabel(
                              i < 0
                                  ? l10n.clipBlendFromLayer
                                  : engineLabel(blendModeNames()[i]),
                              ticked: clip.blend == (i < 0 ? null : i),
                            ),
                          ),
                      ],
                    ),
                  ),
                ),
              ),
              child: MenuLabel(l10n.tipBlendMode, ticked: false),
            ),
          // The clip's takes, the one showing ticked: picked to show it, or
          // deleted. Takes come from footage dropped on the clip with Alt.
          if (clip.takes.length > 1)
            _flyout(close, 'ctl-menu-takes', l10n.cutTakeMenu, [
              for (var n = 0; n < clip.takes.length; n++)
                _cell(close, 'ctl-menu-take-$n', clip.takes[n], () {
                  if (n == 0) return;
                  _afterCut(
                      _cut(() => comp.cutSetTake(clip: clip.id, take: n)));
                }, ticked: n == 0),
              const MenuDivider(),
              _cell(
                  close,
                  'ctl-menu-take-delete',
                  l10n.cutDeleteTake,
                  () => _afterCut(
                      _cut(() => comp.cutDeleteTake(clip: clip.id, take: 0))),
                  ticked: false),
            ], width: 240),
          if (clip.angle != null) ...[
            const MenuDivider(),
            ..._multicamRows(close, ui, comp, clip),
          ],
          const MenuDivider(),
          // What opens or looks at the clip. A solid has nothing inside it to
          // open, and making a composition of a clip changes the document, so
          // it stands last of these.
          if (ui.currentViewerPane != null &&
              !clip.sourceIsComp &&
              !clip.sourceIsSolid)
            _cell(close, 'ctl-menu-open-source', l10n.cutOpenInSource,
                () => _openClip(ui, comp, track, clip),
                enabled: one),
          _cell(close, 'ctl-menu-effects', l10n.effectControls,
              () => _badge(ui, track, clip, CutBadge.fx),
              enabled: one),
          if (picture)
            _cell(close, 'ctl-menu-pan-crop', l10n.cutPanCropMenu,
                () => _badge(ui, track, clip, CutBadge.panCrop),
                enabled: one),
          if (!one)
            _cell(close, 'ctl-menu-fold', l10n.cutNewCompositionFromSelection,
                () => _foldPicked(ui, comp))
          // A multicam clip's comp is opened from its own rows.
          else if (!clip.sourceIsSolid && clip.angle == null)
            _cell(
                close,
                'ctl-menu-open-comp',
                clip.sourceIsComp
                    ? l10n.cutOpenComposition
                    : l10n.cutNewCompositionFromClip,
                () => _openAsComposition(ui, comp, clip)),
          const MenuDivider(),
          _cell(close, 'ctl-menu-cut', l10n.menuCut,
              () => _copyFromMenu(ui, comp, cut: true),
              action: 'edit.cut'),
          _cell(close, 'ctl-menu-copy', l10n.menuCopy,
              () => _copyFromMenu(ui, comp, cut: false),
              action: 'edit.copy'),
          _cell(close, 'ctl-menu-paste', l10n.menuPaste,
              () => _pasteAtPlayhead(ui, comp),
              action: 'edit.paste', enabled: _clipsHeld),
          _cell(close, 'ctl-menu-duplicate', l10n.menuDuplicate,
              () => _duplicatePicked(comp),
              action: 'cut.duplicate'),
          // The copied clip's effects, levels, fades and speed onto the picked
          // clips.
          _cell(
              close,
              'ctl-menu-paste-attributes',
              l10n.cutPasteAttributes,
              () => _afterCut(
                  _cut(() => comp.cutPasteAttributes(clips: _selectedIds()))),
              action: 'cut.paste.attributes',
              enabled: _clipsHeld),
          const MenuDivider(),
          if (extra != null) ...[...extra(close), const MenuDivider()],
          if (cutPicksMany)
            _cell(close, 'ctl-menu-group', l10n.cutGroup,
                () => _group(join: true),
                action: 'cut.group', enabled: cutPicked.length > 1),
          if (clip.group != null)
            _cell(close, 'ctl-menu-ungroup', l10n.cutUngroup,
                () => _group(join: false),
                action: 'cut.ungroup'),
          // This clip and everything after it, on the picked clips' tracks.
          if (cutPicksMany) ...[
            _cell(close, 'ctl-menu-select-to-end', l10n.cutSelectToEnd, () {
              cutPicked = {
                for (final row in _candidateTracks())
                  for (final c in row.clips)
                    if (c.startFrame >= clip.startFrame)
                      ..._withMates(c.id.toString()),
              };
            }),
            _cell(close, 'ctl-menu-select-track', l10n.cutSelectTrack,
                () => _selectTrack(track)),
          ],
          const MenuDivider(),
          SubmenuRow(
            key: const ValueKey('ctl-menu-clip-colour'),
            closeParent: () => close(null),
            submenu: (dismiss) => clipColourMenu(
              ThemeScope.of(context).theme,
              keyPrefix: 'ctl-menu-clip-colour',
              onPick: (label) {
                dismiss();
                _setClipLabel(label);
              },
            ),
            child: MenuLabel(l10n.clipColour, ticked: false),
          ),
          if (clip.effects.isNotEmpty)
            _cell(
                close,
                'ctl-menu-bypass-fx',
                clip.fx ? l10n.cutBypassEffects : l10n.cutUseEffects, () {
              try {
                track.entry.layer.setClipFx(clip: clip.id, on_: !clip.fx);
              } catch (_) {}
              Provider.of<LumitState>(context, listen: false)
                  .notifyDocumentChanged();
              _afterWrite();
            }),
          const MenuDivider(),
          _cell(close, 'ctl-menu-delete', l10n.clipDelete,
              () => _deleteSelected(ripple: false),
              action: 'edit.delete.selection'),
          _cell(close, 'ctl-menu-ripple-delete', l10n.cutRippleDelete,
              () => _deleteSelected(ripple: true),
              action: 'cut.delete.ripple'),
        ];
      },
    );
  }

  /// A fade on one end of every picked clip, or none when [frames] is zero.
  /// One undo step.
  void _transitionPicked(LumitUiState ui, CompositionReference comp,
      {required bool endEdge, required int frames}) {
    var refused = BridgeCutResult.done;
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
      for (final id in cutPicked) {
        final at = cutTrackOf[id];
        final clip = at == null ? null : cutTracks[at].clip(id);
        if (at == null || clip == null) continue;
        // A clip the picked clip's link reached with a neighbour at that end
        // has a dissolve to keep, and no fade to give.
        if (cutClipAcross(cutTracks[at], clip, endEdge: endEdge) != null) {
          continue;
        }
        final result = _cut(() => comp.cutTransition(
            clip: clip.id, endEdge: endEdge, frames: frames, linked: false));
        if (result != BridgeCutResult.done) refused = result;
      }
    }, name: 'Transition');
    _afterCut(BridgeCutResult.done);
    ui.reportCut(refused);
  }

  /// A copy of the picked clips straight after them, or at the far end of
  /// their tracks where something is in the way, since a clip that lands on
  /// another overwrites it.
  void _duplicatePicked(CompositionReference comp) {
    final picked = [
      for (final id in cutPicked)
        if (cutTrackOf[id] case final at?)
          if (cutTracks[at].clip(id) case final clip?) (at, clip),
    ];
    if (picked.isEmpty) return;
    _afterCut(_cut(() => comp.cutDuplicate(
          clips: [for (final p in picked) p.$2.id],
          grabbed: picked.first.$2.id,
          byFrames: cutDuplicateFrames(
            [for (final p in picked) p.$2],
            [for (final at in {for (final p in picked) p.$1}) cutTracks[at].clips],
          ),
          target: null,
          linked: _linked,
          how: _how(copy: true),
        )));
  }

  /// The picked clips into one new group, or out of the groups they are in.
  /// One undo step.
  void _group({required bool join}) {
    final picked = [
      for (final id in cutPicked)
        if (cutTrackOf[id] case final at?)
          if (cutTracks[at].clip(id) case final clip?) (cutTracks[at], clip),
    ];
    if (picked.isEmpty || (join && picked.length < 2)) return;
    final group = join ? UuidValue.fromString(const Uuid().v4()) : null;
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
      for (final (track, clip) in picked) {
        try {
          track.entry.layer.setClipGroup(clip: clip.id, group: group);
        } catch (_) {}
      }
    });
    _afterCut(BridgeCutResult.done);
  }

  /// The picked clips on picture tracks, which are the ones with a speed.
  List<(CutTrack, BridgeClip)> _pickedPicture() => [
        for (final id in cutPicked)
          if (cutTrackOf[id] case final at?)
            if (cutTracks[at].kind == CutTrackKind.picture)
              if (cutTracks[at].clip(id) case final clip?) (cutTracks[at], clip),
      ];

  /// Ask one speed for the picked picture clips.
  Future<void> _askSpeed() async {
    final percent =
        await showStretchDialogFrb(context: context, durationFrames: null);
    if (percent != null && mounted) _setSpeed(percent);
  }

  /// Every picked picture clip at one constant speed, in one undo step.
  void _setSpeed(double percent) {
    final picked = _pickedPicture();
    if (picked.isEmpty) return;
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
      for (final (track, clip) in picked) {
        try {
          track.entry.layer.setClipSpeed(
              clip: clip.id, percent: percent, endPercent: percent);
        } catch (_) {}
      }
    });
    _afterCut(BridgeCutResult.done);
  }

  /// The copied speed line onto every picked picture clip, stretched to
  /// each one's length, so a ramp cut to one beat can be given to the rest.
  void _pasteSpeed(LumitUiState ui) {
    final copied = speedLineClipboard;
    final picked = _pickedPicture();
    if (copied == null || picked.isEmpty) return;
    final fps = ui.model.fps;
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    asOneUndoStep(Provider.of<LumitState>(context, listen: false).project, () {
      for (final (track, clip) in picked) {
        final own = keysOf(clip.retime);
        if (own.isEmpty || fps <= 0) continue;
        final keys = envelopeStretchedOnto(
          copied,
          (clip.endFrame - clip.startFrame).toDouble() / fps,
          own.first.value,
          (seconds) => timeOfSubframe(seconds * fps, fpsNum, fpsDen),
        );
        if (keys.isEmpty) continue;
        try {
          track.entry.layer.setClipRetime(
              clip: clip.id, value: BridgeScalar.keyframed(keys));
        } catch (_) {}
      }
    });
    _afterCut(BridgeCutResult.done);
  }

  /// Give the picked picture clips a blend mode of their own, by its place
  /// in the engine's list, or hand them back to their track's with null. One
  /// undo step for all of them.
  void _setBlend(int? index) {
    final app = Provider.of<LumitState>(context, listen: false);
    asOneUndoStep(app.project, () {
      for (final id in cutPicked) {
        final at = cutTrackOf[id];
        if (at == null || cutTracks[at].kind != CutTrackKind.picture) continue;
        final clip = cutTracks[at].clip(id);
        // The mode it already has makes no undo step.
        if (clip == null || clip.blend == index) continue;
        try {
          cutTracks[at].entry.layer.setClipBlend(clip: clip.id, index: index);
        } catch (_) {
          // A locked track refuses, and the rest still take it.
        }
      }
    });
    app.notifyDocumentChanged();
    _afterWrite();
  }

  /// Split or merge the audio streams of the selected clips. The engine
  /// follows each clip's link, so asking it of a picture clip reaches the
  /// clip that carries its sound.
  void _streams(CompositionReference comp, {required bool split}) {
    final clips = _selectedIds();
    final result = _cut(() => split
        ? comp.cutSplitAudioStreams(clips: clips)
        : comp.cutMergeAudioStreams(clips: clips));
    if (!mounted) return;
    if (result == BridgeCutResult.nothing) {
      // The Cut panel's own word for "nothing there" is about clips and
      // gaps. This one is about streams.
      Provider.of<LumitState>(context, listen: false).postNotice(split
          ? l10n.audioStreamsNothingToSplit
          : l10n.audioStreamsNothingToMerge);
      return;
    }
    _afterCut(result);
  }

  /// Detach or attach the sound of the selected picture clips. The engine
  /// passes over the clips each cannot be asked of, the sound clips among
  /// them say.
  void _sound(CompositionReference comp, {required bool detach}) {
    final clips = _selectedIds();
    final result = _cut(() => detach
        ? comp.cutDetachAudio(clips: clips)
        : comp.cutAttachAudio(clips: clips));
    if (!mounted) return;
    if (result == BridgeCutResult.nothing) {
      Provider.of<LumitState>(context, listen: false).postNotice(
          detach ? l10n.cutNoSoundToDetach : l10n.cutNoSoundToAttach);
      return;
    }
    _afterCut(result);
  }
}
