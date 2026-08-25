// How often a live drag is allowed to ask the engine for a preview frame, and
// — the part that matters — what happens to the ticks in between.
//
// In plain terms: a mouse drag produces far more positions than any renderer
// can draw. Something has to decide which ones to ask for. The obvious way is
// "ignore anything that arrives too soon after the last one", and that is what
// this replaces, because it throws away the one tick that matters most: the
// last one before you let go. The pointer stops, the final delta is dropped
// because it came 3 ms after its predecessor, and the picture sits one step
// behind the mouse until the release commits and a fresh frame is asked for.
// That is the stutter — the preview always showing where the pointer *was*.
//
// So: send the first tick at once, and while the interval is running hold the
// newest tick and send *that* when it is up. Nothing is dropped without a
// replacement already in hand, and the request rate stays bounded. The engine
// coalesces its side too (`drain_to_newest` in the worker), so at most one
// superseded render is ever in flight.

import 'dart:async';

/// A coalescing rate limit for live preview renders: the first tick goes at
/// once, the newest of the ticks that follow goes when [interval] is up.
class PreviewThrottle {
  /// The shortest gap between two preview requests. 20 ms — about one render on
  /// a warm comp, and fast enough that the picture reads as following the
  /// pointer rather than catching up with it.
  static const Duration defaultInterval = Duration(milliseconds: 20);

  final Duration interval;

  /// Running between a send and the earliest the next one may go out. Null
  /// means the next tick can go immediately.
  Timer? _timer;

  /// The newest tick that arrived while the interval was running.
  void Function()? _pending;

  PreviewThrottle({this.interval = defaultInterval});

  /// True while a tick is held — the drag has asked for something not yet sent.
  bool get holding => _pending != null;

  /// Ask for a preview. [send] is called now, or at the end of the interval if
  /// it is still the newest by then — so build the request *inside* it, never
  /// before, or a held tick will send a stale one.
  void request(void Function() send) {
    if (_timer == null) {
      _send(send);
      return;
    }
    // Too soon: hold it, replacing any earlier held tick. A superseded position
    // is the only thing ever dropped, and its replacement is already here.
    _pending = send;
  }

  void _send(void Function() send) {
    send();
    _timer = Timer(interval, _fire);
  }

  void _fire() {
    _timer = null;
    final send = _pending;
    _pending = null;
    if (send != null) _send(send);
  }

  /// Send any held tick now rather than waiting out the interval.
  void flush() {
    final send = _pending;
    if (send == null) return;
    _pending = null;
    _timer?.cancel();
    _send(send);
  }

  /// Forget a held tick — a cancelled gesture, a commit that supersedes it, or
  /// a widget going away.
  void cancel() {
    _timer?.cancel();
    _timer = null;
    _pending = null;
  }
}
