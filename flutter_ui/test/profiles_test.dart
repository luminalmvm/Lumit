// Profiles put one person's settings away and take another's out. A mistake
// there loses somebody's shortcuts or theme without a word, so this checks
// the round trip: a profile comes back as it was left, the machine's own
// settings never move, and two machines' edits merge without one eating the
// other.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/account.dart';
import 'package:lumit_flutter/state/profiles.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';

/// A settings folder of this test's own, emptied first.
Workspace _workspace(String name) {
  final dir = Directory('${Directory.systemTemp.path}'
      '${Platform.pathSeparator}lumit-test-$name');
  if (dir.existsSync()) dir.deleteSync(recursive: true);
  Workspace.storeOverride =
      '${dir.path}${Platform.pathSeparator}workspace.json';
  addTearDown(() => Workspace.storeOverride = null);
  return Workspace();
}

void main() {
  test('a profile comes back as it was left, and the machine stays put',
      () async {
    final w = _workspace('profiles-switch');
    final profiles = ProfilesState(w, AccountState(), afterApply: () {});
    final mine = profiles.current;
    profiles.rename(mine, 'Mack');
    w.setScheme(LumitColorScheme.darkBlue);
    w.setAutosave(7, 5);
    w.setShareName('Mack');

    final theirs = profiles.add('Client desk');
    await profiles.switchTo(theirs);
    expect(w.colorScheme, LumitColorScheme.darkBlue,
        reason: 'a new profile starts with the settings in force');
    w.setScheme(LumitColorScheme.light);
    w.setAutosave(3, 5);
    // The machine's, not the profile's: set while one profile is in use and
    // still there under the other.
    w.setAudioDevice('speakers');

    await profiles.switchTo(mine);
    expect(w.colorScheme, LumitColorScheme.darkBlue);
    expect(w.autosaveMinutes, 7);
    expect(w.shareName, 'Mack');
    expect(w.audioDevice, 'speakers');

    await profiles.switchTo(theirs);
    expect(w.colorScheme, LumitColorScheme.light);
    expect(w.autosaveMinutes, 3);

    // What the next launch reads: the same list, on the same profile.
    final next = ProfilesState(Workspace()..load(), AccountState(),
        afterApply: () {});
    expect(next.all.map((p) => p.name), ['Mack', 'Client desk']);
    expect(next.current.name, 'Client desk');
    expect(next.workspace.colorScheme, LumitColorScheme.light);
  });

  test('edits on two machines merge setting by setting', () {
    const base = {
      'theme': 'dark',
      'interface': {'compact': false, 'tooltips': true},
      'old': 1,
    };
    final here = {
      'theme': 'light',
      'interface': {'compact': false, 'tooltips': false},
      'old': 1,
    };
    final there = {
      'theme': 'blue',
      'interface': {'compact': true, 'tooltips': true},
      'new': 2,
    };
    expect(mergeSettings(base, here, there), {
      // Changed on both: this machine's is kept.
      'theme': 'light',
      // Changed on one each, inside the same group: both are kept.
      'interface': {'compact': true, 'tooltips': false},
      // Added there, and removed there.
      'new': 2,
    });
  });
}
