// The Viewer: the exactly-neutral pasteboard with a placeholder slate and
// the transport row. The decoded frame path arrives in phase F2.

import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../state/app_state.dart';
import '../widgets/controls.dart';

class ViewerPanel extends StatelessWidget {
  final AppStateStub app;
  const ViewerPanel({super.key, required this.app});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      color: t.viewerSurround,
      child: Column(
        children: [
          Expanded(
            child: Center(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  lumitIcon(LumitIcon.film, size: 32, color: t.textDisabled),
                  const SizedBox(height: 8),
                  Text(
                    'The composited frame arrives with the shared-texture path (phase F2)',
                    style: t.small,
                  ),
                ],
              ),
            ),
          ),
          ListenableBuilder(
            listenable: app,
            builder: (context, _) => Container(
              height: 28,
              color: t.surface1,
              padding: const EdgeInsets.symmetric(horizontal: 6),
              child: Row(
                children: [
                  LumitTooltip(
                    message: app.playing ? 'Pause (Space)' : 'Play (Space)',
                    child: HouseButton(
                      frameless: true,
                      small: true,
                      onPressed: app.togglePlay,
                      child: lumitIcon(
                        app.playing ? LumitIcon.pause : LumitIcon.play,
                        size: 14,
                        color: t.textPrimary,
                      ),
                    ),
                  ),
                  const SizedBox(width: 8),
                  Text('frame ${app.previewFrame}', style: t.small),
                  const Spacer(),
                  Text('Full', style: t.small),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
