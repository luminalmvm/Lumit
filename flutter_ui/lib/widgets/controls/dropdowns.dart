// The dropdowns: the closed face they share, and the three open lists —
// eager, searchable and lazily built.

import 'package:flutter/material.dart';

import '../../l10n/strings.dart';
import '../../theme/theme.dart';
import 'base.dart';
import 'buttons.dart';
import 'menus.dart';
import 'popups.dart';
import 'text_field.dart';

/// The closed face all three bare dropdowns share: the label and the caret.
///
/// Ellipsised rather than allowed to overflow: a dropdown sits in whatever
/// width its caller has, and a label longer than that is a layout error the
/// user sees as striped tape. `Flexible` keeps the button intrinsic-width when
/// there is room, so nothing that fits changes shape.
///
/// [face] replaces the label with a mark of the caller's own — the Viewer
/// bar's channel picker, whose answer is a tinted glyph rather than a word. The
/// caret is the same one either way, so an icon dropdown still reads as a
/// dropdown.
Widget dropdownFace(LumitTheme t, String label, {Widget? face}) =>
    LayoutBuilder(builder: (context, c) {
      // In a cell too tight for even the caret and its gap (a fold-out value
      // column at its minimum), the caret is the first thing to go — a
      // sliver of the word still says more than striped overflow tape.
      final caretFits = !c.hasBoundedWidth || c.maxWidth >= 20;
      return Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          face ?? Flexible(child: Text(label, overflow: TextOverflow.ellipsis)),
          if (caretFits) ...[
            // 6, the gap every `.dd` the mockups compute leaves between its
            // label and its caret.
            const SizedBox(width: 6),
            // A small quiet mark: the border already says this is a control,
            // so the caret only has to say which kind (§12A, no raised look).
            CustomPaint(
              size: const Size(7, 7),
              painter: _CaretPainter(t.textMuted),
            ),
          ],
        ],
      );
    });

/// The **in-row** dropdown's label size (§12A.6's table): the pickers that sit
/// inside a Timeline row — matte, blend and parent — carry a 10px label, as the
/// approved mockups draw them. Their *height* is a density token
/// (`DensityTokens.inRowPicker`) because it is one of the handful of
/// measurements the Compact setting moves; the label size is not, and never
/// will be — Compact takes room out of rows, never legibility out of words.
const double inRowDropdownTextSize = 10;

/// The closed face of every bare dropdown, in its two sizes.
///
/// **Vertical 1 rather than the button default's 3**, so a label's descenders
/// are not clipped in a property row. The sum is tighter than it looks: a row
/// gives the button 18, the decoration's border insets the child by 1 top and
/// bottom, and body text at 11 carries a line box of about 13.3 — so the
/// padding has 3.4 to spend and 3 does not fit. At 1 the label has 14 to sit in
/// and centres there with room for the tails on p, q and g. Horizontal is the
/// button's own, so nothing moves sideways.
///
/// **Both heights are stated rather than left to the text**, because the
/// mockups' measurements are measurements and not consequences: a face that
/// grew out of its own font drifted every time the type did. [dense] is the
/// in-row face — the pickers inside a Timeline row — and the other is every
/// dropdown in a panel row or a bar. Both come from the density tokens, so the
/// Compact setting moves them together.
///
/// **Horizontal 6, not the button's 8**: every `.dd` the mockups compute pads
/// its label by exactly 6 either side, in both sizes.
Widget dropdownButton({
  required LumitTheme t,
  required bool dense,
  required VoidCallback? onPressed,
  required Widget face,
}) =>
    SizedBox(
      height: dense ? t.density.inRowPicker : t.density.dropdownFace,
      child: HouseButton(
        padding: EdgeInsets.symmetric(horizontal: 6, vertical: dense ? 0 : 1),
        onPressed: onPressed,
        dropdown: true,
        child: dense
            ? DefaultTextStyle.merge(
                style: const TextStyle(fontSize: inRowDropdownTextSize),
                child: face,
              )
            : face,
      ),
    );

/// A dropdown drawn as a bare label + caret; the open list floats on the
/// standard menu surface (`bare_dropdown` in the Rust settings window).
class BareDropdown<T> extends StatelessWidget {
  final T value;
  final List<T> options;
  final String Function(T) label;

  /// Null disables the control — the closed face still names the value, drawn
  /// in [HouseButton]'s own disabled style, and opens nothing. For a choice
  /// something else is currently making (the Viewer's resolution while
  /// adaptive playback picks the tier itself).
  final ValueChanged<T>? onChanged;

  /// The heading an option sits under, or null for none. Options keep their
  /// given order; a heading is drawn each time the answer changes, so a list
  /// that is already grouped needs nothing else, and one that is not gets no
  /// headings rather than a scrambled list.
  final String? Function(T)? group;

  /// A mark to show instead of the value's name on the closed face. The menu
  /// still lists [label]'s words, so nothing is lost by showing a glyph — see
  /// [dropdownFace].
  final Widget? face;

  /// The in-row face: 16 tall with a 10px label, for a picker that sits inside
  /// a Timeline row rather than in a dialog or a bar (§12A.6).
  final bool dense;

  /// Why an option cannot be chosen, or null where it can — the
  /// disabled-not-hidden rule inside a list. The row stays in the menu, drawn
  /// quiet, with the reason on hover; a list that removed it would leave the
  /// reader hunting for a name they know exists.
  final String? Function(T)? disabledReason;

  const BareDropdown({
    super.key,
    required this.value,
    required this.options,
    required this.label,
    required this.onChanged,
    this.group,
    this.face,
    this.dense = false,
    this.disabledReason,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return dropdownButton(
      t: t,
      dense: dense,
      onPressed: onChanged == null ? null : () => _open(context, t),
      face: dropdownFace(t, label(value), face: face),
    );
  }

  Future<void> _open(BuildContext context, LumitTheme t) async {
    final box = context.findRenderObject()! as RenderBox;
    final origin = box.localToGlobal(Offset.zero);
    // A one-item list rather than the value itself. The popup answers null when
    // it is dismissed, so for an option list that *contains* null — "System
    // default" on the Audio page, "Follow the machine" on General — choosing
    // that option and closing the menu were the same answer, and the option
    // could never be picked at all. Boxing keeps the two apart.
    final picked = await showLumitPopup<List<T>>(
      context: context,
      position: origin + Offset(0, box.size.height + 2),
      // IntrinsicWidth bounds the stretch: a float in the overlay has
      // unbounded width, and a stretched Column inside one otherwise
      // forces an infinite width (the settings-dropdown crash).
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (var i = 0; i < options.length; i++) ...[
                if (group != null &&
                    group!(options[i]) != null &&
                    (i == 0 || group!(options[i - 1]) != group!(options[i])))
                  Padding(
                    padding: EdgeInsets.fromLTRB(10, i == 0 ? 6 : 10, 10, 2),
                    child: Text(
                      group!(options[i])!,
                      style: t.small.copyWith(color: t.textMuted),
                    ),
                  ),
                if (disabledReason?.call(options[i]) case final why?)
                  LumitTooltip(
                    message: why,
                    child: MenuRow(
                      selected: options[i] == value,
                      onPressed: () {},
                      child: Text(label(options[i]),
                          style: TextStyle(color: t.textDisabled)),
                    ),
                  )
                else
                  MenuRow(
                    selected: options[i] == value,
                    onPressed: () => close([options[i]]),
                    child: Text(label(options[i])),
                  ),
              ],
            ],
          ),
        ),
      ),
    );
    if (picked != null) onChanged!(picked.single);
  }
}

/// Options at or above this count get [BareSearchDropdown] instead of the
/// plain [BareDropdown]. A plain dropdown builds every row eagerly inside an
/// `IntrinsicWidth`, which walks all of them twice — fine for the handful of
/// options every parameter has today — and fatal for the original Lens flare
/// library, whose 1299 rows took the app down in layout. The flare is a curated
/// twenty now; the guard stays.
const int searchableOptionThreshold = 40;

/// A dropdown for long option lists: a search field over a **lazily built**
/// list, with the group headings drawn inline.
///
/// The list is a `ListView.builder` inside a bounded box, so only the rows
/// on screen are ever built no matter how many options there are — the
/// difference between a thousand-row list being a feature and a crash.
class BareSearchDropdown extends StatelessWidget {
  final int value;
  final List<String> options;
  final ValueChanged<int> onChanged;

  /// The heading an option sits under, or null for none.
  final String? Function(String)? group;

  /// Placeholder for the search field — what the user is looking for. Null
  /// takes the plain word "Search", which is what most callers want.
  final String? hint;

  const BareSearchDropdown({
    super.key,
    required this.value,
    required this.options,
    required this.onChanged,
    this.group,
    this.hint,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final label = value >= 0 && value < options.length ? options[value] : '—';
    return HouseButton(
      // Vertical 1 rather than the button default's 3, so a label's descenders
      // are not clipped in a property row. The sum is tighter than it looks: a
      // row gives the button 18, the decoration's border insets the child by 1
      // top and bottom, and body text at 11 carries a line box of about 13.3 —
      // so the padding has 3.4 to spend and 3 does not fit. At 1 the label has
      // 14 to sit in and centres there with room for the tails on p, q and g.
      // Shrinking the text instead would not have done it: 10 still asks for
      // 12.1, which clears 3 by nothing at all. Horizontal is the button's own,
      // so nothing moves sideways.
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 1),
      onPressed: () async {
        final box = context.findRenderObject()! as RenderBox;
        final origin = box.localToGlobal(Offset.zero);
        final picked = await showLumitPopup<int>(
          context: context,
          position: origin + Offset(0, box.size.height + 2),
          builder: (close) => FloatSurface(
            child: _SearchPickerBody(
              value: value,
              options: options,
              group: group,
              hint: hint ?? l10n.search,
              onPick: close,
            ),
          ),
        );
        if (picked != null) onChanged(picked);
      },
      dropdown: true,
      child: dropdownFace(t, label),
    );
  }
}

/// One row of the picker's flattened list: a heading, or an option.
class _PickerEntry {
  final String? heading;
  final int? optionIndex;
  const _PickerEntry.heading(this.heading) : optionIndex = null;
  const _PickerEntry.option(this.optionIndex) : heading = null;
}

class _SearchPickerBody extends StatefulWidget {
  final int value;
  final List<String> options;
  final String? Function(String)? group;
  final String hint;
  final void Function(int?) onPick;

  const _SearchPickerBody({
    required this.value,
    required this.options,
    required this.group,
    required this.hint,
    required this.onPick,
  });

  @override
  State<_SearchPickerBody> createState() => _SearchPickerBodyState();
}

class _SearchPickerBodyState extends State<_SearchPickerBody> {
  final TextEditingController _query = TextEditingController();
  late List<_PickerEntry> _entries = _build('');

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  /// The visible rows for a query: every option whose label contains it
  /// (case-insensitively, all terms), with a heading each time the group
  /// changes. Flattened so the list builder stays lazy.
  List<_PickerEntry> _build(String query) {
    final terms =
        query.toLowerCase().split(' ').where((w) => w.isNotEmpty).toList();
    final out = <_PickerEntry>[];
    String? lastGroup;
    for (var i = 0; i < widget.options.length; i++) {
      final label = widget.options[i];
      final lower = label.toLowerCase();
      if (terms.any((w) => !lower.contains(w))) continue;
      final g = widget.group?.call(label);
      if (g != null && g != lastGroup) {
        out.add(_PickerEntry.heading(g));
        lastGroup = g;
      }
      out.add(_PickerEntry.option(i));
    }
    return out;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // A fixed box: the popup's own scroll view would otherwise give the
    // list unbounded height, and an unbounded ListView cannot be lazy.
    return SizedBox(
      width: 300,
      height: 380,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 2, 4, 6),
            child: HouseTextField(
              controller: _query,
              width: 288,
              autofocus: true,
              hint: widget.hint,
              onSubmitted: (_) {
                // Enter takes the only match, which is what a search that
                // has narrowed to one thing means.
                final only = _entries.where((e) => e.optionIndex != null);
                if (only.length == 1) widget.onPick(only.first.optionIndex);
              },
            ),
          ),
          Expanded(
            child: _entries.isEmpty
                ? Center(
                    child: Text(
                      l10n.noMatches,
                      style: t.small.copyWith(color: t.textMuted),
                    ),
                  )
                : ListView.builder(
                    primary: false,
                    itemCount: _entries.length,
                    itemBuilder: (context, i) {
                      final e = _entries[i];
                      final heading = e.heading;
                      if (heading != null) {
                        return Padding(
                          padding:
                              EdgeInsets.fromLTRB(10, i == 0 ? 2 : 8, 10, 2),
                          child: Text(
                            heading,
                            style: t.small.copyWith(color: t.textMuted),
                          ),
                        );
                      }
                      final idx = e.optionIndex!;
                      return MenuRow(
                        selected: idx == widget.value,
                        onPressed: () => widget.onPick(idx),
                        child: Text(
                          widget.options[idx],
                          overflow: TextOverflow.ellipsis,
                        ),
                      );
                    },
                  ),
          ),
        ],
      ),
    );
  }

  @override
  void initState() {
    super.initState();
    _query.addListener(() {
      setState(() => _entries = _build(_query.text));
    });
  }
}

/// A [BareDropdown] whose option list is built only when the menu opens.
///
/// For pickers whose options are bridge reads (the Timeline's parent picker):
/// the resting button then costs nothing per rebuild, and the reads happen
/// once per click instead of once per rebuild.
class BareLazyDropdown<T> extends StatelessWidget {
  /// What the closed button shows.
  final String label;

  /// The options, as (value, label) pairs — called when the menu opens.
  final List<(T, String)> Function() options;
  final ValueChanged<T> onChanged;

  /// The in-row face: 16 tall with a 10px label, for a picker that sits inside
  /// a Timeline row rather than in a dialog or a bar (§12A.6).
  final bool dense;

  const BareLazyDropdown({
    super.key,
    required this.label,
    required this.options,
    required this.onChanged,
    this.dense = false,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return dropdownButton(
      t: t,
      dense: dense,
      onPressed: () async {
        final box = context.findRenderObject()! as RenderBox;
        final origin = box.localToGlobal(Offset.zero);
        final built = options();
        final picked = await showLumitPopup<(T,)>(
          context: context,
          position: origin + Offset(0, box.size.height + 2),
          builder: (close) => FloatSurface(
            child: IntrinsicWidth(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final (value, optionLabel) in built)
                    MenuRow(
                      selected: optionLabel == label,
                      // Wrapped in a record so a null value survives the
                      // popup's null-means-dismissed contract.
                      onPressed: () => close((value,)),
                      child: Text(optionLabel),
                    ),
                ],
              ),
            ),
          ),
        );
        if (picked != null) onChanged(picked.$1);
      },
      face: dropdownFace(t, label),
    );
  }
}

class _CaretPainter extends CustomPainter {
  final Color color;
  const _CaretPainter(this.color);
  @override
  void paint(Canvas canvas, Size size) {
    final p = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5
      ..strokeCap = StrokeCap.round;
    final w = size.width, h = size.height;
    canvas.drawLine(Offset(w * 0.2, h * 0.35), Offset(w * 0.5, h * 0.65), p);
    canvas.drawLine(Offset(w * 0.5, h * 0.65), Offset(w * 0.8, h * 0.35), p);
  }

  @override
  bool shouldRepaint(_CaretPainter old) => old.color != color;
}
