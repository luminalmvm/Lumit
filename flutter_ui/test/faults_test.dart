// A panel whose build throws says so, on screen and on disk.
//
// The regression these hold is the one that made "the Viewer is grey"
// undiagnosable: a release build replaced a failed panel with Flutter's own
// blank grey rectangle and printed the exception to a console a windowed
// Windows build does not have, so the fault left no trace of any kind.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/faults.dart';

/// A file of this test's own, never the diagnostics file a real session — or
/// the developer running this — is writing to.
File _scratch(String name) {
  final dir = Directory.systemTemp.createTempSync('lumit-faults');
  addTearDown(() {
    try {
      dir.deleteSync(recursive: true);
    } catch (_) {}
  });
  return File('${dir.path}${Platform.pathSeparator}$name');
}

/// A widget that cannot be built, which is the whole subject here.
class _Broken extends StatelessWidget {
  const _Broken();

  @override
  Widget build(BuildContext context) => throw StateError('the panel broke');
}

/// Run [body] with the shell's handlers installed, then put back what the test
/// framework insists on finding: it checks `ErrorWidget.builder` at the end of
/// the test *body*, before any tearDown gets a turn.
Future<void> _installed(Future<void> Function() body) async {
  final previousBuilder = ErrorWidget.builder;
  final previousOnError = FlutterError.onError;
  recordFaults();
  try {
    await body();
  } finally {
    ErrorWidget.builder = previousBuilder;
    FlutterError.onError = previousOnError;
  }
}

void main() {
  group('the diagnostics record', () {
    test('names the fault and the frames under it', () {
      final file = _scratch('one.log');
      recordFaultTo(file, 'Bad state: the panel broke', StackTrace.current);

      final written = file.readAsStringSync();
      expect(written, contains('shell: Bad state: the panel broke'));
      expect(written, contains('faults_test.dart'),
          reason: 'the frames are what say which panel it was');
      expect(written.endsWith('\n'), isTrue);
    });

    test('starts again past the cap', () {
      final file = _scratch('big.log');
      file.writeAsStringSync('x' * (256 * 1024 + 1));
      recordFaultTo(file, 'after the cap', null);

      final written = file.readAsStringSync();
      expect(written, contains('after the cap'));
      expect(written.contains('x' * 100), isFalse,
          reason: 'a fault in a loop must not fill the disk');
    });
  });

  group('the fault box', () {
    testWidgets('replaces a failed build with words rather than a grey box',
        (tester) async {
      await _installed(() async {
        await tester.pumpWidget(const Center(child: _Broken()));

        expect(tester.takeException(), isA<StateError>());
        expect(find.byType(FaultBox), findsOneWidget,
            reason: "Flutter's own error widget is blank in a release build");
        expect(find.textContaining('could not be drawn'), findsOneWidget);
        expect(find.textContaining('the panel broke'), findsOneWidget,
            reason: 'the fault has to name itself to be worth photographing');
      });
    });
  });
}
