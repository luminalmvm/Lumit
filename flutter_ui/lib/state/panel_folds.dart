// What is twirled open or shut in one project's Timeline and Effect Controls.
// Held here rather than in the panels so it outlives them and is saved with
// the rest of the project's view.

/// The folds of one project, by the paths the panels already key them by.
///
/// The sets are handed to the panels as they are and filled in place on a
/// restore, so a panel holding one never has to be told it changed.
class PanelFolds {
  /// Timeline twirls that are open: layer ids and the paths under them.
  final Set<String> timelineOpen = {};

  /// Timeline layer groups folded shut, by group id.
  final Set<String> foldedGroups = {};

  /// Timeline group headers with their effect lanes open, by group id.
  final Set<String> groupFxOpen = {};

  /// Timeline layers the last `U` opened, so a later `U` still shuts them once
  /// the panel has been built again. Not saved.
  final Set<String> revealCycle = {};

  /// Effect Controls sections twirled shut, by path.
  final Set<String> effectsShut = {};

  /// Effect Controls parameter groups the user has opened or shut, by path.
  /// Absent means the schema's own default.
  final Map<String, bool> paramGroupsOpen = {};

  Map<String, dynamic> toJson() => {
        'timeline_open': timelineOpen.toList(),
        'folded_groups': foldedGroups.toList(),
        'group_fx_open': groupFxOpen.toList(),
        'effects_shut': effectsShut.toList(),
        'param_groups_open': Map.of(paramGroupsOpen),
      };

  void clear() {
    timelineOpen.clear();
    foldedGroups.clear();
    groupFxOpen.clear();
    revealCycle.clear();
    effectsShut.clear();
    paramGroupsOpen.clear();
  }

  /// Put back what [raw] holds. Anything not the shape [toJson] writes is
  /// skipped, so a project from another build still opens.
  void restore(Object? raw) {
    clear();
    if (raw is! Map) return;
    void fill(Set<String> into, Object? list) {
      if (list is List) into.addAll(list.whereType<String>());
    }

    fill(timelineOpen, raw['timeline_open']);
    fill(foldedGroups, raw['folded_groups']);
    fill(groupFxOpen, raw['group_fx_open']);
    fill(effectsShut, raw['effects_shut']);
    final groups = raw['param_groups_open'];
    if (groups is Map) {
      for (final e in groups.entries) {
        if (e.key is String && e.value is bool) {
          paramGroupsOpen[e.key as String] = e.value as bool;
        }
      }
    }
  }
}
