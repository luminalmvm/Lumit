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

    test('appends rather than replacing', () {
      final file = _scratch('two.log');
      recordFaultTo(file, 'first', null);
      recordFaultTo(file, 'second', null);

      final written = file.readAsStringSync();
      expect(written, contains('first'));
      expect(written, contains('second'),
          reason: 'a session that faults twice keeps both');
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

    test('a write that cannot happen is not a crash', () {
      // A directory where a file should be: the write throws inside, and the
      // caller must never see it. A diagnostic that can break what it is
      // diagnosing is worse than none.
      final dir = Directory.systemTemp.createTempSync('lumit-faults-dir');
      addTearDown(() => dir.deleteSync(recursive: true));
      expect(() => recordFaultTo(File(dir.path), 'x', null), returnsNormally);
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

    testWidgets('survives being drawn very small', (tester) async {
      // A failed row is given a row's worth of space, not a panel's. The box
      // has to clip rather than overflow: an overflow in this of all widgets
      // would be a second fault on top of the first.
      await _installed(() async {
        await tester.pumpWidget(const Center(
          child: SizedBox(width: 40, height: 12, child: _Broken()),
        ));

        expect(tester.takeException(), isA<StateError>());
        expect(find.byType(FaultBox), findsOneWidget);
      });
    });

    testWidgets('needs no Directionality of its own above it', (tester) async {
      // The box can land anywhere, including above the widgets the application
      // installs. A `Text` with no `Directionality` ancestor throws, and an
      // error widget that errors costs the whole frame.
      await tester.pumpWidget(FaultBox(
        details: FlutterErrorDetails(exception: StateError('no direction')),
      ));

      expect(tester.takeException(), isNull);
      expect(find.textContaining('no direction'), findsOneWidget);
    });
  });

  group('the summary', () {
    test('is the exception first line only', () {
      // Flutter's own exception text runs to a paragraph with the offending
      // widget's whole description in it. The box has room for a sentence; the
      // file keeps the rest.
      final details = FlutterErrorDetails(
        exception: StateError('the panel broke\nand here is why\nat length'),
      );
      expect(faultSummary(details), contains('the panel broke'));
      expect(faultSummary(details), isNot(contains('at length')));
    });
  });
}
