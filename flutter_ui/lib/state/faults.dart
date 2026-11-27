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
//   * [FaultBox] replaces the anonymous grey rectangle with a box that says
//     the part could not be drawn and prints the exception's first line. A
//     user can photograph it and the fault names itself.
//   * [recordFaults] sends every framework error to the file the *engine*
//     already writes its own faults to, so a fault that has scrolled off the
//     screen is still on disk afterwards.
//
// Neither is allowed to be the reason anything fails. Every write is guarded,
// and the box asks the tree for nothing. See [faultTheme] for why that matters.

import 'dart:io' hide Platform;
import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/state/cache_dir.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/state/platform.dart'
    if (dart.library.js_interop) 'package:lumit_flutter/state/platform_web.dart';

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
File faultLog() => File('${lumitCacheDir().path}'
    '${Platform.pathSeparator}lumit-diagnostics.log');

/// Past this many bytes the file starts again — the engine's own cap
/// (`faults.rs`), matched so whichever side rolls it over leaves the same
/// amount of history behind.
const _capBytes = 256 * 1024;

/// Send framework errors to [faultLog] and give a failed build [FaultBox].
///
/// Called once, from `main`. [FlutterError.presentError] is still called for
/// each one, so a debug run keeps its console output. The box takes the place
/// of the red screen there, and of the blank grey rectangle in a release build.
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
    file.parent.createSync(recursive: true);
    // Checked before the write, as the engine does it: the record lands in the
    // file that keeps it rather than in one about to be truncated.
    final over = file.existsSync() && file.lengthSync() > _capBytes;
    final when = DateTime.now().toUtc().toIso8601String();
    final record = '$when shell: ${_faultText(message, stack)}\n';
    // One call, so a fault on the engine's thread cannot land inside this one.
    file.writeAsStringSync(record,
        mode: over ? FileMode.write : FileMode.append, flush: true);
  } catch (_) {
    // A diagnostic that can break the thing it is diagnosing is worse than
    // none — the engine's file says the same, and means it the same way.
  }
}

/// A fault as text: its words, then the frames that place it.
///
/// The frames are cut short deliberately. What is wanted is which panel and
/// which widget, and that is the top of the trace; the rest is Flutter's own
/// build machinery, the same forty lines under every one of these.
String _faultText(String message, StackTrace? stack) {
  final frames = stack == null
      ? const <String>[]
      : stack.toString().trimRight().split('\n').take(12);
  return [message, for (final frame in frames) '    ${frame.trim()}'].join('\n');
}

/// The theme the fault box wears. `main` points it at the workspace's own once
/// there is one. Asked for here and not from the tree: the box replaces a
/// widget that has just failed to build, and a lookup is among the ways a
/// build fails. If the box threw as well, Flutter would give up on the frame.
LumitTheme Function() faultTheme = LumitTheme.dark;

/// What is drawn where a widget failed to build.
///
/// It fills whatever the failed widget was offered, a whole panel or a single
/// row, so where the box is says which part broke.
class FaultBox extends StatefulWidget {
  final FlutterErrorDetails details;

  const FaultBox({super.key, required this.details});

  @override
  State<FaultBox> createState() => _FaultBoxState();
}

class _FaultBoxState extends State<FaultBox> {
  bool _copied = false;

  void _copy() {
    final details = widget.details;
    // A clipboard that refuses is not a second fault.
    Clipboard.setData(ClipboardData(
            text: '${_faultText(details.exceptionAsString(), details.stack)}\n'
                '${faultLog().path}'))
        .ignore();
    setState(() => _copied = true);
  }

  @override
  Widget build(BuildContext context) {
    final t = faultTheme();
    final quiet = t.small;
    // One line height for the heading and the fault's own words, so a row
    // has room for exactly one line of each.
    const line = 16.0;
    // The lines under the heading start where its words do, past the dot.
    const indent = EdgeInsets.only(left: 14);
    // Its own [Directionality] and [DefaultTextStyle]: the box can land above
    // the ones the application installs, and must not wear what a stray
    // ancestor set either.
    return Directionality(
      textDirection: TextDirection.ltr,
      child: DefaultTextStyle(
        style: t.body,
        child: Container(
          clipBehavior: Clip.antiAlias,
          decoration: BoxDecoration(
            color: t.surface2,
            border: Border.all(color: t.hairline),
            borderRadius: BorderRadius.circular(t.tokens.sectionRadius),
          ),
          child: CustomMultiChildLayout(
            delegate: _FaultLayout(),
            children: [
              LayoutId(
                id: 0,
                child: Text.rich(
                  TextSpan(children: [
                    WidgetSpan(
                      alignment: PlaceholderAlignment.middle,
                      child: Container(
                        width: 6,
                        height: 6,
                        margin: const EdgeInsets.only(right: 8),
                        decoration: BoxDecoration(
                            color: t.error, shape: BoxShape.circle),
                      ),
                    ),
                    TextSpan(text: l10n.faultPanelTitle),
                  ]),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: t.bodyPrimary.copyWith(
                      fontSize: 12,
                      height: line / 12,
                      fontWeight: FontWeight.w500),
                ),
              ),
              LayoutId(
                id: 1,
                child: Padding(
                  padding: indent,
                  child: Text(
                    faultSummary(widget.details),
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: t.mono.copyWith(
                        fontSize: quiet.fontSize,
                        height: line / quiet.fontSize!,
                        color: quiet.color),
                  ),
                ),
              ),
              LayoutId(
                id: 2,
                child: Padding(
                  padding: indent,
                  child: Text(
                    l10n.faultPanelWhere(faultLog().path),
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: quiet,
                  ),
                ),
              ),
              LayoutId(
                id: 3,
                child: Padding(
                  padding: indent + const EdgeInsets.only(top: 4),
                  child: MouseRegion(
                    cursor: SystemMouseCursors.click,
                    child: GestureDetector(
                      onTap: _copy,
                      // The house button's resting face: an outline over the
                      // surface it stands on.
                      child: Container(
                        padding: const EdgeInsets.symmetric(
                            horizontal: 8, vertical: 3),
                        decoration: BoxDecoration(
                          border: Border.all(color: t.hairlineStrong),
                          borderRadius:
                              BorderRadius.circular(t.tokens.actionRadius),
                        ),
                        child: Text(
                          _copied ? l10n.faultCopied : l10n.faultCopy,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: t.bodyPrimary,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Stacks the box's lines and keeps only the whole ones that fit, so nothing
/// is cut in half. The box takes all it is offered, and a modest size where it
/// is offered no limit.
class _FaultLayout extends MultiChildLayoutDelegate {
  static const _lines = 4;
  static const _gap = 6.0;
  static const _edge = 12.0;
  static const _above = 8.0;

  @override
  Size getSize(BoxConstraints constraints) => constraints.constrain(Size(
      constraints.hasBoundedWidth ? constraints.maxWidth : 360,
      constraints.hasBoundedHeight ? constraints.maxHeight : 120));

  @override
  void performLayout(Size size) {
    final inner = math.max(0.0, size.width - 2 * _edge);
    final head = layoutChild(0, BoxConstraints(maxWidth: inner));
    // A line with no room goes below the box, where the clip hides it.
    final hidden = Offset(0, size.height + _edge);
    final block = BoxConstraints(maxWidth: math.min(inner, 440));
    if (size.height - 2 * _above < 2 * head.height + _gap) {
      // A row, with no room for a line below. It reads from its left edge
      // like the rows round it, and the fault's own words follow the heading
      // for as far as the row runs.
      final words = layoutChild(
          1,
          BoxConstraints(
              maxWidth: inner - head.width,
              maxHeight: head.height));
      final top = (size.height - head.height) / 2;
      positionChild(0, Offset(_edge, top));
      positionChild(
          1,
          Offset(_edge + head.width,
              top + (head.height - words.height) / 2));
      for (var i = 2; i < _lines; i++) {
        layoutChild(i, block);
        positionChild(i, hidden);
      }
      return;
    }
    final sizes = [head, for (var i = 1; i < _lines; i++) layoutChild(i, block)];
    var shown = 1;
    var height = head.height;
    var width = head.width;
    while (shown < _lines &&
        height + _gap + sizes[shown].height <= size.height - 2 * _above) {
      height += _gap + sizes[shown].height;
      width = math.max(width, sizes[shown].width);
      shown++;
    }
    // The lines stand in the middle as one block.
    final left = (size.width - width) / 2;
    var top = (size.height - height) / 2;
    for (var i = 0; i < _lines; i++) {
      positionChild(i, i < shown ? Offset(left, top) : hidden);
      top += sizes[i].height + _gap;
    }
  }

  @override
  bool shouldRelayout(_FaultLayout oldDelegate) => false;
}

/// The fault in as few words as identify it: the widget that failed, then the
/// first line of the exception.
///
/// The first line only: a Flutter exception's own text is often a paragraph
/// with the offending widget's whole description in it, and the box has room
/// for a sentence. The file has the rest.
String faultSummary(FlutterErrorDetails details) {
  final full = details.exceptionAsString().trim();
  final firstLine = full.split('\n').first.trim();
  // Flutter's context for a failed build is "building" and the widget.
  final part = RegExp(r'^building (\w+)')
      .firstMatch(details.context?.toDescription() ?? '')
      ?.group(1);
  return '${part == null ? '' : '$part: '}'
      '${firstLine.isEmpty ? full : firstLine}';
}
