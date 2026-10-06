// The Paragraph panel: which side a text layer's lines line up on, and the
// room round them.
//
// Text in Lumit is point text, so each line the user breaks is a paragraph of
// its own. There is no box to justify the lines against yet, which is why the
// panel offers left, centre and right alone.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/text_documents.dart';
import '../widgets/controls.dart';
import 'text_target.dart';

class ParagraphPanelFrb extends StatefulWidget {
  const ParagraphPanelFrb({super.key});

  @override
  State<ParagraphPanelFrb> createState() => _ParagraphPanelFrbState();
}

class _ParagraphPanelFrbState extends State<ParagraphPanelFrb>
    with TextTargetState<ParagraphPanelFrb> {
  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return ListenableBuilder(
      listenable: Listenable.merge([ui.model, ui.selectedLayers, ui.tools]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        final target = targetOf(ui);
        final paragraph = target.document.paragraph;

        Widget align(BridgeTextAlign side, LumitIcon icon, String tip) {
          final on = paragraph.align == side;
          return LumitTooltip(
            message: tip,
            child: HouseButton(
              key: ValueKey<String>('paragraph-align-${side.name}'),
              small: true,
              frameless: !on,
              active: on,
              padding: const EdgeInsets.symmetric(horizontal: 4),
              onPressed: () => commitText(
                  ui, target, reflow((p) => p.copyWith(align: side))),
              child: lumitIcon(icon,
                  size: iconSize, color: on ? t.textPrimary : t.textMuted),
            ),
          );
        }

        Widget px(
          String keyName,
          double value,
          BridgeParagraphStyle Function(BridgeParagraphStyle, double) set,
        ) =>
            SizedBox(
              width: 72,
              child: DragValueField(
                key: ValueKey<String>(keyName),
                value: value,
                min: -8000,
                max: 8000,
                decimals: 1,
                suffix: ' px',
                resetTo: 0,
                onChangeLive: (v) => previewText(
                    ui, target, reflow((p) => set(p, v.toDouble()))),
                onChangeEnd: (v) => commitText(
                    ui, target, reflow((p) => set(p, v.toDouble()))),
                onDragCancel: () => cancelTextPreview(ui, target),
                onChanged: (v) => commitText(
                    ui, target, reflow((p) => set(p, v.toDouble()))),
              ),
            );

        return Container(
          color: t.surface0,
          child: ListView(
            children: [
              textTargetHeading(t, target.name ?? l10n.textNewText),
              Padding(
                padding: const EdgeInsets.fromLTRB(10, 2, 10, 4),
                child: Wrap(
                  spacing: 4,
                  children: [
                    align(BridgeTextAlign.left, LumitIcon.textAlignLeft,
                        l10n.tipParagraphAlignLeft),
                    align(BridgeTextAlign.centre, LumitIcon.textAlignCentre,
                        l10n.tipParagraphAlignCentre),
                    align(BridgeTextAlign.right, LumitIcon.textAlignRight,
                        l10n.tipParagraphAlignRight),
                  ],
                ),
              ),
              textGrid([
                textCell(
                  t,
                  l10n.paragraphIndentLeft,
                  px('paragraph-indent-left', paragraph.indentLeft,
                      (p, v) => p.copyWith(indentLeft: v)),
                ),
                textCell(
                  t,
                  l10n.paragraphIndentRight,
                  px('paragraph-indent-right', paragraph.indentRight,
                      (p, v) => p.copyWith(indentRight: v)),
                ),
                textCell(
                  t,
                  l10n.paragraphIndentFirst,
                  px('paragraph-indent-first', paragraph.indentFirst,
                      (p, v) => p.copyWith(indentFirst: v)),
                ),
                textCell(
                  t,
                  l10n.paragraphSpaceBefore,
                  px('paragraph-space-before', paragraph.spaceBefore,
                      (p, v) => p.copyWith(spaceBefore: v)),
                ),
                textCell(
                  t,
                  l10n.paragraphSpaceAfter,
                  px('paragraph-space-after', paragraph.spaceAfter,
                      (p, v) => p.copyWith(spaceAfter: v)),
                ),
              ]),
            ],
          ),
        );
      },
    );
  }
}
