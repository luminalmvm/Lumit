// The Timeline strip skeleton: comp tab strip, ruler band and bottom bar
// with the real zoom / magnet / graph-lens controls. Layer rows, lanes and
// the graph lens arrive in phase F3.

import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../state/app_state.dart';
import '../widgets/controls.dart';

class TimelinePanel extends StatelessWidget {
  final AppStateStub app;
  const TimelinePanel({super.key, required this.app});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ListenableBuilder(
      listenable: app,
      builder: (context, _) => Column(
        children: [
          Container(
            height: 24,
            color: t.surface2,
            padding: const EdgeInsets.symmetric(horizontal: 6),
            child: Row(
              children: [
                Text(
                  app.openComps.isEmpty
                      ? 'No composition open'
                      : app.openComps.join(' · '),
                  style: t.small,
                ),
              ],
            ),
          ),
          Container(height: 18, color: t.surface0),
          Expanded(
            child: Center(
              child: Text(
                'Layer rows, lanes and the graph lens arrive in phase F3.',
                style: t.small,
              ),
            ),
          ),
          Container(
            height: 24,
            color: t.surface2,
            padding: const EdgeInsets.symmetric(horizontal: 6),
            child: Row(
              children: [
                HouseButton(
                  frameless: true,
                  small: true,
                  onPressed: () => app.zoomTimeline(1.4),
                  child: Text('+', style: t.bodyPrimary),
                ),
                HouseButton(
                  frameless: true,
                  small: true,
                  onPressed: () => app.zoomTimeline(1 / 1.4),
                  child: Text('−', style: t.bodyPrimary),
                ),
                Text('${app.timelineZoom.round()}%', style: t.small),
                const SizedBox(width: 10),
                LumitTooltip(
                  message: 'Snapping',
                  child: HouseButton(
                    frameless: true,
                    small: true,
                    onPressed: () {
                      app.snapping = !app.snapping;
                      app.setNotice(
                          app.snapping ? 'snapping on' : 'snapping off');
                    },
                    child: lumitIcon(
                      LumitIcon.magnet,
                      size: 13,
                      color: app.snapping ? t.accent : t.textMuted,
                    ),
                  ),
                ),
                const Spacer(),
                LumitTooltip(
                  message: 'Graph editor (Shift+F3)',
                  child: HouseButton(
                    frameless: true,
                    small: true,
                    onPressed: app.toggleGraphMode,
                    child: lumitIcon(
                      LumitIcon.graphCurve,
                      size: 13,
                      color: app.timelineGraphMode ? t.accent : t.textMuted,
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
