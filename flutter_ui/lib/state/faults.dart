// What the shell does when a panel's build throws.
//
// # In plain terms
//
// A Flutter widget builds itself by running a piece of code. If that code
// throws, Flutter does not take the window down — it replaces just that widget
// with an "error widget" and carries on drawing everything around it. That is a
// good trade: one broken panel instead of a dead editor.
//
// The trouble is what the stock error widget *is*. In a debug build it is the
// familiar red screen with the exception written across it. In a release build
// the message is stripped out and what is left is a plain grey rectangle — no
// words, no colour, nothing to search for. Lumit shipped with no handler of its
// own, so a fault in a panel looked exactly like a panel that had drawn itself
// empty, and the exception behind it went nowhere at all: a windowed Windows
// build has no console for it to be printed to.
//
// That combination is why "the Viewer is grey" could not be diagnosed from a
// screenshot. Two things fix it, and both are here:
//
//   * [FaultBox] replaces the anonymous grey rectangle with a box that says a
//     panel could not be drawn and prints the exception's first line. A user
//     can photograph it and the fault names itself.
//   * [recordFaults] sends every framework error to the file the *engine*
//     already writes its own faults to, so a fault that has scrolled off the
//     screen is still on disk afterwards.
//
// Neither is allowed to be the reason anything fails. Every write is guarded,
// and the box asks the tree for nothing — see [FaultColours] for why that
// matters.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/theme/theme.dart';

/// The file the engine appends its own faults to
/// (`crates/lumit-bridge/src/faults.rs`), which this writes to as well.
///
/// **One file, not two.** The engine's is already capped, already printed at
/// start-up so a bug report can say where to look, and already the thing a user
/// is asked for. A second file beside it would mean asking for both and reading
/// them interleaved by hand — and a Dart fault and the engine fault that caused
/// it belong on adjacent lines, not in different places.
///
/// Reached by path rather than through the bridge because a bridge call is the
/// wrong dependency for a diagnostic: this has to work when the engine is what
/// went wrong. Both sides append whole records in a single write, which is what
/// keeps two writers from splitting each other's lines.
File faultLog() => File('${Directory.systemTemp.path}'
    '${Platform.pathSeparator}lumit-diagnostics.log');

/// Past this many bytes the file starts again — the engine's own cap
/// (`faults.rs`), matched so whichever side rolls it over leaves the same
/// amount of history behind.
const _capBytes = 256 * 1024;

/// Send framework errors to [faultLog] and give a failed build [FaultBox].
///
/// Called once, from `main`. [FlutterError.presentError] is still called for
/// each one, so a debug run keeps the console output and the red screen it has
/// always had; this adds the file and the box a *release* build had neither of.
void recordFaults() {
  final wasPresenting = FlutterError.onError;
  FlutterError.onError = (details) {
    (wasPresenting ?? FlutterError.presentError)(details);
    recordFault(details.exceptionAsString(), details.stack);
  };
  ErrorWidget.builder = (details) => FaultBox(details: details);
}

/// Append one fault to [faultLog]. Never throws, whatever the disk says.
void recordFault(String message, StackTrace? stack) =>
    recordFaultTo(faultLog(), message, stack);

/// The write itself, with the file passed in — the same split the engine's
/// `record_to` makes, and for the same reason: the size cap can be tested
/// without filling the file a real session is writing to.
void recordFaultTo(File file, String message, StackTrace? stack) {
  try {
    // Checked before the write, as the engine does it: the record lands in the
    // file that keeps it rather than in one about to be truncated.
    final over = file.existsSync() && file.lengthSync() > _capBytes;
    final when = DateTime.now().toUtc().toIso8601String();
    // The frames are cut short deliberately. What is wanted is which panel and
    // which widget, and that is the top of the trace; the rest is Flutter's own
    // build machinery, the same forty lines under every one of these.
    final frames = stack == null
        ? const <String>[]
        : stack.toString().trimRight().split('\n').take(12);
    final record = StringBuffer('$when shell: $message');
    for (final frame in frames) {
      record.write('\n    ${frame.trim()}');
    }
    record.write('\n');
    // One call, so a fault on the engine's thread cannot land inside this one.
    file.writeAsStringSync(record.toString(),
        mode: over ? FileMode.write : FileMode.append, flush: true);
  } catch (_) {
    // A diagnostic that can break the thing it is diagnosing is worse than
    // none — the engine's file says the same, and means it the same way.
  }
}

/// What is drawn where a widget failed to build.
///
/// Sized to whatever the failed widget was given, which can be a whole panel or
/// a single row, so everything here has to survive being drawn very small: the
/// text is clipped rather than allowed to overflow, and nothing measures itself
/// against a minimum.
class FaultBox extends StatelessWidget {
  final FlutterErrorDetails details;

  const FaultBox({super.key, required this.details});

  @override
  Widget build(BuildContext context) {
    const colours = FaultColours.standard;
    // Its own [Directionality] and [DefaultTextStyle]: this widget can land
    // anywhere, including above the ones the application installs, and a [Text]
    // with neither ancestor throws — which would make the error widget the next
    // error, and Flutter stops drawing the frame when that happens.
    return Directionality(
      textDirection: TextDirection.ltr,
      child: ColoredBox(
        color: colours.background,
        child: ClipRect(
          child: Padding(
            padding: const EdgeInsets.all(8),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  l10n.faultPanelTitle,
                  // Not `const`: reading a field off a const object is not
                  // itself a constant expression in Dart.
                  style:
                      const TextStyle(fontSize: 12, fontWeight: FontWeight.w600)
                          .copyWith(color: colours.heading),
                ),
                const SizedBox(height: 4),
                Text(
                  faultSummary(details),
                  maxLines: 4,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(fontSize: 11)
                      .copyWith(color: colours.detail),
                ),
                const SizedBox(height: 4),
                Text(
                  l10n.faultPanelWhere(faultLog().path),
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(fontSize: 11)
                      .copyWith(color: colours.detail),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// The exception in as few words as identify it.
///
/// The first line only: a Flutter exception's own text is often a paragraph
/// with the offending widget's whole description in it, and the box has room
/// for a sentence. The file has the rest.
String faultSummary(FlutterErrorDetails details) {
  final full = details.exceptionAsString().trim();
  final firstLine = full.split('\n').first.trim();
  return firstLine.isEmpty ? full : firstLine;
}
