// The Debug view: how fast the interface is drawing, and every call it has made
// over the bridge, slowest first and as they happened.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/performance_view.dart';
import 'package:lumit_flutter/shell/dialog_frame.dart';
import 'package:lumit_flutter/shell/question_window.dart';
import 'package:lumit_flutter/shell/review_link_parts.dart' show reviewRowMenu;
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/hover_intent.dart';

import '../l10n/strings.dart';

/// The columns of a statistics row, and how tall a row is.
const double _countColumn = 48;
const double _timeColumn = 60;
const double _rowHeight = 20;

class DebugPanel extends StatefulWidget {
  const DebugPanel({super.key});

  @override
  State<DebugPanel> createState() => _DebugPanelState();
}

class _DebugPanelState extends State<DebugPanel> {
  StreamSubscription? sub;
  late List<StackTraceEntry> items;
  late List<MapEntry<String, FunctionCallStats>> stats;
  late Timer t;

  @override
  void initState() {
    sub = debugInfo.onChange.stream.listen((_) => onChange());
    items = debugInfo.rustCalls.toList();

    getStats();

    t = Timer.periodic(const Duration(milliseconds: 100), (_) {
      onChange();
    });
    super.initState();
  }

  @override
  void dispose() {
    sub?.cancel();
    // The refresh tick dies with the panel. Left running it outlives the
    // widget tree, which the test binding rightly calls a leak.
    t.cancel();
    super.dispose();
  }

  void onChange() {
    setState(() {
      getStats();
      items = debugInfo.rustCalls.toList();
    });
  }

  void getStats() {
    stats = debugInfo.stats.entries.toList();
    stats.sort((a, b) => b.value.averageMs.compareTo(a.value.averageMs));
  }

  Color msToColor(double ms) {
    final theme = ThemeScope.of(context).theme;
    if (ms > 8) return theme.error;
    if (ms > 3) return theme.warning;
    return theme.textMuted;
  }

  void _copy(String text) =>
      unawaited(Clipboard.setData(ClipboardData(text: text)));

  void _showTrace(StackTraceEntry entry) {
    final trace = entry.trace.toString();
    showLumitModal<void>(
      context: context,
      id: 'debug-trace',
      initialSize: const Size(760, 460),
      builder: (close) => _TraceWindow(
          name: entry.name, trace: trace, onClose: () => close(null)),
    );
  }

  Widget _heading(LumitTheme theme, String words, {List<Widget> end = const []}) =>
      Padding(
        padding: const EdgeInsets.only(top: 10, bottom: 4),
        child: Row(
          children: [
            Expanded(
                child: Text(theme.kickerCase(words), style: theme.kicker)),
            ...end,
          ],
        ),
      );

  /// A figure in its column.
  Widget _figure(LumitTheme theme, String text, double width, Color? colour) =>
      SizedBox(
        width: width,
        child: Text(text,
            textAlign: TextAlign.right,
            style: dialogMono(theme).copyWith(color: colour)),
      );

  @override
  Widget build(BuildContext context) {
    final theme = ThemeScope.of(context).theme;
    final now = DateTime.now();
    final len = debugInfo.rustCalls
        .where((i) => now.difference(i.time).inMilliseconds < 1000)
        .length;

    return Padding(
      padding: const EdgeInsets.all(8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const PerformanceMonitor(),
          // The menu guard is invisible by nature, so testing it means being
          // able to see it. Amber while it is actually holding a row switch
          // back, and the accent otherwise.
          Padding(
            padding: const EdgeInsets.only(top: 6),
            child: Row(
              spacing: 8,
              children: [
                ValueListenableBuilder<bool>(
                  valueListenable: debugShowSafeTriangles,
                  builder: (_, on, __) => HouseCheckbox(
                    value: on,
                    onChanged: (v) => debugShowSafeTriangles.value = v,
                  ),
                ),
                Expanded(
                  child: Text(l10n.debugSafeTriangles,
                      style: theme.small, overflow: TextOverflow.ellipsis),
                ),
              ],
            ),
          ),
          _heading(theme, l10n.debugStatistics),
          Row(
            children: [
              Expanded(child: Text(l10n.debugCalls, style: theme.small)),
              _figure(theme, l10n.debugCount, _countColumn, theme.textMuted),
              _figure(theme, l10n.debugAverage, _timeColumn, theme.textMuted),
              _figure(theme, l10n.debugLast, _timeColumn, theme.textMuted),
              _figure(theme, l10n.debugTotal, _timeColumn, theme.textMuted),
            ],
          ),
          Flexible(
            child: ListView.builder(
              itemCount: stats.length,
              itemExtent: _rowHeight,
              itemBuilder: (context, index) {
                final item = stats[index];
                final last = item.value.lastTime.inMilliseconds;
                return GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onSecondaryTapDown: (press) => reviewRowMenu(
                      context, press.globalPosition, [
                    ('debug-copy-name', l10n.debugCopyName,
                        () => _copy(item.key)),
                  ]),
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(item.key,
                            style: dialogMono(theme)
                                .copyWith(color: theme.textPrimary),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis),
                      ),
                      _figure(theme, '${item.value.numCalls}', _countColumn,
                          theme.textMuted),
                      _figure(
                          theme,
                          '${item.value.averageMs.toStringAsFixed(2)} ms',
                          _timeColumn,
                          msToColor(item.value.averageMs)),
                      _figure(theme, '$last ms', _timeColumn,
                          msToColor(last.toDouble())),
                      _figure(
                          theme,
                          '${item.value.totalTime.inMilliseconds} ms',
                          _timeColumn,
                          theme.textMuted),
                    ],
                  ),
                );
              },
            ),
          ),
          _heading(theme, l10n.debugCallLog, end: [
            Text(l10n.debugInLastSecond(len),
                style: dialogMono(theme)
                    .copyWith(color: len > 20 ? theme.error : null)),
            const SizedBox(width: 8),
            HouseButton(
              key: const ValueKey('debug-clear'),
              small: true,
              onPressed: debugInfo.clear,
              child: Text(l10n.clear, style: theme.small),
            ),
          ]),
          Flexible(
            flex: 2,
            child: ListView.builder(
              itemCount: items.length,
              itemExtent: _rowHeight,
              itemBuilder: (context, index) {
                final entry = items[index];
                final ms = entry.duration.inMilliseconds;
                return HoverMark(
                  builder: (context, lit) => GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: () => _showTrace(entry),
                    onSecondaryTapDown: (press) => reviewRowMenu(
                        context, press.globalPosition, [
                      ('debug-trace', l10n.debugTrace,
                          () => _showTrace(entry)),
                      null,
                      ('debug-copy-call', l10n.debugCopyName,
                          () => _copy(entry.name)),
                      ('debug-copy-trace', l10n.debugCopyTrace,
                          () => _copy(entry.trace.toString())),
                    ]),
                    child: ColoredBox(
                      color: theme.surface2.withValues(alpha: lit),
                      child: Row(
                        spacing: 8,
                        children: [
                          _figure(theme, '$ms ms', _timeColumn,
                              msToColor(ms.toDouble())),
                          Expanded(
                            child: Text(entry.name,
                                style: dialogMono(theme)
                                    .copyWith(color: theme.textPrimary),
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis),
                          ),
                          Text(
                            entry.time
                                .toLocal()
                                .toIso8601String()
                                .substring(11, 23),
                            style: dialogMono(theme),
                          ),
                        ],
                      ),
                    ),
                  ),
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}

/// One call's stack, in the frame every other window has.
class _TraceWindow extends StatefulWidget {
  final String name, trace;
  final VoidCallback onClose;

  const _TraceWindow(
      {required this.name, required this.trace, required this.onClose});

  @override
  State<_TraceWindow> createState() => _TraceWindowState();
}

class _TraceWindowState extends State<_TraceWindow> {
  final _scroll = ScrollController();

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = ThemeScope.of(context).theme;
    return DialogFrame(
      children: [
        dialogTitleBar(theme,
            title: l10n.debugTrace,
            subject: widget.name,
            onClose: widget.onClose,
            keyPrefix: 'debug-trace'),
        Expanded(
          child: dialogScroller(
            context,
            theme,
            _scroll,
            child: SingleChildScrollView(
              controller: _scroll,
              padding: const EdgeInsets.all(dialogPadding),
              child: Text(widget.trace, style: dialogMono(theme)),
            ),
          ),
        ),
        dialogFooter(
          theme,
          keyPrefix: 'debug-trace',
          actions: [
            questionPlain(
                'debug-trace-copy',
                l10n.debugCopyTrace,
                () => unawaited(
                    Clipboard.setData(ClipboardData(text: widget.trace)))),
            questionFilled('debug-trace-close', l10n.close, widget.onClose),
          ],
        ),
      ],
    );
  }
}
