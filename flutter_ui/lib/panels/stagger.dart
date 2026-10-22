// What the Stagger panel and a stagger drag on bars or keys share: the keys
// that turn a move into a stagger, the order the layers are taken in, and how
// far each one goes.

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:uuid/uuid.dart';

import '../state/workspace.dart';

/// Ctrl+Alt held, or Cmd+Option on a Mac: a move drag staggers instead. Linux
/// takes Ctrl+Shift, as its window managers can move a window on an Alt drag.
bool staggerKeysHeld() {
  final keys = HardwareKeyboard.instance;
  return switch (defaultTargetPlatform) {
    TargetPlatform.macOS => keys.isMetaPressed && keys.isAltPressed,
    TargetPlatform.linux => keys.isControlPressed && keys.isShiftPressed,
    _ => keys.isControlPressed && keys.isAltPressed,
  };
}

/// [stack] put in [order]. [layerOf] says which layer an item is on, for
/// *Order clicked*. Items on a layer that was never clicked come last, in
/// stack order.
List<T> inStaggerOrder<T>(
  List<T> stack,
  LayerStaggerOrder order, {
  List<UuidValue> clicked = const [],
  UuidValue? Function(T item)? layerOf,
}) {
  switch (order) {
    case LayerStaggerOrder.topDown:
      return [...stack];
    case LayerStaggerOrder.bottomUp:
      return stack.reversed.toList();
    case LayerStaggerOrder.random:
      return [...stack]..shuffle();
    case LayerStaggerOrder.clicked:
      int rank(T item) {
        final layer = layerOf?.call(item);
        final at = layer == null ? -1 : clicked.indexOf(layer);
        return at < 0 ? clicked.length : at;
      }
      final indexed = stack.indexed.toList()
        ..sort((a, b) {
          final by = rank(a.$2).compareTo(rank(b.$2));
          return by != 0 ? by : a.$1.compareTo(b.$1);
        });
      return [for (final (_, item) in indexed) item];
  }
}

/// How far the one at [rank] travels when the one at [held] is dragged
/// [delta] frames: the first stays put, the one in hand follows the pointer
/// and the rest are spaced evenly. With the first in hand the last goes the
/// whole way, as After Effects' Quick Offset has it.
double staggerShare({
  required int rank,
  required int held,
  required int count,
  required num delta,
}) {
  final reach = held > 0 ? held : count - 1;
  return reach <= 0 ? 0 : delta * rank / reach;
}

/// The frames between two neighbours in a stagger drag, to one place where
/// the spacing does not come out whole.
String staggerGap({required int held, required int count, required num delta}) {
  final gap = staggerShare(rank: 1, held: held, count: count, delta: delta);
  return gap == gap.roundToDouble() ? '${gap.round()}' : gap.toStringAsFixed(1);
}
