// The window fading from one look to the next when a style is picked, its
// fonts arrive or the window turns see-through, rather than every part
// jumping at once. The look as it was is held as a picture over the new one
// and faded out, at the motion level's `swap`. Hover previews of a scheme
// never come here: they change too often to wait on.

import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

import '../state/workspace.dart';
import '../widgets/controls/base.dart' show ThemeScope;

class LookFade extends StatefulWidget {
  final Workspace workspace;
  final Widget child;
  const LookFade({super.key, required this.workspace, required this.child});

  @override
  State<LookFade> createState() => _LookFadeState();
}

class _LookFadeState extends State<LookFade>
    with SingleTickerProviderStateMixin {
  final _boundary = GlobalKey();
  late final _fade = AnimationController(vsync: this, value: 0);
  ui.Image? _was;

  @override
  void initState() {
    super.initState();
    widget.workspace.beforeLookChange = _hold;
    _fade.addStatusListener((status) {
      if (status == AnimationStatus.dismissed) _drop();
    });
  }

  /// Keep the look as it is now, to fade out once the new one is drawn.
  void _hold() {
    final spec = ThemeScope.of(context).motion.swap;
    final box = _boundary.currentContext?.findRenderObject();
    if (spec.isStill || box is! RenderRepaintBoundary || box.debugNeedsPaint) {
      return;
    }
    final was = box.toImageSync(
        pixelRatio: MediaQuery.devicePixelRatioOf(context));
    _was?.dispose();
    setState(() => _was = was);
    _fade
      ..value = 1
      ..animateTo(0, duration: spec.duration, curve: spec.curve);
  }

  void _drop() {
    final was = _was;
    if (was == null) return;
    setState(() => _was = null);
    // Painted once more by the frame that drops it.
    WidgetsBinding.instance.addPostFrameCallback((_) => was.dispose());
  }

  @override
  void dispose() {
    if (widget.workspace.beforeLookChange == _hold) {
      widget.workspace.beforeLookChange = null;
    }
    _fade.dispose();
    _was?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Stack(
        fit: StackFit.expand,
        children: [
          RepaintBoundary(key: _boundary, child: widget.child),
          // Softened a little as it goes, so two different layouts read as
          // one changing rather than two laid over each other.
          if (_was case final was?)
            IgnorePointer(
              child: AnimatedBuilder(
                animation: _fade,
                builder: (context, picture) => ImageFiltered(
                  imageFilter: ui.ImageFilter.blur(
                      sigmaX: (1 - _fade.value) * 3,
                      sigmaY: (1 - _fade.value) * 3),
                  child: Opacity(opacity: _fade.value, child: picture),
                ),
                child: RawImage(image: was, fit: BoxFit.fill),
              ),
            ),
        ],
      );
}
