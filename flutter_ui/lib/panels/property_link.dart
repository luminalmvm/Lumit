// Linking a property by dragging: a small mark on a property row, dragged
// onto another row, writes an expression that follows it.
//
// The drag starts on the row that will hold the expression and lands on the
// row it should read, in any panel. The drop writes the reference in the
// row's language, or in the one that can say it, and opens the editor with
// the caret after it, so `* 2` is the next thing typed. A drop on a layer's
// name row writes the layer alone. The expression editor's own mark drags
// the other way round: onto a row, and the reference lands at the caret.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:uuid/uuid.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/expression_language.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart' show newExpressionLanguage;
import 'fx_section.dart' show FxRowHover;

/// What Copy link on a row's menu put down: the row, waiting to be named from
/// wherever Paste link is pressed.
PropertyRef? Function(PropertyLink link)? copiedLink;

/// What a link drag carries: where it began, so the drop can write there.
class PropertyLink {
  /// The layer the drag began on, for the short form of a reference.
  final UuidValue layer;

  /// The language the expression will be in, or null for a fresh one, which
  /// takes the language that can say the reference.
  final BridgeExpressionLanguage? language;

  /// Write [reference] as the row's expression in [language] and open its
  /// editor: a drag from a row.
  final void Function(String reference, BridgeExpressionLanguage language)?
      write;

  /// Put [reference] at the caret: a drag from the editor's own mark.
  final void Function(String reference)? insert;

  /// The rows the drag began on, so a drop back on one of them is refused
  /// rather than writing a row that reads itself.
  final Set<Object> from;

  const PropertyLink({
    required this.layer,
    this.language,
    this.write,
    this.insert,
    this.from = const {},
  });

  /// The text a drop on [ref] writes, and its language, or null when no
  /// language the row can take says it.
  (String, BridgeExpressionLanguage)? textFor(
      PropertyRef ref, BridgeExpressionLanguage fresh) {
    final held = language;
    if (held != null) {
      final text = referenceText(ref, held);
      return text == null ? null : (text, held);
    }
    for (final lang in [fresh, BridgeExpressionLanguage.javaScript]) {
      final text = referenceText(ref, lang);
      if (text != null) return (text, lang);
    }
    return null;
  }
}

/// The mark a link is dragged from. On a row it shows while the row is
/// hovered; in the editor's footer it is always there.
class LinkMark extends StatelessWidget {
  final PropertyLink link;

  /// What the drag shows under the pointer.
  final String name;
  final bool always;
  const LinkMark(
      {super.key, required this.link, required this.name, this.always = false});

  @override
  Widget build(BuildContext context) {
    final over = always
        ? null
        : context.dependOnInheritedWidgetOfExactType<FxRowHover>()?.over;
    if (over == null) return _mark(context, true);
    // Laid out whether or not it shows, so the row's columns never move:
    // the glyph goes clear rather than the row losing it.
    return ValueListenableBuilder<bool>(
      valueListenable: over,
      builder: (context, lit, _) => _mark(context, lit),
    );
  }

  Widget _mark(BuildContext context, bool shown) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: l10n.tipLinkProperty,
      child: HoverMark(
        cursor: SystemMouseCursors.grab,
        builder: (context, lit) => Draggable<PropertyLink>(
          data: link,
          dragAnchorStrategy: pointerDragAnchorStrategy,
          feedback: FloatSurface(
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  glyph.LumitIcon(LumitIcons.link, size: 14, colour: t.accent),
                  const SizedBox(width: 6),
                  Text(name, style: t.small),
                ],
              ),
            ),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 4),
            child: glyph.LumitIcon(LumitIcons.link,
                size: 14,
                colour: Color.lerp(t.textSecondary, t.textPrimary, lit)!
                    .withValues(alpha: shown ? 1 : 0)),
          ),
        ),
      ),
    );
  }
}

/// A row a link can land on. [ref] says what the row is, worked out at the
/// drop; null for a row nothing can name.
class LinkDrop extends StatelessWidget {
  final PropertyRef? Function(PropertyLink link) ref;
  final Widget child;
  const LinkDrop({super.key, required this.ref, required this.child});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final fresh = newExpressionLanguage(context);
    (String, BridgeExpressionLanguage)? textOf(PropertyLink link) {
      final at = ref(link);
      return at == null ? null : link.textFor(at, fresh);
    }

    return DragTarget<PropertyLink>(
      onWillAcceptWithDetails: (d) => textOf(d.data) != null,
      onAcceptWithDetails: (d) {
        final link = d.data;
        final got = textOf(link);
        if (got == null) return;
        final (text, language) = got;
        if (link.insert case final insert?) {
          insert(text);
        } else {
          link.write?.call(text, language);
        }
      },
      builder: (context, candidate, _) => candidate.isEmpty
          ? child
          : DecoratedBox(
              position: DecorationPosition.foreground,
              decoration: BoxDecoration(
                color: t.accent.withValues(alpha: 0.12),
                border: Border.all(color: t.accent),
              ),
              child: child,
            ),
    );
  }
}
