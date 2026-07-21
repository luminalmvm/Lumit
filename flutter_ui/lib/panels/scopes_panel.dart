// The Scopes panel: draws the real graticule on the fixed scope colours —
// never themed (15-DESIGN §8). Traces over the decoded frame arrive with F2.

import 'package:flutter/widgets.dart';

import '../theme/theme.dart';
import '../widgets/controls.dart';

class ScopesPanel extends StatelessWidget {
  const ScopesPanel({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      color: ScopeColours.standard.bg,
      child: Column(
        children: [
          Container(
            height: 22,
            color: t.surface2,
            padding: const EdgeInsets.symmetric(horizontal: 6),
            alignment: Alignment.centerLeft,
            child: Text('Waveform (luma)', style: t.small),
          ),
          Expanded(
            child: CustomPaint(
              size: Size.infinite,
              painter: _GraticulePainter(),
            ),
          ),
        ],
      ),
    );
  }
}

class _GraticulePainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final p = Paint()
      ..color = ScopeColours.standard.graticule
      ..strokeWidth = 1;
    for (var i = 0; i <= 4; i++) {
      final y = size.height * i / 4;
      canvas.drawLine(Offset(0, y), Offset(size.width, y), p);
    }
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => false;
}
