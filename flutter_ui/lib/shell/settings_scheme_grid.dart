// The colour schemes as a grid of small pictures, in three groups.
//
// A dropdown of thirty-odd names showed no scheme and asked for a choice by
// name alone. Each tile here is the window drawn in that scheme under the
// style in force, with the name under it, so a scheme can be picked by how it
// looks. Dark, Light and the user's own each have a heading.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'first_run_frb.dart' show windowThumbnail;

/// One tile's width, and the gap between tiles. Six fit across the window as it
/// opens.
const double _tileWidth = 96;
const double _tileGap = 8;
const double _tilePadding = 5;

/// Built-in schemes already drawn under a style, so a rebuild of the page does
/// not build thirty themes again.
final Map<(LumitColorScheme, ThemeShape), LumitTheme> _built = {};

LumitTheme _themeOf(LumitColorScheme scheme, ThemeShape shape) => _built
    .putIfAbsent((scheme, shape), () => LumitTheme.forScheme(scheme, shape));

/// The grid. [workspace] is read for the schemes and the choice in force.
class SchemeGrid extends StatelessWidget {
  final Workspace workspace;
  final ValueChanged<ThemeChoice> onChoose;

  const SchemeGrid({
    super.key,
    required this.workspace,
    required this.onChoose,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final shape = t.shape;
    final chosen = workspace.themeChoice;

    Widget group(String title, List<Widget> tiles) => Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Padding(
              padding: const EdgeInsets.only(top: 10, bottom: 8),
              child: Text(title,
                  style: t.bodyStrong.copyWith(color: t.textSecondary)),
            ),
            Wrap(spacing: _tileGap, runSpacing: _tileGap, children: tiles),
          ],
        );

    Widget builtIn(LumitColorScheme s) {
      final choice = ThemeChoice.builtIn(s);
      return _SchemeTile(
        id: 'settings-scheme-${s.name}',
        name: s.label,
        preview: _themeOf(s, shape),
        chosen: choice == chosen,
        onTap: () => onChoose(choice),
      );
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        group(l10n.schemeDark, [
          for (final s in LumitColorScheme.values)
            if (s.mode == ThemeMode2.dark) builtIn(s),
        ]),
        group(l10n.schemeLight, [
          for (final s in LumitColorScheme.values)
            if (s.mode == ThemeMode2.light) builtIn(s),
        ]),
        if (workspace.customThemes.isNotEmpty)
          group(l10n.custom, [
            for (final c in workspace.customThemes)
              _SchemeTile(
                id: 'settings-scheme-custom-${c.name}',
                name: c.name,
                preview: c.build(shape),
                chosen: ThemeChoice.custom(c.name) == chosen,
                onTap: () => onChoose(ThemeChoice.custom(c.name)),
              ),
          ]),
        const SizedBox(height: 4),
      ],
    );
  }
}

/// One scheme: its picture and its name. The one in force is edged in the
/// accent.
class _SchemeTile extends StatelessWidget {
  final String id;
  final String name;
  final LumitTheme preview;
  final bool chosen;
  final VoidCallback onTap;

  const _SchemeTile({
    required this.id,
    required this.name,
    required this.preview,
    required this.chosen,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final corner = BorderRadius.circular(t.tokens.controlRadius <= 0
        ? 0
        : (t.tokens.controlRadius < 6 ? 6 : t.tokens.controlRadius));
    return SizedBox(
      width: _tileWidth,
      child: HoverFill(
        radius: corner,
        child: GestureDetector(
          key: ValueKey<String>(id),
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          child: Container(
            padding: const EdgeInsets.all(_tilePadding),
            decoration: BoxDecoration(
              color: chosen ? t.surface3 : null,
              borderRadius: corner,
              border: Border.all(
                color: chosen ? t.accent : t.hairline,
                width: chosen ? 1.5 : 1,
              ),
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                windowThumbnail(preview),
                const SizedBox(height: 5),
                Text(
                  name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  textAlign: TextAlign.center,
                  style: chosen
                      ? t.small.copyWith(
                          color: t.textPrimary, fontWeight: FontWeight.w500)
                      : t.small,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
