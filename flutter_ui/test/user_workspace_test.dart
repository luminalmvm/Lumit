// The user's own workspaces (docs/07 §1.4, item 7.19): saving the arrangement
// on screen under a name, renaming it, deleting it, sending it to somebody, and
// reaching it from the strip or from `Alt+Shift+1…9`.
//
// The store is a folder of files beside the settings, so every test here writes
// into a scratch folder of its own and reads it back with a second `Workspace` —
// which is the round trip that matters: what a user saves today has to still be
// there after a restart.

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/workspace.dart';

/// A settings file of this test's own: saving a workspace writes beside it, and
/// the real store is machine state a test must not reach.
String _scratchStore(String name) =>
    '${Directory.systemTemp.path}${Platform.pathSeparator}'
    'lumit-test-$name${Platform.pathSeparator}workspace.json';

/// A workspace writing somewhere harmless, torn down after the test. The
/// scratch folder is emptied first, so a previous run's files are not read as
/// this run's.
Workspace _workspace(String name) {
  Workspace.storeOverride = _scratchStore(name);
  addTearDown(() => Workspace.storeOverride = null);
  final dir = Workspace.userWorkspaceDir();
  if (dir.existsSync()) dir.deleteSync(recursive: true);
  return Workspace();
}

/// The same store read afresh — what the next launch sees.
Workspace _reopened() => Workspace()..loadUserWorkspaces();

void main() {
  group('saving one of the user\'s own', () {
    test('a saved workspace survives a restart', () {
      final w = _workspace('user-workspace-roundtrip');
      w.applyWorkspacePreset(WorkspacePreset.colour);
      final name = w.saveWorkspaceAs('Grading');

      expect(name, 'Grading');
      expect(w.activeUserWorkspace, 'Grading',
          reason: 'saving switches to what was just saved');
      expect(w.activePreset, isNull,
          reason: 'the strip ticks the new name, not the preset it came from');

      final next = _reopened();
      expect(next.userWorkspaces.map((s) => s.name), ['Grading']);
      next.applyUserWorkspace('Grading');
      expect(
          panelsIn(next.dock), panelsIn(presetLayout(WorkspacePreset.colour)),
          reason: 'the arrangement is what was saved, not the default');
    });

    /// The name is the identity — the strip shows it and the store files by
    /// it — so a second save under a taken name is numbered rather than
    /// silently overwriting somebody's arrangement.
    test('a taken name is numbered rather than overwritten', () {
      final w = _workspace('user-workspace-taken');
      w.saveWorkspaceAs('Mine');
      w.applyWorkspacePreset(WorkspacePreset.audio);
      final second = w.saveWorkspaceAs('Mine');

      expect(second, 'Mine 2');
      expect(w.userWorkspaces.length, 2);
      expect(_reopened().userWorkspaces.map((s) => s.name), ['Mine', 'Mine 2']);
    });

    /// Layout changes persist automatically to the active workspace (§1.4):
    /// a workspace edited by dragging keeps what it was dragged into.
    test('dragging the panels writes back to the workspace in force', () {
      final w = _workspace('user-workspace-drag');
      w.saveWorkspaceAs('Mine');
      setPanelVisible(w.dock, Panel.scopes, false);
      w.rememberActiveWorkspaceLayout();

      final next = _reopened()..applyUserWorkspace('Mine');
      expect(panelVisible(next.dock, Panel.scopes), isFalse);
    });

    /// A preset's factory layout is not the user's to overwrite: a drag under
    /// a preset changes the arrangement and nothing else.
    test('dragging under a preset writes to no user workspace', () {
      final w = _workspace('user-workspace-preset-drag');
      w.saveWorkspaceAs('Mine');
      w.applyWorkspacePreset(WorkspacePreset.edit);
      setPanelVisible(w.dock, Panel.scopes, false);
      w.rememberActiveWorkspaceLayout();

      final next = _reopened()..applyUserWorkspace('Mine');
      expect(panelVisible(next.dock, Panel.scopes), isTrue,
          reason: 'the saved workspace is untouched by a drag made under Edit');
    });
  });

  group('renaming and deleting', () {
    test('a rename survives a restart and leaves no old file', () {
      final w = _workspace('user-workspace-rename');
      w.saveWorkspaceAs('Grading');
      expect(w.renameUserWorkspace('Grading', 'Colour'), 'Colour');
      expect(w.activeUserWorkspace, 'Colour',
          reason: 'the selection follows the name');

      expect(_reopened().userWorkspaces.map((s) => s.name), ['Colour'],
          reason: 'the file it was under is gone, not left behind as a second');
    });

    test('a rename onto a taken name is numbered', () {
      final w = _workspace('user-workspace-rename-taken');
      w.saveWorkspaceAs('One');
      w.saveWorkspaceAs('Two');
      expect(w.renameUserWorkspace('Two', 'One'), 'One 2');
      expect(_reopened().userWorkspaces.map((s) => s.name), ['One', 'One 2']);
    });

    test('a deleted workspace is gone from the store', () {
      final w = _workspace('user-workspace-delete');
      w.saveWorkspaceAs('Grading');
      w.deleteUserWorkspace('Grading');

      expect(w.userWorkspaces, isEmpty);
      expect(w.activeUserWorkspace, isNull);
      expect(_reopened().userWorkspaces, isEmpty);
    });
  });

  group('the file a workspace travels in', () {
    test('a workspace survives being written and read', () {
      final w = _workspace('user-workspace-file');
      w.applyWorkspacePreset(WorkspacePreset.nodes);
      w.saveWorkspaceAs('Sent');
      final saved = w.userWorkspaces.single;

      final read = UserWorkspace.fromJson(jsonDecode(saved.encode()));
      expect(read, isNotNull);
      expect(read!.name, 'Sent');
      final tree = DockNode.fromJson(read.dock);
      expect(tree, isA<DockSplit>());
      expect(panelsIn(tree as DockSplit),
          panelsIn(presetLayout(WorkspacePreset.nodes)));
    });

    /// Picking the wrong file is a normal thing to do, so it comes back as a
    /// refusal rather than as an exception or a workspace that does nothing.
    test('anything that is not a workspace is refused', () {
      expect(UserWorkspace.fromJson('not json at all'), isNull);
      expect(UserWorkspace.fromJson({'name': 'No tree'}), isNull);
      expect(UserWorkspace.fromJson({'name': 'Bad tree', 'dock': {}}), isNull);
      expect(
          UserWorkspace.fromJson({
            'format': 'lumit-theme',
            'name': 'Wrong sort',
            'dock': defaultLayout().toJson(),
          }),
          isNull,
          reason: 'a theme renamed .lumworkspace is not a workspace');
    });

    test('an import never overwrites one of the user\'s own', () {
      final w = _workspace('user-workspace-import');
      w.saveWorkspaceAs('Mine');
      final landed = w
          .importUserWorkspace(UserWorkspace('Mine', defaultLayout().toJson()));

      expect(landed, 'Mine 2');
      expect(_reopened().userWorkspaces.map((s) => s.name), ['Mine', 'Mine 2']);
    });

    /// One unreadable file costs its workspace, never the launch.
    test('a corrupt file is skipped rather than fatal', () {
      final w = _workspace('user-workspace-corrupt');
      w.saveWorkspaceAs('Good');
      File('${Workspace.userWorkspaceDir().path}'
              '${Platform.pathSeparator}broken.$workspaceFileExtension')
          .writeAsStringSync('{ not json');

      expect(_reopened().userWorkspaces.map((s) => s.name), ['Good']);
    });
  });

  group('the strip slots Alt+Shift+1…9 count', () {
    test('the presets come first and the user\'s own after them', () {
      final w = _workspace('user-workspace-slots');
      w.saveWorkspaceAs('Alpha');
      w.saveWorkspaceAs('Beta');

      expect(w.switchToWorkspaceSlot(1), isTrue);
      expect(w.activePreset, WorkspacePreset.values.first);
      expect(w.activeUserWorkspace, isNull);

      final firstUser = WorkspacePreset.values.length + 1;
      expect(w.switchToWorkspaceSlot(firstUser), isTrue);
      expect(w.activeUserWorkspace, 'Alpha');
      expect(w.activePreset, isNull);

      expect(w.switchToWorkspaceSlot(firstUser + 1), isTrue);
      expect(w.activeUserWorkspace, 'Beta');
    });

    /// A slot past the end of the strip is a key that has not been given a
    /// meaning: it answers false so the chord falls through.
    test('a slot past the end of the strip is not handled', () {
      final w = _workspace('user-workspace-slots-empty');
      expect(
          w.switchToWorkspaceSlot(WorkspacePreset.values.length + 1), isFalse);
      expect(w.switchToWorkspaceSlot(0), isFalse);
    });
  });
}
