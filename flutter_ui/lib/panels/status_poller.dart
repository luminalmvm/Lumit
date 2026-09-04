// The sampling behind the long-job status cards — the Camera track, the Planar
// track and the Roto brush — in one copy.
//
// **In plain terms.** Following a shot or cutting a subject out of it takes a
// while, and it happens somewhere else: on its own thread, over the media file,
// while you carry on editing. The engine keeps how it is getting on as a
// *value* rather than a stream, so the card that reports it asks twice a second
// while something is moving and stops asking the moment it is not. All three
// cards did exactly that, in three copies down to the comments; this is the one
// copy, and each card keeps only the sentence it says.
//
// **Read, not subscribed to.** There is no stream to hold and nothing to
// unsubscribe: a press moves no document revision, so there is nothing to
// refresh against, and the engine keeps progress as a value precisely so nobody
// has to hold a subscription.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

/// How often the reading is sampled while a job is moving. Twice a second is
/// faster than anyone reads a number and slower than anything that could cost:
/// the call is a lookup in a map behind a mutex, and it is made only while the
/// card is open **and** a job is actually in flight.
const Duration statusPoll = Duration(milliseconds: 500);

/// The polling half of a status card, where [T] is the engine's reading.
///
/// Mix into the card's `State` and give it three answers — where the reading
/// comes from, whether it is still moving, and what a widget change means. The
/// lifecycle (first read after the frame, the timer, cancelling on dispose) and
/// what happens when a job lands are the same for every card, so they live
/// here.
mixin StatusPoller<T, W extends StatefulWidget> on State<W> {
  /// The last reading, or null before the first one.
  T? status;

  Timer? _timer;

  /// Ask the engine. Throwing means the layer went away under the card.
  T fetchStatus();

  /// Whether the reading is still moving, and so worth asking about again.
  bool isMoving(T? status);

  /// Something changed that the rest of the interface should re-read.
  VoidCallback get onChanged;

  /// Whether this widget change is a reason to read again, off the clock.
  bool shouldResample(W old);

  /// Whether this pair of readings is the job *landing*. By default it is: the
  /// answer was moving and has stopped.
  bool hasLanded(T? was, T next) => isMoving(was) && !isMoving(next);

  @override
  void initState() {
    super.initState();
    // The first reading after the frame is up, never from `build`.
    WidgetsBinding.instance.addPostFrameCallback((_) => sample());
  }

  @override
  void didUpdateWidget(W oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (shouldResample(oldWidget)) sample();
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  /// Read once, and keep or stop the clock by what came back.
  void sample() {
    if (!mounted) return;
    final T next;
    try {
      next = fetchStatus();
    } catch (_) {
      // The layer went away under the card; the line simply stops moving.
      _timer?.cancel();
      _timer = null;
      return;
    }
    final was = status;
    if (next != was) setState(() => status = next);
    if (hasLanded(was, next)) {
      onChanged();
      // Re-reading is not enough on its own. A job landing moves
      // neither the playhead nor the document's revision, so the picture would
      // still be the one banked before it, and the Viewer — keyed by exactly
      // those two — would have no reason to ask the engine again. Both are told
      // here, at the one place that knows a job has landed.
      final ui = Provider.of<LumitUiState>(context, listen: false);
      ui.solveLanded.value++;
      ui.requestFrame();
    }
    if (isMoving(next)) {
      _timer ??= Timer.periodic(statusPoll, (_) => sample());
    } else {
      _timer?.cancel();
      _timer = null;
    }
  }
}
