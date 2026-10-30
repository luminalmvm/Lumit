// The overflow mark, and the row of buttons that folds into it.
//
// A bar too narrow for its buttons gives the least important ones up into one
// mark at its end, the Viewer bar's ⋯, rather than scrolling them out of sight.

import 'dart:math';

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

import '../../icons/icons.dart' show iconSize;
import '../../icons/lumit_icon.dart';
import '../../icons/lumit_icons.dart';
import '../../l10n/strings.dart';
import 'base.dart';
import 'buttons.dart';
import 'menus.dart';
import 'popups.dart';

/// The app's one overflow mark: a frameless button with a ⋯ on it. [onOpen]
/// gets the button's own context, to anchor what it opens.
class OverflowMark extends StatelessWidget {
  final double width;
  final double height;
  final Key? buttonKey;
  final void Function(BuildContext context) onOpen;

  const OverflowMark({
    super.key,
    required this.width,
    required this.height,
    required this.onOpen,
    this.buttonKey,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: l10n.tipViewerMoreControls,
      child: Builder(
        builder: (markContext) => HouseButton(
          key: buttonKey,
          frameless: true,
          padding: EdgeInsets.zero,
          onPressed: () => onOpen(markContext),
          child: SizedBox(
            width: width,
            height: height,
            child: Center(
                child: LumitIcon(LumitIcons.more, colour: t.textSecondary)),
          ),
        ),
      ),
    );
  }
}

/// One button on a bar that folds: the button as the bar draws it, and the
/// row the overflow menu lists it as once it has folded.
class FoldItem {
  final Widget child;
  final String label;
  final Widget? icon;
  final VoidCallback? onPressed;
  final bool selected;

  /// Never folds: what the bar is for.
  final bool keep;

  const FoldItem({
    required this.child,
    required this.label,
    this.icon,
    this.onPressed,
    this.selected = false,
    this.keep = false,
  });
}

/// A row of [items] that folds the ones it has no room for into an
/// [OverflowMark], from the end of the row back. Items marked [FoldItem.keep]
/// stay; if they alone are too wide the row is cut at its end.
///
/// A folded item keeps working through its shortcut, which never went through
/// the button.
class FoldingRow extends StatefulWidget {
  final List<FoldItem> items;

  /// The mark's size, which the bar decides.
  final double markWidth;
  final double markHeight;
  final Key? markKey;

  const FoldingRow({
    super.key,
    required this.items,
    required this.markWidth,
    required this.markHeight,
    this.markKey,
  });

  @override
  State<FoldingRow> createState() => _FoldingRowState();
}

class _FoldingRowState extends State<FoldingRow> {
  // Which items the last layout folded, written by the render object and read
  // when the mark is pressed.
  final _folded = <int>[];

  void _open(BuildContext context) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    final t = ThemeScope.of(context).theme;
    final items = [for (final i in _folded) widget.items[i]];
    showLumitPopup<void>(
      context: context,
      position: box.localToGlobal(Offset(0, box.size.height)),
      // As wide as its longest row, not the window.
      builder: (close) => IntrinsicWidth(
          child: FloatSurface(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final item in items)
              MenuRow(
                selected: item.selected,
                onPressed: () {
                  close(null);
                  item.onPressed?.call();
                },
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    SizedBox(width: iconSize, child: item.icon),
                    const SizedBox(width: 8),
                    Text(
                      item.label,
                      style: item.onPressed == null
                          ? TextStyle(color: t.textDisabled)
                          : null,
                    ),
                  ],
                ),
              ),
          ],
        ),
      )),
    );
  }

  @override
  Widget build(BuildContext context) => _Fold(
        keep: [for (final item in widget.items) item.keep],
        folded: _folded,
        children: [
          for (final item in widget.items) item.child,
          OverflowMark(
            buttonKey: widget.markKey,
            width: widget.markWidth,
            height: widget.markHeight,
            onOpen: _open,
          ),
        ],
      );
}

class _Fold extends MultiChildRenderObjectWidget {
  final List<bool> keep;
  final List<int> folded;

  const _Fold({
    required this.keep,
    required this.folded,
    required super.children,
  });

  @override
  _RenderFold createRenderObject(BuildContext context) =>
      _RenderFold(keep, folded);

  @override
  void updateRenderObject(BuildContext context, _RenderFold renderObject) {
    renderObject
      ..keep = keep
      ..folded = folded;
  }
}

class _FoldParent extends ContainerBoxParentData<RenderBox> {
  bool shown = true;
}

/// Lays every child out at its own width, then shows as many as fit, left to
/// right, folding from the end and skipping the kept ones. The last child is
/// the mark, shown only when something folded.
class _RenderFold extends RenderBox
    with
        ContainerRenderObjectMixin<RenderBox, _FoldParent>,
        RenderBoxContainerDefaultsMixin<RenderBox, _FoldParent> {
  _RenderFold(this._keep, this.folded);

  List<bool> _keep;
  set keep(List<bool> v) {
    _keep = v;
    markNeedsLayout();
  }

  List<int> folded;
  bool _cut = false;
  double _clip = 0;

  @override
  void setupParentData(RenderBox child) {
    if (child.parentData is! _FoldParent) child.parentData = _FoldParent();
  }

  List<RenderBox> get _kids {
    final out = <RenderBox>[];
    var child = firstChild;
    while (child != null) {
      out.add(child);
      child = childAfter(child);
    }
    return out;
  }

  @override
  double computeMinIntrinsicWidth(double height) => 0;

  @override
  double computeMaxIntrinsicWidth(double height) {
    final kids = _kids;
    var w = 0.0;
    for (var i = 0; i < kids.length - 1; i++) {
      w += kids[i].getMaxIntrinsicWidth(height);
    }
    return w;
  }

  @override
  double computeMaxIntrinsicHeight(double width) => _kids.fold(
      0.0, (h, k) => max(h, k.getMaxIntrinsicHeight(double.infinity)));

  @override
  double computeMinIntrinsicHeight(double width) =>
      computeMaxIntrinsicHeight(width);

  @override
  void performLayout() {
    final kids = _kids;
    final loose = BoxConstraints(maxHeight: constraints.maxHeight);
    var height = 0.0;
    for (final k in kids) {
      k.layout(loose, parentUsesSize: true);
      if (k.size.height > height) height = k.size.height;
    }
    final items = kids.length - 1;
    final mark = kids.last;
    final shown = List<bool>.filled(items, true);
    var used = 0.0;
    for (var i = 0; i < items; i++) {
      used += kids[i].size.width;
    }
    folded.clear();
    if (used > constraints.maxWidth) {
      used += mark.size.width;
      for (var i = items - 1; i >= 0 && used > constraints.maxWidth; i--) {
        if (i < _keep.length && _keep[i]) continue;
        shown[i] = false;
        used -= kids[i].size.width;
        folded.insert(0, i);
      }
    }
    // The mark only when something folded. Kept items too wide even so are
    // cut short of the mark, which stays in reach at the end.
    final markShown = folded.isNotEmpty;
    height = constraints.constrainHeight(height);
    var x = 0.0;
    for (var i = 0; i < items; i++) {
      final data = kids[i].parentData! as _FoldParent;
      data.shown = shown[i];
      if (!data.shown) continue;
      data.offset = Offset(x, (height - kids[i].size.height) / 2);
      x += kids[i].size.width;
    }
    final markData = mark.parentData! as _FoldParent;
    markData.shown = markShown;
    _clip = min(x, constraints.maxWidth);
    if (markShown) {
      _clip = min(x, max(0.0, constraints.maxWidth - mark.size.width));
      markData.offset = Offset(_clip, (height - mark.size.height) / 2);
    }
    _cut = _clip < x;
    size = constraints
        .constrain(Size(_clip + (markShown ? mark.size.width : 0), height));
  }

  @override
  void paint(PaintingContext context, Offset offset) {
    final kids = _kids;
    void items(PaintingContext context, Offset offset) {
      for (final k in kids.take(kids.length - 1)) {
        final data = k.parentData! as _FoldParent;
        if (data.shown) context.paintChild(k, offset + data.offset);
      }
    }

    if (_cut) {
      context.pushClipRect(
          needsCompositing, offset, Offset.zero & Size(_clip, size.height), items);
    } else {
      items(context, offset);
    }
    final mark = kids.last;
    final data = mark.parentData! as _FoldParent;
    if (data.shown) context.paintChild(mark, offset + data.offset);
  }

  @override
  bool hitTestChildren(BoxHitTestResult result, {required Offset position}) {
    for (final k in _kids.reversed) {
      final data = k.parentData! as _FoldParent;
      if (!data.shown) continue;
      final hit = result.addWithPaintOffset(
        offset: data.offset,
        position: position,
        hitTest: (result, transformed) => k.hitTest(result, position: transformed),
      );
      if (hit) return true;
    }
    return false;
  }
}
