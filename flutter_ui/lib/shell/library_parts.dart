// The small pieces the Library window is drawn from: what a kind and a state
// are called, an item's preview tile, the disc per computer, the mark an
// exception wears, and the storage bar. Kept apart from the window so the
// profile card and Settings can say the library's status in the same words.

import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data' show ByteData, Uint8List;

import 'package:flutter/services.dart' show FontLoader;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/template/library.dart'
    show templatePoster;

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../panels/effect_param_row_frb.dart' show effectLabelOf;
import '../state/library.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'pro_window.dart' show writtenBytes;

/// What a kind is called in the sidebar and the headers. By name, so the
/// state can grow a kind before the window has words for it.
String libraryKindLabel(LibraryKind kind) {
  switch (kind.name) {
    case 'presets':
      return l10n.libraryKindPresets;
    case 'styles':
      return l10n.libraryKindStyles;
    case 'themes':
      return l10n.libraryKindThemes;
    case 'fonts':
      return l10n.libraryKindFonts;
    case 'shortcuts':
      return l10n.libraryKindShortcuts;
    case 'lenses':
      return l10n.libraryKindLenses;
    case 'flares':
      return l10n.libraryKindFlares;
    case 'exports':
      return l10n.libraryKindExports;
    case 'luts':
      return l10n.libraryKindLuts;
    case 'templates':
      return l10n.libraryKindTemplates;
  }
  return kind.name;
}

/// Whether a kind has a row in the sidebar: every kind but templates, which
/// is listed only once there are some.
bool libraryKindListed(LibraryKind kind, int count) =>
    kind.name != 'templates' || count > 0;

/// An item's detail line. A shortcut set's, a flare's and the export
/// presets' come as a bare count, and are written out here.
String libraryDetail(LibraryEntry e) => libraryDetailOf(e.kind, e.detail);

/// The same for a [detail] line of [kind], as a conflict's other side has.
String libraryDetailOf(LibraryKind kind, String detail) {
  // A preset's line is its effects, kept by their ids and said by name.
  if (kind.name == 'presets' && detail.isNotEmpty) {
    return [for (final id in detail.split(', ')) effectLabelOf(id)].join(', ');
  }
  final n = int.tryParse(detail);
  if (n == null) return detail;
  return switch (kind.name) {
    'shortcuts' => l10n.libraryBindings(n),
    'flares' => l10n.libraryElements(n),
    'exports' => l10n.libraryEntries(n),
    _ => detail,
  };
}

/// The last part of an item's path: its file, or its folder for a style.
String libraryFileName(LibraryEntry e) => e.path.split('/').last;

/// What Undo or Redo would do, from the state's `<action>:<names>` label:
/// "Undo delete · Teal grade".
String libraryUndoWords(String label, {required bool redo}) {
  final cut = label.indexOf(':');
  final action = cut < 0 ? label : label.substring(0, cut);
  final names = cut < 0 ? '' : label.substring(cut + 1).trim();
  final verb =
      redo ? l10n.libraryRedoAction(action) : l10n.libraryUndoAction(action);
  return names.isEmpty ? verb : '$verb · $names';
}

/// How a status line is coloured: nothing to say, all well, under way, or
/// something for the person to look at.
enum LibraryTone { quiet, good, busy, warn }

/// The one line the library has to say about itself, for the title strip,
/// the profile card and Settings. Only the fact: "In sync", "Offline · 3
/// waiting", "Needs you · 1".
(String, LibraryTone) libraryStatusWords(LibraryState library) {
  if (!library.syncing) return (l10n.libraryStatusLocal, LibraryTone.quiet);
  final conflicts = library.conflicts.length;
  return switch (library.status) {
    LibraryStatus.syncing => (l10n.syncStatusSyncing, LibraryTone.busy),
    _ when conflicts > 0 => (
        l10n.libraryStatusNeedsYou(conflicts),
        LibraryTone.warn
      ),
    LibraryStatus.offline => (
        library.waiting > 0
            ? l10n.libraryStatusOfflineWaiting(library.waiting)
            : l10n.libraryStatusOffline,
        LibraryTone.quiet
      ),
    LibraryStatus.failed => (l10n.libraryStatusFailed, LibraryTone.warn),
    LibraryStatus.off => (l10n.libraryStatusLocal, LibraryTone.quiet),
    // Not in sync while something here cannot go up: said, not hidden.
    LibraryStatus.idle when library.entries
            .any((e) => e.state == LibraryItemState.noRoom) =>
      (l10n.libraryStateNoRoom, LibraryTone.warn),
    LibraryStatus.idle when library.entries
            .any((e) => e.state == LibraryItemState.tooBig) =>
      (l10n.libraryStateTooBig, LibraryTone.warn),
    LibraryStatus.idle => (l10n.libraryStatusInSync, LibraryTone.good),
  };
}

/// The colour a tone reads in.
Color libraryToneColour(LumitTheme t, LibraryTone tone) => switch (tone) {
      LibraryTone.quiet => t.textMuted,
      LibraryTone.good => t.textSecondary,
      LibraryTone.busy => t.textSecondary,
      LibraryTone.warn => t.warning,
    };

/// A state said in words, for a tooltip and the detail pane. Null for the
/// ordinary synced state, which wears no mark and needs no words.
String? libraryStateWords(LibraryItemState state) => switch (state) {
      LibraryItemState.synced => null,
      LibraryItemState.waiting => l10n.libraryStateWaiting,
      LibraryItemState.coming => l10n.libraryStateComing,
      LibraryItemState.notHere => l10n.libraryStateNotHere,
      LibraryItemState.onlyHere => l10n.libraryStateOnlyHere,
      LibraryItemState.conflict => l10n.libraryNeedsYou,
      LibraryItemState.tooBig => l10n.libraryStateTooBig,
      LibraryItemState.noRoom => l10n.libraryStateNoRoom,
    };

/// An item's own words: why it is not here when its name is the reason,
/// else its state's. Null for the ordinary synced state.
String? libraryEntryWords(LibraryEntry e) =>
    e.cannotWrite ? l10n.libraryCannotWrite : libraryStateWords(e.state);

/// How a computer stands with an item: filled, it has the newest; a ring, an
/// older one or one on its way; dashed, not kept there.
enum LibraryHold { newest, older, none }

LibraryHold libraryHoldOf(LibraryEntry e, LibraryMachine m) {
  final held = e.held[m.id] ?? 0;
  final head = e.head?.n ?? 0;
  if (m.me) {
    return switch (e.state) {
      LibraryItemState.notHere => LibraryHold.none,
      LibraryItemState.coming ||
      LibraryItemState.waiting ||
      LibraryItemState.conflict =>
        LibraryHold.older,
      _ => e.here ? LibraryHold.newest : LibraryHold.none,
    };
  }
  if (held == 0) return LibraryHold.none;
  return held >= head ? LibraryHold.newest : LibraryHold.older;
}

/// The same in words, for the Where section and a disc's tooltip.
String libraryWhereWords(LibraryEntry e, LibraryMachine m) {
  if (m.me) return libraryEntryWords(e) ?? l10n.libraryWhereUpToDate;
  return switch (libraryHoldOf(e, m)) {
    LibraryHold.newest => l10n.libraryWhereUpToDate,
    LibraryHold.older => l10n.libraryWhereOlder,
    LibraryHold.none => l10n.libraryWhereNotKept,
  };
}

/// A computer's name as the window says it: this one is "This computer".
String libraryMachineName(LibraryMachine m) =>
    m.me ? l10n.libraryThisComputer : m.name;

/// When a computer was last seen, short: now, 12 min, 2 h, 3 d.
String librarySeen(LibraryMachine m) {
  if (m.online || m.me) return l10n.librarySeenNow;
  final seen = m.seen;
  if (seen == null) return '';
  final gap = DateTime.now().difference(seen);
  if (gap.inMinutes < 60) return l10n.libraryAgoMinutes(math.max(1, gap.inMinutes));
  if (gap.inHours < 24) return l10n.libraryAgoHours(gap.inHours);
  return l10n.libraryAgoDays(gap.inDays);
}

/// The glyph an item wears when it has no picture of its own: its kind's, or
/// the node graph's for a saved node group.
String libraryGlyph(LibraryEntry e) =>
    e.path.toLowerCase().endsWith('.lumngrp') ? LumitIcons.nodes : libraryKindGlyph(e.kind);

/// The glyph a kind wears.
String libraryKindGlyph(LibraryKind kind) => switch (kind.name) {
      'presets' => LumitIcons.effectsSwitch,
      'styles' => LumitIcons.label,
      'themes' => LumitIcons.colourSpace,
      'fonts' => LumitIcons.text,
      'shortcuts' => LumitIcons.switches,
      'lenses' => LumitIcons.camera,
      'flares' => LumitIcons.exposure,
      'exports' => LumitIcons.export,
      'luts' => LumitIcons.channels,
      _ => LumitIcons.composition,
    };

// --- Fonts ----------------------------------------------------------------

final Map<String, Future<String?>> _loadedFonts = {};

/// The family name a font file is registered under for previews, once it is
/// loaded. Each file is loaded once a [modified] time and kept for the run, so
/// a font a sync replaced shows its new face; null when the file would not
/// load.
Future<String?> libraryFontFamily(String path, DateTime? modified) {
  final key = '$path|${modified?.millisecondsSinceEpoch}';
  return _loadedFonts[key] ??= _loadFont(path, key);
}

Future<String?> _loadFont(String path, String key) async {
  final family = 'lumit-library-${key.hashCode}';
  try {
    final bytes = await File(path).readAsBytes();
    final loader = FontLoader(family)
      ..addFont(Future.value(ByteData.view(bytes.buffer)));
    await loader.load();
    return family;
  } catch (_) {
    return null;
  }
}

/// "Aa" in [path]'s own face once it has loaded, and in the interface's face
/// until then. [sample] is the longer text the detail pane shows.
class LibraryFontSample extends StatelessWidget {
  final String? path;
  final DateTime? modified;
  final String sample;
  final TextStyle style;
  const LibraryFontSample(
      {super.key,
      required this.path,
      this.modified,
      required this.sample,
      required this.style});

  @override
  Widget build(BuildContext context) {
    final at = path;
    final plain = Text(sample,
        style: style, maxLines: 1, overflow: TextOverflow.ellipsis);
    if (at == null) return plain;
    return FutureBuilder<String?>(
      future: libraryFontFamily(at, modified),
      builder: (context, loaded) {
        final family = loaded.data;
        if (family == null) return plain;
        return Text(sample,
            style: style.copyWith(fontFamily: family),
            maxLines: 1,
            overflow: TextOverflow.ellipsis);
      },
    );
  }
}

// --- Template posters -------------------------------------------------------

final Map<String, Future<Uint8List?>> _posters = {};

/// The poster a template file carries, read once a [modified] time, or null
/// when it has none or will not read.
Future<Uint8List?> libraryPoster(String path, DateTime? modified) {
  final key = '$path|${modified?.millisecondsSinceEpoch}';
  // A long session keeps the last few, not every version it ever showed.
  if (_posters.length > 64 && !_posters.containsKey(key)) _posters.clear();
  return _posters[key] ??= () async {
    try {
      final bytes = await templatePoster(path: path);
      return bytes.isEmpty ? null : bytes;
    } catch (_) {
      return null;
    }
  }();
}

/// A template's poster filling its box, or [fallback] until it is read or
/// when there is none.
class LibraryPoster extends StatelessWidget {
  final String path;
  final DateTime? modified;
  final Widget fallback;
  const LibraryPoster(
      {super.key, required this.path, this.modified, required this.fallback});

  @override
  Widget build(BuildContext context) => FutureBuilder<Uint8List?>(
        future: libraryPoster(path, modified),
        builder: (context, read) {
          final bytes = read.data;
          if (bytes == null) return fallback;
          return Image.memory(bytes,
              fit: BoxFit.cover, gaplessPlayback: true, filterQuality: FilterQuality.medium);
        },
      );
}

// --- The tile -------------------------------------------------------------

/// An item's preview: a font's "Aa" in its own face, a theme's or style's
/// colours, or the kind's glyph.
class LibraryTile extends StatelessWidget {
  final LibraryEntry entry;
  final double size;
  const LibraryTile(this.entry, {super.key, this.size = 24});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final radius = BorderRadius.circular(t.tokens.controlRadius);
    final Widget face;
    if (entry.kind == LibraryKind.fonts) {
      face = LibraryFontSample(
        path: entry.pathHere,
        modified: entry.modified,
        sample: 'Aa',
        style: t.body.copyWith(
            color: t.textPrimary, fontSize: size * 0.5, height: 1),
      );
    } else if (entry.colours.isNotEmpty) {
      face = ClipRRect(
        borderRadius: BorderRadius.circular(
            math.max(0, t.tokens.controlRadius - 1)),
        child: LibrarySwatches(entry.colours),
      );
    } else if (entry.kind == LibraryKind.templates && entry.pathHere != null) {
      face = SizedBox.expand(
        child: LibraryPoster(
          path: entry.pathHere!,
          modified: entry.modified,
          fallback: Center(
            child: glyph.LumitIcon(libraryGlyph(entry),
                size: size * 0.58, colour: t.textSecondary),
          ),
        ),
      );
    } else {
      face = glyph.LumitIcon(libraryGlyph(entry),
          size: size * 0.58, colour: t.textSecondary);
    }
    return Container(
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: radius,
        border: Border.all(color: t.hairline),
      ),
      padding: entry.colours.isNotEmpty && entry.kind != LibraryKind.fonts
          ? const EdgeInsets.all(1)
          : EdgeInsets.zero,
      clipBehavior: Clip.antiAlias,
      child: face,
    );
  }
}

/// A theme's or style's colours as upright stripes, the first the widest.
class LibrarySwatches extends StatelessWidget {
  final List<int> colours;
  /// How many of the colours are drawn, the first one wider.
  final int count;
  const LibrarySwatches(this.colours, {super.key, this.count = 5});

  @override
  Widget build(BuildContext context) => SizedBox.expand(
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final (i, argb) in colours.take(count).indexed)
              Expanded(
                flex: i == 0 && count <= 5 ? 3 : 2,
                child: ColoredBox(color: Color(argb)),
              ),
          ],
        ),
      );
}

// --- Discs -----------------------------------------------------------------

/// One disc per computer: filled when it has the newest version, a ring for
/// an older one or one on its way, dashed when it is not kept there. Each
/// says its state in words on hover.
class LibraryDiscs extends StatelessWidget {
  final LibraryEntry entry;
  final List<LibraryMachine> machines;
  final double size;
  const LibraryDiscs(this.entry, this.machines, {super.key, this.size = 12});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final (i, m) in machines.indexed) ...[
          if (i > 0) const SizedBox(width: 3),
          LumitTooltip(
            message: '${libraryMachineName(m)} · ${libraryWhereWords(entry, m)}',
            child: LibraryDisc(
              hold: libraryHoldOf(entry, m),
              initial: _initial(m),
              size: size,
              theme: t,
            ),
          ),
        ],
      ],
    );
  }

  static String _initial(LibraryMachine m) {
    final name = m.name.trim();
    return name.isEmpty ? '' : String.fromCharCode(name.runes.first).toUpperCase();
  }
}

/// The disc itself, with its computer's initial in it.
class LibraryDisc extends StatelessWidget {
  final LibraryHold hold;
  final String initial;
  final double size;
  final LumitTheme theme;
  const LibraryDisc(
      {super.key,
      required this.hold,
      required this.initial,
      required this.size,
      required this.theme});

  @override
  Widget build(BuildContext context) {
    final t = theme;
    final ink = switch (hold) {
      LibraryHold.newest => t.textSecondary,
      LibraryHold.older => t.textSecondary,
      LibraryHold.none => t.textMuted,
    };
    return SizedBox(
      width: size,
      height: size,
      child: CustomPaint(
        painter: _DiscPainter(hold, ink, t.surface1),
        child: hold == LibraryHold.none
            ? null
            : Center(
                child: Text(
                  initial,
                  style: t.small.copyWith(
                    fontSize: size * 0.58,
                    height: 1,
                    fontWeight: FontWeight.w500,
                    color: hold == LibraryHold.newest ? t.surface1 : ink,
                  ),
                ),
              ),
      ),
    );
  }
}

class _DiscPainter extends CustomPainter {
  final LibraryHold hold;
  final Color ink, ground;
  const _DiscPainter(this.hold, this.ink, this.ground);

  @override
  void paint(Canvas canvas, Size size) {
    final box = Offset.zero & size;
    final pen = Paint()..color = ink;
    switch (hold) {
      case LibraryHold.newest:
        canvas.drawOval(box, pen);
      case LibraryHold.older:
        canvas.drawOval(
            box.deflate(0.6),
            pen
              ..style = PaintingStyle.stroke
              ..strokeWidth = 1.2);
      case LibraryHold.none:
        pen
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1;
        final r = size.width / 2 - 0.5;
        final c = box.center;
        const dashes = 6;
        for (var i = 0; i < dashes; i++) {
          final from = i * 2 * math.pi / dashes;
          canvas.drawArc(Rect.fromCircle(center: c, radius: r), from,
              math.pi / dashes, false, pen);
        }
    }
  }

  @override
  bool shouldRepaint(_DiscPainter old) =>
      old.hold != hold || old.ink != ink || old.ground != ground;
}

// --- Marks -----------------------------------------------------------------

/// The mark an exception wears at the end of its row, with its words on
/// hover. Nothing for the ordinary synced state. "Not on this computer" is
/// the one that is a button, since Get is what it wants.
class LibraryMark extends StatelessWidget {
  final LibraryEntry entry;
  final VoidCallback? onGet;

  /// What the Get button is keyed by, for the row it sits in.
  final String? getKey;
  const LibraryMark(this.entry, {super.key, this.onGet, this.getKey});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final state = entry.state;
    final words = libraryEntryWords(entry);
    if (words == null) return const SizedBox.shrink();
    // One kept out by its name cannot be fetched, so it wears the warning
    // rather than Get.
    if (state == LibraryItemState.notHere && onGet != null && !entry.cannotWrite) {
      return LumitTooltip(
        message: words,
        child: HouseButton(
          key: getKey == null ? null : ValueKey<String>(getKey!),
          small: true,
          padding: const EdgeInsets.symmetric(horizontal: 8),
          onPressed: onGet,
          child: Text(l10n.libraryGet, style: t.small),
        ),
      );
    }
    final Widget mark = switch (state) {
      LibraryItemState.waiting =>
        glyph.LumitIcon(LumitIcons.export, size: 12, colour: t.textSecondary),
      LibraryItemState.coming =>
        glyph.LumitIcon(LumitIcons.import, size: 12, colour: t.textSecondary),
      LibraryItemState.onlyHere =>
        glyph.LumitIcon(LumitIcons.solo, size: 12, colour: t.textSecondary),
      LibraryItemState.tooBig || LibraryItemState.noRoom =>
        glyph.LumitIcon(LumitIcons.missing, size: 12, colour: t.warning),
      LibraryItemState.notHere when entry.cannotWrite =>
        glyph.LumitIcon(LumitIcons.missing, size: 12, colour: t.warning),
      LibraryItemState.conflict => Container(
          width: 6,
          height: 6,
          decoration: BoxDecoration(color: t.warning, shape: BoxShape.circle),
        ),
      _ => const SizedBox.shrink(),
    };
    return LumitTooltip(
      message: words,
      child: SizedBox(width: 16, height: 16, child: Center(child: mark)),
    );
  }
}

// --- Storage ----------------------------------------------------------------

/// The footer's storage: a bar with the library's own share lit in the
/// accent and the rest of the account's use behind it, and the line that
/// says the numbers.
class LibraryStorage extends StatelessWidget {
  final ({int bytes, int limit, int library}) usage;
  const LibraryStorage(this.usage, {super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final limit = math.max(1, usage.limit);
    final used = (usage.bytes / limit).clamp(0.0, 1.0);
    final own = (usage.library / limit).clamp(0.0, used);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox(
          width: 120,
          height: 4,
          child: ClipRRect(
            borderRadius: BorderRadius.circular(2),
            child: Stack(
              children: [
                Positioned.fill(child: ColoredBox(color: t.hairline)),
                FractionallySizedBox(
                  widthFactor: used,
                  child: ColoredBox(color: t.hairlineStrong),
                ),
                FractionallySizedBox(
                  widthFactor: own,
                  child: ColoredBox(color: t.accent),
                ),
              ],
            ),
          ),
        ),
        const SizedBox(width: 10),
        Text(
          l10n.libraryStorageLine(
            writtenBytes(usage.library),
            writtenBytes(usage.bytes),
            writtenBytes(usage.limit),
          ),
          style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
          overflow: TextOverflow.ellipsis,
        ),
      ],
    );
  }
}
