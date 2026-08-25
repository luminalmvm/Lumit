// The one clock face: `HH:MM:SS:FF` at an exact rational rate.
//
// Shared by the Viewer's readout, the Project panel's info header and the
// Composition settings duration field, so a length reads the same everywhere.

/// `HH:MM:SS:FF` for `frame` at an exact rate.
///
/// The frame count per second is the rate *rounded up* — 29.97 fps counts 30
/// frames in a second of timecode, which is what every editor shows and what
/// makes 00:00:29:29 the last frame of a 30-second 29.97 comp rather than an
/// impossible one. The frames field widens with the rate: 600 fps counts to
/// :599, so it gets three digits rather than a lying two.
String timecodeOfRate(int frame, int fpsNum, int fpsDen) {
  final den = fpsDen == 0 ? 1 : fpsDen;
  final perSecond = (fpsNum / den).ceil().clamp(1, 1000);
  final total = frame < 0 ? 0 : frame;

  final frames = total % perSecond;
  final seconds = (total ~/ perSecond) % 60;
  final minutes = (total ~/ (perSecond * 60)) % 60;
  final hours = total ~/ (perSecond * 3600);

  final frameDigits = (perSecond - 1).toString().length.clamp(2, 4);
  String two(int v) => v.toString().padLeft(2, '0');
  return '${two(hours)}:${two(minutes)}:${two(seconds)}:'
      '${frames.toString().padLeft(frameDigits, '0')}';
}

/// How many digits the frames field of a clock face has at this rate — two up
/// to 99 fps, three to 999, four beyond. The readouts size their slots from
/// this, so a number never moves what is beside it as it counts.
int timecodeFrameDigits(int fpsNum, int fpsDen) {
  final den = fpsDen == 0 ? 1 : fpsDen;
  final perSecond = (fpsNum / den).ceil().clamp(1, 1000);
  return (perSecond - 1).toString().length.clamp(2, 4);
}

/// How many characters a clock face takes at this rate: `HH:MM:SS:` and its
/// frames field.
int timecodeChars(int fpsNum, int fpsDen) =>
    9 + timecodeFrameDigits(fpsNum, fpsDen);

/// `HH:MM:SS:FF` for a frame that may be **before zero**, written with a minus
/// sign — the clock face a Retime needs, where a source time earlier than the
/// start of the media is an ordinary thing to ask for (docs/04 §7).
String timecodeOfRateSigned(int frame, int fpsNum, int fpsDen) =>
    frame < 0
        ? '-${timecodeOfRate(-frame, fpsNum, fpsDen)}'
        : timecodeOfRate(frame, fpsNum, fpsDen);

/// [framesOfTimecode], accepting a leading minus for a time before zero.
int? framesOfTimecodeSigned(String text, int fpsNum, int fpsDen) {
  final trimmed = text.trim();
  final negative = trimmed.startsWith('-');
  final frames = framesOfTimecode(
      negative ? trimmed.substring(1) : trimmed, fpsNum, fpsDen);
  if (frames == null) return null;
  return negative ? -frames : frames;
}

/// `HH:MM:SS:mmm` for a length in seconds — the audio-only clock face, where
/// frames mean nothing: the last field is milliseconds.
String timecodeOfSecondsMs(double seconds) {
  final total = seconds.isFinite && seconds > 0 ? (seconds * 1000).round() : 0;
  final ms = total % 1000;
  final s = (total ~/ 1000) % 60;
  final m = (total ~/ 60000) % 60;
  final h = total ~/ 3600000;
  String two(int v) => v.toString().padLeft(2, '0');
  return '${two(h)}:${two(m)}:${two(s)}:${ms.toString().padLeft(3, '0')}';
}

/// `HH:MM:SS:FF` back to a whole frame count at an exact rate, or null when it
/// is not a timecode. Missing leading fields read the way everybody means
/// them: `SS`, `MM:SS`, `HH:MM:SS` — a frames field only exists when all four
/// are given.
int? framesOfTimecode(String text, int fpsNum, int fpsDen) {
  final parts = text.trim().split(':');
  if (parts.isEmpty || parts.length > 4 || parts.any((p) => p.isEmpty)) {
    return null;
  }
  final values = <int>[];
  for (final part in parts) {
    final v = int.tryParse(part);
    if (v == null || v < 0) return null;
    values.add(v);
  }
  final den = fpsDen == 0 ? 1 : fpsDen;
  final perSecond = (fpsNum / den).ceil().clamp(1, 1000);
  final frames = values.length == 4 ? values.removeLast() : 0;
  var seconds = 0;
  for (final v in values) {
    seconds = seconds * 60 + v;
  }
  return seconds * perSecond + frames;
}
