// The Ctrl+Space console's snapshot (K-324): what the camera button writes,
// and where it lands.
//
// The radial ring the console once raised — and everything this file built
// for it — went with K-658 (owner's ruling): the console is
// the search popover now, and every slice the ring offered lives on in the
// menus, the palette and the panels it was drawn from.
//
// This file is kept apart from `fx_console_frb.dart` so the console widget
// stays a thing that draws what it is given: the widget knows nothing about
// the document, and this is where the document knowledge lives.

import 'dart:io';
import 'dart:typed_data';

import '../main.dart';
import '../src/rust/api/export.dart';
import 'status_line_frb.dart';

/// Write the frame on screen to a PNG (K-324).
///
/// It is a one-frame **export**, not a new engine path: the exporter already
/// writes PNGs (`codec: 'png'`, K-201) and is the tested way a Lumit frame
/// becomes a file, so a snapshot is that with the range set to the playhead
/// and the frame after it. The alternative — a second still-writer beside the
/// exporter — is a second thing to keep correct about colour and size for no
/// gain. The engine numbers an image sequence `<stem>.00001.png`, so the frame
/// number lands in the file name whatever this passes.
///
/// Nothing is reported here beyond what the status line already says: the
/// strip polls the exporter and shows the finished path itself, which is the
/// same feedback any other export gives.
void saveSnapshotFrb(LumitState app, LumitUiState ui) {
  final comp = ui.selectedComp;
  if (comp == null) return;
  final path = snapshotPathFor(
    compName: comp.getSettings().name,
    projectPath: app.project?.path(),
  );
  try {
    comp.startExport(
      // Every field of the spec, because a snapshot is an export like any
      // other and the seam carries the whole of one (K-485): the composition's
      // own frame and rate, one frame of range, no sound, and every picture
      // and render setting at what an ordinary export takes.
      spec: BridgeExportSpec(
        preset: '',
        codec: 'png',
        width: 0,
        height: 0,
        bitrateMbps: 0,
        peakMbps: 0,
        bitrateAuto: false,
        fps: 0,
        rangeStartFrame: ui.playheadFrame.value,
        rangeEndFrame: ui.playheadFrame.value + 1,
        includeAudio: false,
        audioBitRate: 0,
        depth: 8,
        alphaChannel: false,
        straightAlpha: false,
        colourSpace: '',
        cropTop: 0,
        cropLeft: 0,
        cropBottom: 0,
        cropRight: 0,
        useRegionOfInterest: false,
        region: Float64List(0),
        metadata: const [],
        qualityDivisor: 1,
        diskCacheReadOnly: false,
        effects: true,
        honourSolo: true,
        makeANoise: false,
        openFolder: false,
      ),
      path: path,
    );
    // Wake the status line, which polls only while an export is live.
    statusLineExportStarted.value++;
  } on Object {
    // An export already running is the everyday refusal, and a calm one: the
    // status line is already showing that export's progress, which answers
    // the question better than a window over a console just dismissed.
  }
}

/// Where a snapshot goes: a `Snapshots` folder beside the saved project, so
/// the stills of a job live with the job. An unsaved project has nowhere of
/// its own, so those land in the user's pictures folder instead — never in
/// whatever directory the application happens to have been started from,
/// which is where a bare file name would put them.
///
/// The composition's name is the file's, with anything a file name cannot
/// carry taken out; the engine appends the frame number.
String snapshotPathFor({
  required String compName,
  String? projectPath,
  Map<String, String>? environment,
}) {
  final env = environment ?? Platform.environment;
  final sep = Platform.pathSeparator;
  final safe = compName.replaceAll(RegExp(r'[^A-Za-z0-9 _-]'), '').trim();
  final stem = safe.isEmpty ? 'snapshot' : safe;

  if (projectPath != null && projectPath.trim().isNotEmpty) {
    final cut = projectPath.lastIndexOf(RegExp(r'[/\\]'));
    if (cut > 0) {
      return '${projectPath.substring(0, cut)}${sep}Snapshots$sep$stem.png';
    }
  }
  final home = env['USERPROFILE'] ?? env['HOME'] ?? '.';
  return '$home${sep}Pictures${sep}Lumit$sep$stem.png';
}
