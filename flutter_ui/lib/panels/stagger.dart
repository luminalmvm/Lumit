// The order the Stagger panel takes the layers in.

import 'package:uuid/uuid.dart';

import '../state/workspace.dart';

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
