// Files are the operating system's everywhere but the browser build, which
// imports files_web.dart in this file's place.

import 'package:file_selector/file_selector.dart' show XFile;

/// Nothing to put in place: `dart:io` has the machine's own files.
Future<void> installFiles() async {}

/// Where a picked or dropped file is: where it already was.
Future<String> keepPicked(XFile file) async => file.path;

/// Never asked on a desktop, whose own dialogue asks (file_dialogs.dart).
Future<String?> pickSaveTarget(String suggestedName) async => null;

/// Nothing to hand over: a saved file is already on the person's disk.
Future<void> handOver(String path) async {}

/// Every file imports here, since FFmpeg reads it.
bool importsHere(String path) => true;
