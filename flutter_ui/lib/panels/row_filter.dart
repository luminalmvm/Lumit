// The filter a panel of property rows opens on Ctrl+F: a well in the header,
// there only while it is wanted. Effect controls and the Node panel share it,
// so finding a row works the same way in both.
//
// The well is shown by the keymap's search action and goes again when it
// loses focus with nothing in it, or on Escape. While it holds text the rows
// whose names match stay, with the headings above them.

import 'package:flutter/widgets.dart';

import '../icons/icons.dart' show iconSize;
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../widgets/controls.dart';

/// The text and whether the well is up. Listeners hear both change.
class RowFilter extends ChangeNotifier {
  final TextEditingController text = TextEditingController();
  final FocusNode focus = FocusNode();
  bool open = false;

  RowFilter() {
    text.addListener(notifyListeners);
    focus.addListener(_onFocus);
  }

  /// What the rows are matched against: lowercase, empty for everything.
  String get needle => open ? text.text.trim().toLowerCase() : '';

  bool matches(String label) =>
      needle.isEmpty || label.toLowerCase().contains(needle);

  /// Put the well up and the cursor in it.
  void show() {
    if (!open) {
      open = true;
      notifyListeners();
    }
    WidgetsBinding.instance.addPostFrameCallback((_) => focus.requestFocus());
  }

  /// Escape: the text goes and the well with it.
  void clear() {
    text.clear();
    if (!open) return;
    open = false;
    notifyListeners();
  }

  void _onFocus() {
    if (!focus.hasFocus && open && text.text.trim().isEmpty) clear();
  }

  @override
  void dispose() {
    text.dispose();
    focus.dispose();
    super.dispose();
  }
}

/// The well in a header's name slot, 20 tall, with the search glyph that says
/// what it is for.
Widget rowFilterWell(BuildContext context, RowFilter filter, {Key? key}) {
  final t = ThemeScope.of(context).theme;
  return SizedBox(
    height: 20,
    child: HouseTextField(
      key: key,
      controller: filter.text,
      focusNode: filter.focus,
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      leading: glyph.LumitIcon(LumitIcons.search, size: 12, colour: t.textMuted),
      onCancelled: filter.clear,
    ),
  );
}

/// The search mark a header shows while its well is down: the visible way to
/// the filter that Ctrl+F opens.
Widget rowFilterMark(BuildContext context, VoidCallback show, {Key? key}) {
  final t = ThemeScope.of(context).theme;
  return LumitTooltip(
    message: l10n.filterRows,
    action: 'panel.search.focus',
    child: HouseButton(
      key: key,
      frameless: true,
      small: true,
      padding: const EdgeInsets.symmetric(horizontal: 3, vertical: 2),
      onPressed: show,
      child: glyph.LumitIcon(LumitIcons.search, size: iconSize, colour: t.textMuted),
    ),
  );
}
