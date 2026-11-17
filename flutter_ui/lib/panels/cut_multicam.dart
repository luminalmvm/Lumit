part of 'cut_timeline_panel_frb.dart';

// The Cut timeline's multicam rows: the angle menus of a multicam clip, and
// the Sync to angle rows a track has inside a multicam comp. The engine makes
// every edit; these only name the angle and the clips.

/// The multicam comp [clip] plays an angle of, as a reference.
CompositionReference cutMulticamOf(
        CompositionReference comp, BridgeClip clip) =>
    CompositionReference(
        internalproject: comp.internalproject, internalid: clip.source);

extension CutMulticamMenu<W extends StatefulWidget> on CutClipMenu<W> {
  /// A multicam clip's own rows: its picture angle, its sound angle, its
  /// angles in the source side, its comp, and Flatten.
  List<Widget> _multicamRows(void Function(void) close, LumitUiState ui,
      CompositionReference comp, BridgeClip clip) {
    final mc = cutMulticamOf(comp, clip);
    BridgeMulticam? info;
    try {
      info = mc.multicam();
    } catch (_) {}
    if (info == null) return const [];
    final angles = info.angles;
    final id = clip.id.toString();
    // The clip's halves, top row first: the one drawn as picture, and the
    // sound halves under it.
    final halves = [
      for (final each in {id, ...?cutMates[id]})
        if (cutTrackOf[each] case final at?)
          if (cutTracks[at].clip(each) case final c?)
            if (c.angle != null && c.source == clip.source) (at, c),
    ]..sort((a, b) => a.$1.compareTo(b.$1));
    (int, BridgeClip)? pictureHalf;
    final soundHalves = <(int, BridgeClip)>[];
    for (final half in halves) {
      final (at, c) = half;
      if (cutHeard(cutTracks[at], c)) {
        soundHalves.add(half);
      } else {
        pictureHalf ??= half;
      }
    }
    // The top sounding half is what Sound angle switches and ticks: the
    // picture clip while it is heard, else the topmost sound half. Any sound
    // half under that is an angle heard as well.
    final pictureHeard = pictureHalf != null &&
        cutTracks[pictureHalf.$1].entry.info.switches.audible &&
        pictureHalf.$2.audible;
    final top = pictureHeard ? pictureHalf : soundHalves.firstOrNull;
    final picture = pictureHalf?.$2.angle?.id;
    final sound = top?.$2.angle?.id;
    final alsoHeard = {
      for (final half in soundHalves)
        if (half != top) half.$2.angle!.id,
    };
    void switchTo(UuidValue angle, BridgeAngleSwitch mode) {
      ui.multicam.switchTo(angle, clips: _selectedIds(), mode: mode);
      _afterWrite();
    }

    final t = ThemeScope.of(context).theme;
    // The number before the name, in the chord's own style. An angle
    // switched off in its comp has none.
    Widget angleRow(String key, BridgeAngle a, bool on, VoidCallback run) =>
        MenuRow(
          key: ValueKey<String>('$key-${a.id}'),
          onPressed: () {
            close(null);
            run();
          },
          child: Row(children: [
            menuTick(on),
            SizedBox(
                width: 20,
                child: Text(a.number > 0 ? '${a.number}' : '',
                    style: t.small.copyWith(color: t.textMuted))),
            Expanded(
                child: Text(a.name,
                    maxLines: 1, overflow: TextOverflow.ellipsis)),
          ]),
        );
    return [
      if (angles.any((a) => !a.sound))
        _flyout(close, 'ctl-menu-angle', l10n.cutAngleMenu, [
          for (final a in angles)
            if (!a.sound)
              angleRow('ctl-menu-angle', a, a.id == picture,
                  () => switchTo(a.id, BridgeAngleSwitch.picture)),
        ]),
      _flyout(close, 'ctl-menu-sound-angle', l10n.cutSoundAngleMenu, [
        for (final a in angles)
          angleRow('ctl-menu-sound-angle', a, a.id == sound,
              () => switchTo(a.id, BridgeAngleSwitch.sound)),
      ]),
      // Another angle heard under the clip as well, or taken away again.
      _flyout(close, 'ctl-menu-add-sound', l10n.cutAddSoundMenu, [
        for (final a in angles)
          if (a.id != sound)
            angleRow(
                'ctl-menu-add-sound',
                a,
                alsoHeard.contains(a.id),
                () => _afterCut(_cut(() => comp.cutHearAngle(
                    clip: clip.id,
                    angle: a.id,
                    on_: !alsoHeard.contains(a.id))))),
      ]),
      _cell(close, 'ctl-menu-show-angles', l10n.cutShowAngles,
          () => ui.showAngles(mc)),
      _cell(close, 'ctl-menu-open-multicam', l10n.multicamOpen,
          () => _openAsComposition(ui, comp, clip)),
      _cell(close, 'ctl-menu-flatten', l10n.cutFlatten, () {
        ui.multicam.flatten(_selectedIds());
        _afterWrite();
      }),
    ];
  }

  /// Inside a multicam comp, a Sequence track's Sync to angle: the other
  /// angles by name, each lining this track's files up against it by their
  /// sound. Nothing in any other comp.
  List<Widget> _syncAngleRows(void Function(void) close, LumitUiState ui,
      CompositionReference comp, CutTrack track) {
    if (track.entry.info.kind != BridgeLayerKind.sequence) return const [];
    BridgeMulticam? info;
    try {
      info = comp.multicam();
    } catch (_) {}
    final layer = track.entry.layer.internallayerId;
    final others = [
      for (final a in info?.angles ?? const <BridgeAngle>[])
        if (a.id != layer) a,
    ];
    if (others.isEmpty) return const [];
    return [
      _flyout(close, 'ctl-head-sync-angle', l10n.multicamSyncToAngle, [
        for (final a in others)
          _cell(close, 'ctl-head-sync-angle-${a.id}', a.name,
              () => ui.multicam.syncAngle(comp, layer, a.id)),
      ]),
    ];
  }
}
