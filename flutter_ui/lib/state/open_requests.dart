// What Lumit is asked to open by the system once it is running, or before
// it has a window to open it in.

import 'package:flutter/services.dart';

/// The macOS runner's end of this is `OpenRequests` in AppDelegate.swift.
/// The Windows and Linux runners answer it too, for the open Lumit a second
/// start hands its arguments to.
const _openRequests = MethodChannel('lumit/open');

/// What macOS has asked Lumit to open and nobody has taken yet: a project
/// double-clicked in Finder as its path, an invite link clicked in a browser
/// as it was written. macOS hands these over as events and never as
/// arguments, so they are read as arguments once they are here, by
/// [projectPathFromArgs] and [inviteFromArgs]. Elsewhere it is what a second
/// start passed on, and empty at start-up.
Future<List<String>> takeOpenRequests() async {
  try {
    return await _openRequests.invokeListMethod<String>('take') ?? const [];
  } catch (_) {
    // No runner to ask, as in a widget test.
    return const [];
  }
}

/// Hand [open] whatever Lumit is asked to open from here on. A click that
/// finds Lumit running comes to it on macOS. On the other systems a package
/// always does, and the rest when Lumit is kept running in the background.
void listenForOpenRequests(void Function(List<String> asked) open) {
  Future<void> take() async {
    final asked = await takeOpenRequests();
    if (asked.isNotEmpty) open(asked);
  }

  // The runner says only that there is something, and it is taken.
  _openRequests.setMethodCallHandler((_) => take());
  // Once now as well, for a request that came after start-up took its own.
  take();
}

/// Call [asked] when something is waiting to be opened, leaving it for
/// whoever calls [listenForOpenRequests] next.
void noticeOpenRequests(VoidCallback asked) {
  _openRequests.setMethodCallHandler((_) async => asked());
}
