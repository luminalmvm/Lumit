// Files are the operating system's everywhere but the browser build, which
// imports files_web.dart in this file's place.

import 'package:file_selector/file_selector.dart' show XFile;

Future<void> installFiles() async {}

Future<String> keepPicked(XFile file) async => file.path;

Future<String?> pickSaveTarget(String suggestedName) async => null;

Future<void> handOver(String path) async {}

bool importsHere(String path) => true;

Future<int?> storageUsed() async => null;

void warnBeforeLeaving(bool Function() unsaved) {}

Future<List<String>?> pickFilesHere(
        String label, List<String> extensions) async =>
    null;

Future<int> reopenFootage(String project) async => 0;

Future<List<String>> allowFootage() async => const [];

Future<void> forgetUnused(List<String> projects) async {}
