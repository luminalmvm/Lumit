// Stabilise's followed points over the picture, and the gestures that drop the
// ones the shot should not be steadied by.
//
// An analysis follows a few hundred little features of the shot. Most move
// with the picture; a few belong to something moving on its own, a person
// walking through or a car going by, and the analysis sets those aside. With
// Show points on, the points at the playhead are drawn where the steadied
// picture shows them: the kept ones as accent dots, the set-aside ones as muted
// rings, each on a thin dark edge so it reads on any picture. Pick some, the
// Camera track cloud's way, and Delete drops them, with every point moving
// with them, so a passer-by no longer pulls the frame about.
//
// **The engine does the maths.** Where a point is drawn, whether it was kept,
// and which points move with a dropped one all come across the bridge. This
// file scales composition pixels into the panel and draws circles.
//
// **Asked for once per frame**, never per rebuild, as the cloud is.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/stabilise.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../state/dock.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import '../widgets/marquee.dart';
import 'timeline_extras_frb.dart' show showMenuAt;
import 'viewer_menus.dart';

/// How near a click has to land to take a point, in panel pixels: the cloud's
/// reach.
const double _grabRadius = 10;

/// The dot's radius at a Point size of 100%: the Camera track cloud's
/// mid-depth dot.
const double _dot = 2.75;

/// How soon the points are read again while the engine works out the plan
/// they are placed with.
const Duration _planWait = Duration(milliseconds: 100);

/// One Stabilise's points over the picture.
class ViewerStabiliseLayer extends StatefulWidget {
  /// The stabilised layer, and which Stabilise on it.
  final LayerReference layer;
  final UuidValue effect;

  /// The Point size row, in percent.
  final double pointSize;

  /// Where the picture is on screen, and how big the composition is.
  final Rect fitted;
  final Size compSize;

  /// The frame on screen and the document revision: what moves the points.
  final int playheadFrame;
  final int? revision;

  /// Bumped when a tracker lands, which moves neither of the other two: the
  /// points are read again.
  final int landed;

  final Color accent;
  final Color muted;

  /// Whether clicks and drags pick points: only while the layer is selected,
  /// so the picture stays clickable otherwise.
  final bool selecting;

  /// Points were dropped or restored: the panels re-read.
  final VoidCallback onChanged;

  /// Where a right click with nothing to offer goes.
  final void Function(TapUpDetails)? onPictureMenu;

  const ViewerStabiliseLayer({
    super.key,
    required this.layer,
    required this.effect,
    required this.pointSize,
    required this.fitted,
    required this.compSize,
    required this.playheadFrame,
    required this.revision,
    this.landed = 0,
    required this.accent,
    required this.muted,
    required this.selecting,
    required this.onChanged,
    this.onPictureMenu,
  });

  @override
  State<ViewerStabiliseLayer> createState() => _ViewerStabiliseLayerState();
}

class _ViewerStabiliseLayerState extends State<ViewerStabiliseLayer> {
  List<BridgeStabilisePoint> _points = const [];

  /// Which points are picked, by track id. Panel state, as the cloud's is.
  final Set<int> _picked = <int>{};

  /// What the last read was for.
  int? _askedFrame;
  int? _askedRevision;
  int? _askedLanded;

  /// The analysis the points on screen belong to, which a pick names.
  int? _generation;

  /// Reads again while the plan the points are placed with is being made.
  Timer? _again;

  VoidCallback? _release;

  @override
  void initState() {
    super.initState();
    _release = EscapeLadder.register(EscapeRung.selection, _escape);
    _ui = Provider.of<LumitUiState>(context, listen: false);
    _priorDeleteClaim = _ui!.deleteClaim;
    _ui!.deleteClaim = _deleteClaim;
    WidgetsBinding.instance.addPostFrameCallback((_) => _read());
  }

  @override
  void didUpdateWidget(ViewerStabiliseLayer old) {
    super.didUpdateWidget(old);
    _read();
  }

  @override
  void dispose() {
    _again?.cancel();
    if (_ui?.deleteClaim == _deleteClaim) _ui!.deleteClaim = _priorDeleteClaim;
    _release?.call();
    _release = null;
    super.dispose();
  }

  /// Escape clears the pick, on the ladder's selection rung.
  bool _escape() {
    if (!mounted || _picked.isEmpty) return false;
    setState(_picked.clear);
    return true;
  }

  LumitUiState? _ui;
  bool Function()? _priorDeleteClaim;

  /// Delete or Backspace drops the picked points, through the shell's claim
  /// rather than a key handler of its own: every handler hears every key, so
  /// handling it here would not stop the shell deleting the layer as well.
  /// Only with a pick, from the Viewer, so the Timeline keeps Delete otherwise.
  bool _deleteClaim() {
    if (!mounted ||
        !widget.selecting ||
        _picked.isEmpty ||
        _ui?.activePanel != Panel.viewer) {
      return _priorDeleteClaim?.call() ?? false;
    }
    _drop();
    return true;
  }

  void _read() {
    if (!mounted) return;
    if (_askedFrame == widget.playheadFrame &&
        _askedRevision == widget.revision &&
        _askedLanded == widget.landed) {
      return;
    }
    _askedFrame = widget.playheadFrame;
    _askedRevision = widget.revision;
    _askedLanded = widget.landed;
    BridgeStabilisePoints next;
    try {
      next = stabilisePoints(
          layer: widget.layer,
          effect: widget.effect,
          frame: widget.playheadFrame);
    } catch (_) {
      // The layer went away under the overlay; the points simply go.
      next = BridgeStabilisePoints(
          ready: true, generation: 0, points: const []);
    }
    if (!mounted) return;
    _again?.cancel();
    _again = null;
    if (!next.ready) {
      // The engine is working out the plan: keep what is drawn, and the pick,
      // and ask again shortly.
      _askedFrame = null;
      _again = Timer(_planWait, _read);
      return;
    }
    setState(() {
      // A new analysis numbers its points afresh, so a pick from the last one
      // would name other points.
      if (_generation != null && _generation != next.generation) {
        _picked.clear();
      }
      _generation = next.generation;
      _points = next.points;
      // A pick only names points still on screen.
      final shown = {for (final p in next.points) p.track};
      _picked.removeWhere((t) => !shown.contains(t));
    });
  }

  /// A composition point on the panel.
  Offset _at(BridgeStabilisePoint p) {
    final f = widget.fitted;
    final w = widget.compSize.width == 0 ? 1.0 : widget.compSize.width;
    final h = widget.compSize.height == 0 ? 1.0 : widget.compSize.height;
    return Offset(f.left + p.x * f.width / w, f.top + p.y * f.height / h);
  }

  /// The points drawn: those on the picture and inside the panel, which the
  /// painter clips to.
  Iterable<BridgeStabilisePoint> get _drawn {
    final panel = Offset.zero & (context.size ?? Size.zero);
    final area = widget.fitted.intersect(panel);
    return _points.where((p) => area.contains(_at(p)));
  }

  BridgeStabilisePoint? _hit(Offset where) {
    BridgeStabilisePoint? best;
    var bestD = _grabRadius;
    for (final p in _drawn) {
      final d = (_at(p) - where).distance;
      if (d <= bestD) {
        bestD = d;
        best = p;
      }
    }
    return best;
  }

  void _tapAt(Offset where) {
    final hit = _hit(where);
    final adding = HardwareKeyboard.instance.isShiftPressed;
    setState(() {
      if (!adding) _picked.clear();
      if (hit == null) return;
      if (!_picked.add(hit.track)) _picked.remove(hit.track);
    });
  }

  void _marquee(Rect box, bool adding) {
    setState(() {
      if (!adding) _picked.clear();
      for (final p in _drawn) {
        if (box.contains(_at(p))) _picked.add(p.track);
      }
    });
  }

  /// Drop the picked points, and the engine drops those moving with them, as
  /// one undo step. Read again, since more may have gone than were picked.
  void _drop() {
    if (_picked.isEmpty) return;
    try {
      stabiliseDropPoints(
          layer: widget.layer,
          effect: widget.effect,
          tracks: _picked.toList()..sort());
    } catch (_) {
      // The layer went away; nothing was dropped.
      return;
    }
    setState(_picked.clear);
    _askedFrame = null;
    widget.onChanged();
  }

  /// The points' menu, as the cloud's: only over a point or with a pick, so
  /// the picture's own menu is reached everywhere else. Restore points is on
  /// the status line.
  void _menu(TapUpDetails details) {
    final hit = _hit(details.localPosition);
    if (hit == null && _picked.isEmpty) {
      widget.onPictureMenu?.call(details);
      return;
    }
    if (hit != null && !_picked.contains(hit.track)) {
      setState(() => _picked.add(hit.track));
    }
    showMenuAt<void>(
      context: context,
      position: details.globalPosition,
      rows: (closeWith) {
        void close() => closeWith(null);
        return [
          viewerRow(l10n.stabiliseDropPoints,
              _picked.isEmpty ? null : _drop, close,
              id: 'viewer-stabilise-menu-drop'),
          const MenuDivider(),
          viewerRow(
              l10n.menuSelectAll,
              () => setState(() => _picked
                ..clear()
                ..addAll(_drawn.map((p) => p.track))),
              close,
              id: 'viewer-stabilise-menu-all'),
        ];
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    if (_points.isEmpty) return const SizedBox.shrink();
    final t = ThemeScope.of(context).theme;
    final radius = _dot * (widget.pointSize / 100).clamp(0.1, 10.0);
    return Stack(
      children: [
        // The marquee takes the pointer, under the painter, which takes none.
        if (widget.selecting)
          Positioned.fill(
            child: GestureDetector(
              behavior: HitTestBehavior.translucent,
              onSecondaryTapUp: _menu,
              child: MarqueeSelect(
                key: const ValueKey('viewer-stabilise-marquee'),
                onSelect: _marquee,
                onClear: () => setState(_picked.clear),
                onTapAt: _tapAt,
              ),
            ),
          ),
        Positioned.fill(
          child: IgnorePointer(
            child: CustomPaint(
              key: const ValueKey('viewer-stabilise-points'),
              painter: StabilisePointPainter(
                points: [
                  for (final p in _points)
                    (
                      at: _at(p),
                      kept: p.kept,
                      picked: _picked.contains(p.track),
                    ),
                ],
                picture: widget.fitted,
                radius: radius,
                accent: widget.accent,
                muted: widget.muted,
                // The roto overlay's halo, and the cloud's mark for a pick.
                edge: t.surface0,
                mark: t.textPrimary,
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// The points: kept ones filled in the accent, set-aside ones as muted rings,
/// both over a thin dark edge. A picked one is drawn bigger and carries a second
/// ring in the text colour, so a pick reads at a glance on a busy shot.
class StabilisePointPainter extends CustomPainter {
  final List<({Offset at, bool kept, bool picked})> points;

  /// The picture's rectangle: nothing is drawn outside it.
  final Rect picture;
  final double radius;
  final Color accent;
  final Color muted;

  /// The edge under every mark, and the ring round a picked point.
  final Color edge;
  final Color mark;

  const StabilisePointPainter({
    required this.points,
    required this.picture,
    required this.radius,
    required this.accent,
    required this.muted,
    required this.edge,
    required this.mark,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final area = picture.intersect(Offset.zero & size);
    if (area.isEmpty) return;
    canvas.save();
    canvas.clipRect(area);
    // Four paints for every point: recoloured, never made per point.
    final fill = Paint();
    final under = Paint()..color = edge;
    final ring = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    final halo = Paint()
      ..color = edge
      ..style = PaintingStyle.stroke
      ..strokeWidth = 3;
    final grown = radius + 1.5;
    final outer = radius + 4.5;
    for (final p in points) {
      final r = p.picked ? grown : radius;
      if (p.kept) {
        canvas.drawCircle(p.at, r + 1, under);
        canvas.drawCircle(p.at, r, fill..color = accent);
      } else {
        canvas.drawCircle(p.at, r, halo);
        canvas.drawCircle(p.at, r, ring..color = muted);
      }
      if (p.picked) {
        canvas.drawCircle(p.at, outer, halo);
        canvas.drawCircle(p.at, outer, ring..color = mark);
      }
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(StabilisePointPainter old) =>
      old.picture != picture ||
      old.radius != radius ||
      old.accent != accent ||
      old.muted != muted ||
      old.edge != edge ||
      old.mark != mark ||
      old.points.length != points.length ||
      points.indexed.any((p) => old.points[p.$1] != p.$2);
}
