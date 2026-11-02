// Footage the browser build's page reads, which the desktop never has: it
// reads files off the disk by their paths. The browser build imports
// web_media_web.dart in this file's place.

/// The path the engine knows a browser file by, or null. Never one here.
Future<String?> openWebMedia(Object file) async => null;

/// Pick video and sound files in the browser. Nothing to pick here.
Future<List<String>> pickWebMedia() async => const [];
