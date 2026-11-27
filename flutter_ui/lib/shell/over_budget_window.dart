// A frame too big for the memory budget. The engine didn't draw it, and says
// whether an undo would put it right. When it would, the one button undoes
// the edit; when not, it only closes.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';

import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../widgets/controls.dart';
import 'menu_bar_frb.dart' show undoFrb;
import 'question_window.dart';
import 'settings_rows.dart' show byteSize;

Future<void> showFrameOverBudget(
        BuildContext context, LumitState app, BridgeFrameOverBudget over) =>
    showLumitModal<void>(
      context: context,
      builder: (close) => QuestionWindow(
        title: l10n.frameOverBudgetTitle,
        body: l10n.frameOverBudgetFacts(
            byteSize(over.wanted.toInt()), byteSize(over.budget.toInt())),
        keyPrefix: 'over-budget',
        onClose: () => close(null),
        actions: [
          over.undoable
              ? questionFilled('over-budget-undo', l10n.frameOverBudgetUndo,
                  () {
                  undoFrb(app);
                  close(null);
                })
              : questionFilled(
                  'over-budget-dismiss', l10n.close, () => close(null)),
        ],
      ),
    );
