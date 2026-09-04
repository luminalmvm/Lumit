// What app_en.arb is allowed to say (docs/07-UI-SPEC.md §13.2).
//
// The .arb is the one file a translator reads, so the rules that used to live in
// reviewers' heads live here instead: a tooltip is the control's name, not a
// sentence about it; every string carries a note saying where it appears; and
// the glossary's banned words stay banned in the copy as well as in the code.

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// The longest a tooltip may be, in words.
///
/// **There is no exception list, and one may not be added**. This file
/// used to keep nine names that were allowed a sentence each — the cache
/// meters, the playback modes, the Flow switches, the preview-view badge —
/// under docs/07-UI-SPEC.md §13.2's reserved "rich" tooltip. The reservation is
/// withdrawn: a tooltip is a name, so the same limit holds for every one of
/// them. A string that will not fit belongs in a settings row's own line, in an
/// empty state, or nowhere.
///
/// The limit is **two**, which is what docs/07-UI-SPEC.md §13.2 and
/// docs/15-DESIGN.md both say — "one or two words, never more".
/// It stood at five here for as long as the copy did, so the gate agreed with
/// the older rule and let a sentence through as long as it was a short one.
/// A shortcut no longer fits beside the name; the keymap teaches those, and
/// the toolbar still appends the live chord to the tool's own label, which is
/// composed in code and not a string in this file.
const _tooltipWordLimit = 2;

/// docs/01-GLOSSARY.md §9, as it applies to what the user reads. `render` is
/// missing on purpose: it is banned only for writing a file, and the Timeline's
/// render-time column and the Viewer's render stages are the legitimate sense.
const _banned = {
  'track': 'layer',
  'velocity': 'speed',
  'time remap': 'Retime',
  'time-remap': 'Retime',
  'CTI': 'playhead',
};

/// Strings where a banned word is not the banned *sense*.
///
/// The glossary bans "track" where Lumit means a **layer**. It says nothing
/// about tracking as a verb (§9), and these are features whose names contain
/// it: following a camera, following motion, and the matte that After Effects
/// calls a track matte. The menu rows are for work not built yet;
/// `fxCameraTrack` is the effect itself, which is built.
const _bannedWordIsAnotherSense = {
  'menuTrackCamera',
  'menuTrackMotion',
  'menuTrackMatte',
  'toolCameraPan',
  'fxCameraTrack',
  // The planar tracker is the same verb sense as the camera's.
  'fxPlanarTrack',
};

/// Every `.arb` in lib/l10n, source and translations alike, in a stable order.
List<File> _arbFiles() => (Directory('lib/l10n')
    .listSync()
    .whereType<File>()
    .where((f) => f.path.endsWith('.arb'))
    .toList())
  ..sort((a, b) => a.path.compareTo(b.path));

Map<String, dynamic> _arb() =>
    json.decode(File('lib/l10n/app_en.arb').readAsStringSync())
        as Map<String, dynamic>;

Iterable<MapEntry<String, String>> _messages(Map<String, dynamic> arb) =>
    arb.entries
        .where((e) => !e.key.startsWith('@'))
        .map((e) => MapEntry(e.key, e.value as String));

void main() {
  test('app_en.arb is valid JSON with messages in it', () {
    expect(_messages(_arb()).length, greaterThan(500));
  });

  test('a tooltip is the control name, not a sentence about it', () {
    final long = <String>[];
    var checked = 0;
    for (final m in _messages(_arb())) {
      if (!m.key.startsWith('tip')) continue;
      checked++;
      // A placeholder stands for one word whatever it expands to.
      final words = m.value.replaceAll(RegExp(r'\{\w+\}'), 'x').split(' ');
      if (words.length > _tooltipWordLimit) long.add('${m.key}: "${m.value}"');
    }
    // The walk itself is asserted: a renamed prefix would otherwise turn this
    // into a test that checks nothing and passes for ever.
    expect(checked, greaterThan(50), reason: 'the tip* keys were not found');
    expect(
      long,
      isEmpty,
      reason: 'a tooltip is the control\'s name '
          '(docs/07-UI-SPEC.md §13.2) — one word where one will do, and '
          'never more than $_tooltipWordLimit. Shorten these; there is '
          'no exception list to add them to.',
    );
  });

  test('every string tells the translator where it appears', () {
    final arb = _arb();
    final undescribed = <String>[];
    for (final m in _messages(arb)) {
      final meta = arb['@${m.key}'] as Map<String, dynamic>?;
      final description = meta?['description'] as String?;
      if (description == null || description.trim().length < 10) {
        undescribed.add(m.key);
      }
    }
    expect(
      undescribed,
      isEmpty,
      reason: 'The description is shown beside the string and is all the '
          'context a translator gets. Say where it appears and what '
          'constrains it.',
    );
  });

  test('the copy uses the glossary words', () {
    final wrong = <String>[];
    for (final m in _messages(_arb())) {
      if (_bannedWordIsAnotherSense.contains(m.key)) continue;
      for (final entry in _banned.entries) {
        final pattern =
            RegExp('\\b${RegExp.escape(entry.key)}\\b', caseSensitive: false);
        if (pattern.hasMatch(m.value)) {
          wrong.add('${m.key} says "${entry.key}" — say "${entry.value}"');
        }
      }
    }
    expect(wrong, isEmpty,
        reason: 'docs/01-GLOSSARY.md §9 is binding for copy');
  });

  test('every placeholder in a string is declared', () {
    // An undeclared placeholder is generated as literal braces, so the user sees
    // "{path}" where the file name should be.
    final arb = _arb();
    final bad = <String>[];
    for (final m in _messages(arb)) {
      final used = RegExp(r'\{(\w+)\}')
          .allMatches(m.value)
          .map((x) => x.group(1)!)
          .toSet();
      if (used.isEmpty) continue;
      final meta = arb['@${m.key}'] as Map<String, dynamic>?;
      final declared =
          (meta?['placeholders'] as Map<String, dynamic>?)?.keys.toSet() ??
              const <String>{};
      final missing = used.difference(declared);
      if (missing.isNotEmpty) bad.add('${m.key}: ${missing.join(', ')}');
    }
    expect(bad, isEmpty);
  });

  test('the target languages have a file to be translated into', () {
    // The ingest tool writes these; a short one is normal and means the
    // language is part-way done. Every key it lacks falls back to English.
    for (final tag in ['de', 'kk', 'uk', 'zh', 'zh_Hant']) {
      expect(File('lib/l10n/app_$tag.arb').existsSync(), isTrue,
          reason: 'app_$tag.arb is missing — it is a target language');
    }
  });

  test('every .arb names the locale its filename says', () {
    // Flutter's generator refuses to run when these disagree, and it runs on
    // `flutter pub get` — so a file called app_zh.arb that says
    // `"@@locale": "zh-CN"` takes down every Flutter job in CI before a single
    // test is reached. It has happened twice, which is why this is a
    // test and not a convention.
    final wrong = <String>[];
    for (final file in _arbFiles()) {
      final name = file.uri.pathSegments.last;
      final fromName =
          name.substring('app_'.length, name.length - '.arb'.length);
      final declared = (json.decode(file.readAsStringSync())
          as Map<String, dynamic>)['@@locale'] as String?;
      if (declared != fromName) {
        wrong.add('$name says "@@locale": "$declared" — expected "$fromName"');
      }
    }
    expect(wrong, isEmpty,
        reason: 'the @@locale key and the filename name the same language');
  });

  test('there is no en-US', () {
    // British English is the source and stays the source. An
    // app_en_US.arb is a copy of the source under another name — an American
    // spelling of Lumit is not a translation, it is a second source to keep
    // level with the first.
    expect(File('lib/l10n/app_en_US.arb').existsSync(), isFalse,
        reason: 'British English is the source and there is no en-US');
  });
}
