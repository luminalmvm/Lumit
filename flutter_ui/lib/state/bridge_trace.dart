// The debug-build tracer for calls that cross into Rust, lifted out of
// main.dart unchanged. See CustomHandler for why it is debug-only.

import 'dart:async';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

class StackTraceEntry {
  StackTrace trace;
  String name;
  late DateTime time;
  late Duration duration;
  bool async;

  StackTraceEntry(
      {required this.name,
      required this.trace,
      required this.duration,
      required this.async}) {
    time = DateTime.now();
  }
}

class FunctionCallStats {
  int numCalls = 0;
  Duration totalTime = Duration.zero;
  Duration lastTime = Duration.zero;

  double get averageMs =>
      totalTime.inMilliseconds.toDouble() / numCalls.toDouble();
}

class LumitDebugUI {
  List<StackTraceEntry> rustCalls = List.empty(growable: true);
  Map<String, FunctionCallStats> stats = {};

  StreamController onChange = StreamController.broadcast();

  void addStackTrace(StackTraceEntry trace) {
    rustCalls.insert(0, trace);

    const maxLen = 100;

    if (stats.containsKey(trace.name) == false) {
      stats[trace.name] = FunctionCallStats();
    }
    var stat = stats[trace.name]!;

    stat.numCalls += 1;
    stat.totalTime += trace.duration;
    stat.lastTime = trace.duration;

    if (rustCalls.length > maxLen) {
      rustCalls = rustCalls.sublist(0, maxLen);
    }

    onChange.add(null);
  }

  void clear() {
    rustCalls.clear();
    onChange.add(null);
  }
}

LumitDebugUI debugInfo = LumitDebugUI();

/// Traces every call that crosses into Rust, so the frb seam can be watched
/// while it is being built out. `debugPrint` rather than `print`: it compiles
/// away in release, where a log per bridge call would be far too costly.
class CustomHandler extends BaseHandler {
  @override
  Future<S> executeNormal<S, E extends Object>(NormalTask<S, E> task) async {
    var stack = StackTrace.current;

    var str = stack.toString();
    var lines = str.split("\n");

    var target = lines.elementAtOrNull(2);
    var split = target?.split(" ");

    final start = DateTime.now();
    final result = await super.executeNormal(task);
    final end = DateTime.now();

    var duration = end.difference(start);

    if (split != null) {
      final item = split.elementAtOrNull(split.length - 2);
      debugInfo.addStackTrace(StackTraceEntry(
          name: item!, trace: stack, duration: duration, async: true));
    }

    return result;
  }

  @override
  S executeSync<S, E extends Object, WireSyncType>(
      SyncTask<S, E, WireSyncType> task) {
    var stack = StackTrace.current;

    var str = stack.toString();
    var lines = str.split("\n");

    var target = lines.elementAtOrNull(2);
    var split = target?.split(" ");

    final start = DateTime.now();
    final result = super.executeSync(task);
    final end = DateTime.now();

    var duration = end.difference(start);

    if (split != null) {
      final item = split.elementAtOrNull(split.length - 2);
      debugInfo.addStackTrace(StackTraceEntry(
          name: item!, trace: stack, duration: duration, async: false));
    }

    return result;
  }
}
