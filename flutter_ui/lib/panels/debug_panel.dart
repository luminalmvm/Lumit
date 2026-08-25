import 'dart:async';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/performance_view.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/hover_intent.dart';

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

    t = Timer.periodic(Duration(milliseconds: 100), (_) {
      onChange();
    });
    super.initState();
  }

  @override
  void dispose() {
    sub?.cancel();
    // The refresh tick dies with the panel — left running it outlives the
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

    if (ms > 8) {
      return theme.error;
    }

    if (ms > 3) {
      return theme.warning;
    }

    return theme.textMuted;
  }

  @override
  Widget build(BuildContext context) {
    final theme = ThemeScope.of(context).theme;

    var now = DateTime.now();
    var inLastSecond = debugInfo.rustCalls
        .where((i) => (now.difference(i.time).inMilliseconds < 1000));
    int len = inLastSecond.length;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        PerformanceMonitor(),
        // The menu guard is invisible by nature (K-318), so testing it means
        // being able to see it. Amber while it is actually holding a row
        // switch back; accent otherwise.
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
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
              Text('Show safe hover triangles', style: theme.body),
            ],
          ),
        ),
        Text(
          "Statistics:",
          style: theme.body.copyWith(color: theme.textMuted),
        ),
        Flexible(
            child: ListView.builder(
          itemCount: stats.length,
          itemBuilder: (context, index) {
            var item = stats[index];

            return Padding(
              padding: const EdgeInsets.all(2),
              child: Container(
                decoration: BoxDecoration(
                    borderRadius: BorderRadius.circular(4),
                    color: index % 2 == 0 ? theme.surface2 : theme.surface3),
                child: Padding(
                  padding: const EdgeInsets.all(2),
                  child: Row(
                    spacing: 8,
                    children: [
                      SizedBox(
                        width: 50,
                        child: Text(
                          "[${item.value.numCalls}]",
                          style: theme.body.copyWith(color: theme.textMuted),
                        ),
                      ),
                      Text(
                        item.key,
                        style: theme.mono,
                      ),
                      Text(
                        "avg: ${item.value.averageMs.toStringAsFixed(2)}ms",
                        style: theme.body
                            .copyWith(color: msToColor(item.value.averageMs)),
                      ),
                      Text(
                        "last: ${item.value.lastTime.inMilliseconds}ms",
                        style: theme.body.copyWith(
                            color: msToColor(
                                item.value.lastTime.inMilliseconds.toDouble())),
                      ),
                      Text(
                        "total: ${item.value.totalTime.inMilliseconds}ms",
                        style: theme.body,
                      ),
                    ],
                  ),
                ),
              ),
            );
          },
        )),
        SizedBox(
          height: 8,
        ),
        Text(
          "Call Log:",
          style: theme.body.copyWith(color: theme.textMuted),
        ),
        Row(
          children: [
            HouseButton(
              child: Text(
                "$len in last second",
                style: theme.body.copyWith(color: len > 20 ? theme.error : null),
              ),
            ),
            HouseButton(
              child: Text("Clear"),
              onPressed: () {
                debugInfo.clear();
              },
            ),
          ],
        ),
        Flexible(
          flex: 2,
          child: ListView.builder(
            itemCount: items.length,
            itemBuilder: (context, index) {
              return Padding(
                padding: const EdgeInsets.all(2.0),
                child: Row(
                  children: [
                    HouseButton(
                        onPressed: () {
                          showLumitModal(
                            context: context,
                            builder: (close) {
                              return SizedBox(
                                width: 1000,
                                height: 500,
                                child: SingleChildScrollView(
                                  child: Text(
                                    items[index].trace.toString(),
                                    style: theme.mono,
                                  ),
                                ),
                              );
                            },
                          );
                        },
                        child: Row(
                          spacing: 8,
                          children: [
                            Text(
                              "${items[index].duration.inMilliseconds}ms",
                              style: theme.body.copyWith(
                                  color: msToColor(items[index]
                                      .duration
                                      .inMilliseconds
                                      .toDouble())),
                            ),
                            Text(
                              items[index].name,
                              style: theme.mono,
                            ),
                            Text(
                              items[index].time.toLocal().toString(),
                              style: theme.small,
                            ),
                          ],
                        ))
                  ],
                ),
              );
            },
          ),
        ),
      ],
    );
  }
}
