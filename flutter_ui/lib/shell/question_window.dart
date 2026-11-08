// A window that asks one thing: the question in the title strip, a line under
// it only when the answer depends on it, and the answers in the footer. The
// last answer is the filled one, and Enter presses it.

import 'package:flutter/widgets.dart';

import '../widgets/controls.dart';
import 'dialog_frame.dart';

class QuestionWindow extends StatelessWidget {
  final String title;

  /// What can't be told from the question.
  final String? body;

  /// A fact for the footer: a size, a count.
  final String summary;
  final String keyPrefix;
  final VoidCallback onClose;
  final List<Widget> actions;
  final double width;

  /// Anything longer than a line of words, in the body's place.
  final Widget? content;

  /// The answers in a column, for ones too long to sit in a row in every
  /// language.
  final bool stacked;

  const QuestionWindow({
    super.key,
    required this.title,
    this.body,
    this.summary = '',
    required this.keyPrefix,
    required this.onClose,
    required this.actions,
    this.width = 380,
    this.content,
    this.stacked = false,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final words = body;
    return DialogFrame(
      width: width,
      children: [
        dialogTitleBar(t, title: title, onClose: onClose, keyPrefix: keyPrefix),
        if (content != null || words != null)
          Padding(
            padding: const EdgeInsets.all(dialogPadding),
            child: content ??
                Text(words!,
                    style:
                        t.body.copyWith(color: t.textSecondary, height: 1.4)),
          ),
        dialogFooter(t,
            keyPrefix: keyPrefix,
            summary: summary,
            stacked: stacked,
            actions: actions),
      ],
    );
  }
}

/// A footer's plain answer.
Widget questionPlain(String key, String label, VoidCallback? onPressed) =>
    HouseButton(
      key: ValueKey<String>(key),
      padding: const EdgeInsets.symmetric(horizontal: 12),
      onPressed: onPressed,
      child: Text(label),
    );

/// A footer's filled answer, which Enter presses.
Widget questionFilled(String key, String label, VoidCallback? onPressed) =>
    HouseButton(
      key: ValueKey<String>(key),
      primary: true,
      autofocus: true,
      padding: const EdgeInsets.symmetric(horizontal: 16),
      onPressed: onPressed,
      child: Text(label),
    );
