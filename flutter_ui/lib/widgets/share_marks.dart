// How a row says that someone else in a shared project has it in hand: a
// bar of their colour down its leading edge, shared out when it is several
// people, and a tint of the first. The Timeline's property rows and every row
// of the Effect controls panel draw it the same way.
//
// None of this is in the tree while the project is not shared. A panel hands
// its rows a [ShareRows] that says so, and the rows come back as they were
// made.

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import 'controls/base.dart' show ThemeScope;

/// The bar down a row's leading edge, one band for each person who has the
/// row in hand, and with [tint] a wash of the first person's colour over the
/// whole row.
class TheirBar extends Decoration {
  const TheirBar(this.colours, {this.tint = 0, this.width = 2});

  final List<Color> colours;

  /// How strong the wash over the row is, 0 for none.
  final double tint;
  final double width;

  @override
  BoxPainter createBoxPainter([VoidCallback? onChanged]) =>
      _TheirBarPainter(this);

  @override
  bool operator ==(Object other) =>
      other is TheirBar &&
      other.tint == tint &&
      other.width == width &&
      listEquals(other.colours, colours);

  @override
  int get hashCode => Object.hash(tint, width, Object.hashAll(colours));
}

class _TheirBarPainter extends BoxPainter {
  _TheirBarPainter(this.bar);

  final TheirBar bar;

  @override
  void paint(Canvas canvas, Offset offset, ImageConfiguration configuration) {
    final size = configuration.size;
    if (size == null || bar.colours.isEmpty) return;
    final paint = Paint();
    if (bar.tint > 0) {
      paint.color = bar.colours.first.withValues(alpha: bar.tint);
      canvas.drawRect(offset & size, paint);
    }
    final band = size.height / bar.colours.length;
    for (var i = 0; i < bar.colours.length; i++) {
      paint.color = bar.colours[i];
      canvas.drawRect(
        Rect.fromLTWH(offset.dx, offset.dy + band * i, bar.width, band),
        paint,
      );
    }
  }
}

/// What a panel tells its rows about the shared project: which rows other
/// people have in hand, and where to say that this person has touched one.
///
/// Rows are named by the path the Timeline gives them, so a row taken in
/// hand in one panel is marked in the other. A row with no Timeline path is
/// named by its section and its place in it.
///
/// Always above the rows, and saying nothing while the project is not
/// shared: [active] is false, and a row built under it is the row alone.
class ShareRows extends InheritedWidget {
  const ShareRows({
    super.key,
    required this.marks,
    required this.onTouch,
    required super.child,
  });

  /// By row name, the colour of each other person who has that row in hand.
  final Map<String, List<int>> marks;

  /// Told when this person presses on a row. Null while the project is not
  /// shared.
  final ValueChanged<String>? onTouch;

  bool get active => onTouch != null;

  static ShareRows? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<ShareRows>();

  /// The colours of everyone who has [name] in hand, or anything under it:
  /// what a heading shows for the rows inside it.
  List<int> under(String name) {
    if (marks.isEmpty) return const [];
    final out = <int>[];
    for (final held in marks.entries) {
      if (held.key != name && !held.key.startsWith('$name/')) continue;
      for (final colour in held.value) {
        if (!out.contains(colour)) out.add(colour);
      }
    }
    return out;
  }

  /// [child] as the row called [name]: pressing it says so, and it is drawn
  /// in the colours of whoever else has it. [also] is a second name the same
  /// row answers to, for a row that shows two parameters. While the project
  /// is not shared this is [child] itself.
  Widget row(String name, Widget child, {String? also}) {
    final touched = onTouch;
    if (touched == null) return child;
    final colours = <int>[
      ...?marks[name],
      if (also != null)
        for (final colour in marks[also] ?? const <int>[])
          if (!(marks[name]?.contains(colour) ?? false)) colour,
    ];
    return SharedRow(
      name: name,
      colours: colours,
      onTouch: touched,
      child: child,
    );
  }

  @override
  bool updateShouldNotify(ShareRows old) =>
      active != old.active || !identical(marks, old.marks);
}

/// One named row of a panel in a shared project. Only made while it is
/// shared ([ShareRows.row]).
class SharedRow extends StatelessWidget {
  const SharedRow({
    super.key,
    required this.name,
    required this.colours,
    required this.onTouch,
    required this.child,
  });

  final String name;
  final List<int> colours;
  final ValueChanged<String> onTouch;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Listener(
      // Watches the press and takes nothing from the row's own controls.
      behavior: HitTestBehavior.translucent,
      onPointerDown: (_) => onTouch(name),
      // The same box whether or not anyone has the row, so a mark arriving
      // does not rebuild the row from nothing under a value being typed.
      child: DecoratedBox(
        position: DecorationPosition.foreground,
        decoration: colours.isEmpty
            ? const BoxDecoration()
            : TheirBar([for (final c in colours) t.personColour(c)], tint: 0.14),
        child: child,
      ),
    );
  }
}
