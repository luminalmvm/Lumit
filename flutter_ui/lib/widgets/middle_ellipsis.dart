// One line of text that, when it doesn't fit, loses its middle rather than its
// end. Paths and file names differ at the end (`…v12` and `…v13`), so that is
// the part to keep. The whole text is the tooltip whenever it is cut.

import 'package:flutter/widgets.dart';

import 'controls.dart';

class MiddleEllipsis extends StatelessWidget {
  final String text;
  final TextStyle style;

  /// What the tooltip says when the text is cut. The text itself by default.
  final String? tip;

  const MiddleEllipsis(this.text, {super.key, required this.style, this.tip});

  @override
  Widget build(BuildContext context) => LayoutBuilder(builder: (context, box) {
        final scaler = MediaQuery.textScalerOf(context);
        bool fits(String s) {
          final painter = TextPainter(
            text: TextSpan(text: s, style: style),
            textDirection: TextDirection.ltr,
            textScaler: scaler,
            maxLines: 1,
          )..layout();
          final width = painter.width;
          painter.dispose();
          return width <= box.maxWidth;
        }

        final letters = text.characters.toList();
        String keep(int count) {
          final head = count ~/ 2;
          return '${letters.take(head).join()}…'
              '${letters.skip(letters.length - (count - head)).join()}';
        }

        if (!box.hasBoundedWidth || fits(text)) {
          return Text(text, style: style, maxLines: 1, softWrap: false);
        }
        // The most letters that fit, found by halving.
        var low = 0;
        var high = letters.length - 1;
        while (low < high) {
          final mid = (low + high + 1) ~/ 2;
          if (fits(keep(mid))) {
            low = mid;
          } else {
            high = mid - 1;
          }
        }
        return LumitTooltip(
          message: tip ?? text,
          child: Text(keep(low),
              style: style,
              maxLines: 1,
              softWrap: false,
              overflow: TextOverflow.clip),
        );
      });
}
