// The library of saved graphics templates, as the panels share it: the
// templates in the project that the library has a newer version of, and which
// of the two timelines a template placed at the playhead goes into.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/state.dart' show ScopedChange;
import 'package:lumit_flutter/src/rust/api/template/library.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import 'dock.dart';

/// A template's key in the workspace's favourites, beside the effects' and
/// the presets'.
String templateFavouriteKey(BridgeLibraryTemplate template) =>
    'template:${template.id}';

/// The templates in the open project that the library has a newer version
/// of, read again when the project's items change (a template's mark is one
/// of them), when the library changes and when another project opens, and
/// kept between. Panels ask this and never the engine from a build.
class TemplateUpdates extends ChangeNotifier {
  TemplateUpdates._();

  static final TemplateUpdates _instance = TemplateUpdates._();

  /// The one list, following [app] and [ui].
  static TemplateUpdates of(LumitState app, LumitUiState ui) =>
      _instance.._bind(app, ui);

  LumitState? _app;
  LumitUiState? _ui;
  StreamSubscription<ScopedChange>? _changes;
  bool _soon = false;
  ProjectReference? _project;
  List<BridgeTemplateUpdate> _all = const [];
  Panel? _lastTimeline;

  void _bind(LumitState app, LumitUiState ui) {
    if (identical(_app, app) && identical(_ui, ui)) return;
    _changes?.cancel();
    try {
      _app?.removeListener(_appChanged);
      _ui?.presetsChanged.removeListener(read);
      _ui?.activePane.removeListener(_paneChanged);
    } catch (_) {
      // The state followed before has been let go.
    }
    _app = app;
    _ui = ui;
    _lastTimeline = null;
    _changes = app.onChange.listen((change) {
      if (change.items) _readSoon();
    });
    app.addListener(_appChanged);
    ui.presetsChanged.addListener(read);
    ui.activePane.addListener(_paneChanged);
    read();
  }

  /// Another project open is read at once.
  void _appChanged() {
    if (_app?.project != _project) read();
  }

  /// Read once for a run of changes that arrive together. Reading opens
  /// every file in the library.
  void _readSoon() {
    if (_soon) return;
    _soon = true;
    scheduleMicrotask(read);
  }

  /// Read the list again now.
  void read() {
    _soon = false;
    final project = _app?.project;
    _project = project;
    List<BridgeTemplateUpdate> next;
    try {
      next = project?.templateUpdates() ?? const [];
    } catch (_) {
      next = const [];
    }
    if (listEquals(next, _all)) return;
    _all = next;
    notifyListeners();
  }

  /// Every template the library has a newer version of.
  List<BridgeTemplateUpdate> get all => _all;

  /// The newer version of the template whose composition is [comp], if the
  /// library has one.
  BridgeTemplateUpdate? forComp(UuidValue comp) {
    for (final update in _all) {
      if (update.comp == comp) return update;
    }
    return null;
  }

  /// Put the library's newest version in place of each of [comps], one after
  /// another. Each is one undo step.
  Future<void> update(Iterable<UuidValue> comps) async {
    final app = _app;
    final project = app?.project;
    if (app == null || project == null) return;
    for (final comp in comps.toList()) {
      try {
        await project.updateTemplate(comp: comp, folder: l10n.templatesFolder);
      } catch (_) {
        app.postNotice(l10n.templateUpdateFailed, error: true);
        break;
      }
    }
    app.notifyDocumentChanged();
    _ui?.model.refresh();
    read();
  }

  void _paneChanged() {
    final panel = _ui?.activePanel;
    if (panel == Panel.timeline || panel == Panel.cutTimeline) {
      _lastTimeline = panel;
    }
  }

  /// Which of the Timeline and the Cut timeline had the keys last: where a
  /// template placed at the playhead goes. Before either has, the Timeline
  /// when it is in the layout, else the Cut timeline.
  Panel get lastTimeline {
    if (_lastTimeline case final panel?) return panel;
    final split = _ui?.split;
    if (split == null ||
        panelVisible(split, Panel.timeline) ||
        !panelVisible(split, Panel.cutTimeline)) {
      return Panel.timeline;
    }
    return Panel.cutTimeline;
  }
}
