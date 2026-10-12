// What the Viewer wears while this machine follows someone in a shared
// project: a frame in their colour, and hung from its top edge a tab that
// says who, with the way to stop. The two are drawn as one thing, and
// thicker than a focus ring so it is not taken for one.
//
// Following moves this machine's open composition and playhead to theirs and
// nothing else. The state is `LumitUiState.following`. This only draws it,
// and takes Escape for as long as it is drawn.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';

/// How thick the frame is. Three is thicker than any focus ring in the
/// window, and the tab that hangs from it is filled with the same colour, so
/// the two read as one mark in the person's colour from across a room.
const double viewerFollowFrame = 3;

class ViewerFollow extends StatelessWidget {
  final LumitUiState uiState;
  const ViewerFollow({super.key, required this.uiState});

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<int?>(
        valueListenable: uiState.following,
        builder: (context, following, _) {
          final them = uiState.followed;
          if (following == null || them == null) {
            return const SizedBox.shrink();
          }
          final t = ThemeScope.of(context).theme;
          final colour = t.personColour(them.colour);
          return Positioned.fill(
            child: Stack(
              children: [
                // Over the picture and under the pointer: it is only a line.
                Positioned.fill(
                  child: IgnorePointer(
                    child: DecoratedBox(
                      key: const ValueKey('viewer-follow-frame'),
                      decoration: BoxDecoration(
                          border: Border.all(
                              color: colour, width: viewerFollowFrame)),
                    ),
                  ),
                ),
                Positioned(
                  top: 0,
                  left: 0,
                  right: 0,
                  child: Center(
                    child: _Chip(
                      name: them.name,
                      colour: colour,
                      onStop: () => uiState.follow(null),
                    ),
                  ),
                ),
              ],
            ),
          );
        },
      );
}

/// Who is being followed, and the button that stops it. Escape stops it
/// too, for as long as the tab is up.
class _Chip extends StatefulWidget {
  final String name;
  final Color colour;
  final VoidCallback onStop;
  const _Chip({required this.name, required this.colour, required this.onStop});

  @override
  State<_Chip> createState() => _ChipState();
}

class _ChipState extends State<_Chip> {
  late final VoidCallback _escapeRelease;

  @override
  void initState() {
    super.initState();
    // The last thing Escape takes back: anything in flight or open goes
    // first, and a selection is asked after this.
    _escapeRelease = EscapeLadder.register(EscapeRung.selection, () {
      widget.onStop();
      return true;
    });
  }

  @override
  void dispose() {
    _escapeRelease();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final corner = Radius.circular(t.tokens.actionRadius);
    // Words on the person's colour are in the darkest ground, which every
    // person's colour is bright enough to carry.
    final ink = t.surface0;
    return Container(
      key: const ValueKey('viewer-follow'),
      padding: const EdgeInsets.fromLTRB(10, viewerFollowFrame + 1, 3, 3),
      // Its top is the frame's own edge, so it is square there.
      decoration: BoxDecoration(
        color: widget.colour,
        borderRadius:
            BorderRadius.only(bottomLeft: corner, bottomRight: corner),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(l10n.shareFollowing(widget.name),
              style: t.small
                  .copyWith(color: ink, fontWeight: FontWeight.w600)),
          const SizedBox(width: 10),
          MouseRegion(
            cursor: SystemMouseCursors.click,
            child: GestureDetector(
              key: const ValueKey('viewer-follow-stop'),
              behavior: HitTestBehavior.opaque,
              onTap: widget.onStop,
              child: Container(
                height: 18,
                alignment: Alignment.center,
                padding: const EdgeInsets.symmetric(horizontal: 8),
                decoration: BoxDecoration(
                  color: ink.withValues(alpha: 0.22),
                  borderRadius: BorderRadius.circular(t.tokens.actionRadius),
                ),
                child: Text(l10n.shareFollowStopShort,
                    style: t.small.copyWith(color: ink)),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
