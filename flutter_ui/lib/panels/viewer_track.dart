// The camera-track point cloud over the picture, and the gesture that turns a
// handful of its points into a layer (K-417, docs/07 §2.3.6).
//
// **In plain terms.** When a shot has been tracked, the solver knows where a few
// hundred little features of the scene are in three dimensions. These are drawn
// as dots on the picture: nearer ones bigger and brighter, further ones smaller
// and fainter, so the shape of the room is readable at a glance. Click one to
// select it, shift-click to add, drag a box round several — and then drop a Null
// or a Solid at the middle of what you picked, which is how anything gets
// attached to a tracked shot.
//
// **The engine does the maths.** Where a point lands on the picture at this
// frame, and how near it is relative to the rest of the cloud, both come across
// the bridge already worked out (`trackedPoints`). This file scales composition
// pixels into the panel and draws circles; it knows nothing about cameras.
//
// **Asked for once per frame**, never per rebuild — the Levels histogram's rule
// (K-413), and the bridge-call budget is the gate.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/track.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import '../widgets/marquee.dart';

/// How near a click has to land to take a point, in panel pixels. Generous,
/// because a dot is a few pixels across and the thing being aimed at is a
/// feature of the picture rather than a control.
const double _grabRadius = 10;

/// The dot's radius at the far end of the cloud and at the near end.
const double _farDot = 1.5;
const double _nearDot = 4.0;

/// The point cloud, its selection, and the Create null / Create solid
/// affordance.
class ViewerTrackLayer extends StatefulWidget {
  /// The tracked layer whose solve is drawn, or null when the comp has none —
  /// or when its Camera track's Show points is off. Found by the panel from
  /// the read model it already holds, so this costs no call of its own.
  final LayerReference? tracked;

  /// Where the picture is on screen, and how big the composition is, so a
  /// composition pixel can be put in the right place.
  final Rect fitted;
  final Size compSize;

  /// The frame on screen, and the document revision — the two things that move
  /// the cloud, and the only two that make it worth asking again.
  final int playheadFrame;
  final BigInt? revision;

  /// Bumped when an analysis lands a solve (K-420). A third thing to key the
  /// read by, because a solve arriving changes what the engine would answer
  /// while changing neither of the other two — the frame is where it was, and
  /// a solve is not an edit, so the document's revision has not moved.
  final int generation;

  final Color accent;
  final Color mark;

  /// Whether clicks and drags pick points, or pass through to the picture.
  ///
  /// **On only while the tracked layer is the selected one.** The cloud is
  /// drawn whenever Show points is on and there is a solve — that is what the
  /// switch says — but a cloud that always took the pointer would make the
  /// whole shot unclickable, and clicking the picture is how a layer is
  /// selected (K-217).
  final bool selecting;

  /// A layer was added at the selection: the panels re-read.
  final VoidCallback onChanged;

  /// Where the cloud comes from. Null is the engine's own
  /// [trackedPoints] — the only thing that ever runs in the application.
  ///
  /// The seam exists because a solve cannot be put into the engine's store from
  /// Dart: it is the answer to a minutes-long analysis of a real media file,
  /// and `lumit-render`'s own tests are where that is driven (§5b). The same
  /// shape the engine uses for the same reason, one level down — its `LumaFrames`
  /// is a trait so the job can be tested with no asset.
  final List<BridgeTrackPoint> Function(LayerReference layer, int frame)? fetch;

  const ViewerTrackLayer({
    super.key,
    required this.tracked,
    required this.fitted,
    required this.compSize,
    required this.playheadFrame,
    required this.revision,
    this.generation = 0,
    required this.accent,
    required this.mark,
    required this.selecting,
    required this.onChanged,
    this.fetch,
  });

  @override
  State<ViewerTrackLayer> createState() => _ViewerTrackLayerState();
}

class _ViewerTrackLayerState extends State<ViewerTrackLayer> {
  List<BridgeTrackPoint> _points = const [];

  /// Which points are picked, by track id. **Panel state** (K-417): a selection
  /// of features is a thing you are doing, not a thing the document holds.
  final Set<int> _picked = <int>{};

  /// What the last read was for, so a rebuild that changed neither does not
  /// ask again.
  int? _askedFrame;
  BigInt? _askedRevision;
  int? _askedGeneration;

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_onKey);
    WidgetsBinding.instance.addPostFrameCallback((_) => _read());
  }

  @override
  void didUpdateWidget(ViewerTrackLayer old) {
    super.didUpdateWidget(old);
    _read();
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    super.dispose();
  }

  /// Escape clears the selection — the same thing it does everywhere a
  /// selection is held.
  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent || !mounted) return false;
    if (event.logicalKey != LogicalKeyboardKey.escape) return false;
    if (_picked.isEmpty || lumitModalOpen) return false;
    setState(_picked.clear);
    return true;
  }

  void _read() {
    final tracked = widget.tracked;
    if (!mounted) return;
    if (tracked == null) {
      if (_points.isNotEmpty) setState(() => _points = const []);
      _askedFrame = null;
      _askedRevision = null;
      _askedGeneration = null;
      return;
    }
    if (_askedFrame == widget.playheadFrame &&
        _askedRevision == widget.revision &&
        _askedGeneration == widget.generation) {
      return;
    }
    _askedFrame = widget.playheadFrame;
    _askedRevision = widget.revision;
    _askedGeneration = widget.generation;
    List<BridgeTrackPoint> next;
    try {
      next = (widget.fetch ?? _fromEngine)(tracked, widget.playheadFrame);
    } catch (_) {
      // The layer went away under the overlay; the cloud simply empties.
      next = const [];
    }
    if (!mounted) return;
    setState(() => _points = next);
  }

  /// A composition point on the panel.
  Offset _at(BridgeTrackPoint p) {
    final f = widget.fitted;
    final w = widget.compSize.width == 0 ? 1.0 : widget.compSize.width;
    final h = widget.compSize.height == 0 ? 1.0 : widget.compSize.height;
    return Offset(f.left + p.x * f.width / w, f.top + p.y * f.height / h);
  }

  /// The nearest point to [where], within reach.
  BridgeTrackPoint? _hit(Offset where) {
    BridgeTrackPoint? best;
    var bestD = _grabRadius;
    for (final p in _points) {
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
      if (hit == null) {
        // A click on nothing clears, unless it is a shift-click — which is
        // someone adding to a selection and missing.
        if (!adding) _picked.clear();
        return;
      }
      if (!adding) _picked.clear();
      if (!_picked.add(hit.track)) _picked.remove(hit.track);
    });
  }

  void _marquee(Rect box) {
    final adding = HardwareKeyboard.instance.isShiftPressed;
    setState(() {
      if (!adding) _picked.clear();
      for (final p in _points) {
        if (box.contains(_at(p))) _picked.add(p.track);
      }
    });
  }

  /// Where the affordance sits: under the picked points, kept on the panel.
  Offset _affordanceAt(Size size) {
    var box = Rect.zero;
    for (final p in _points) {
      if (!_picked.contains(p.track)) continue;
      final at = _at(p);
      box = box == Rect.zero
          ? Rect.fromPoints(at, at)
          : box.expandToInclude(Rect.fromPoints(at, at));
    }
    const width = 180.0;
    const height = 28.0;
    return Offset(
      (box.center.dx - width / 2)
          .clamp(4.0, (size.width - width - 4).clamp(4.0, double.infinity)),
      (box.bottom + 12)
          .clamp(4.0, (size.height - height - 4).clamp(4.0, double.infinity)),
    );
  }

  void _create({required bool solid}) {
    final tracked = widget.tracked;
    if (tracked == null || _picked.isEmpty) return;
    try {
      addLayerAtPoints(
        tracked: tracked,
        tracks: _picked.toList()..sort(),
        frame: widget.playheadFrame,
        solid: solid,
      );
    } catch (_) {
      // Nothing solved at those points; nothing is added and the selection
      // stays, so the gesture can be tried somewhere else.
      return;
    }
    setState(_picked.clear);
    widget.onChanged();
  }

  @override
  Widget build(BuildContext context) {
    if (widget.tracked == null || _points.isEmpty) {
      return const SizedBox.shrink();
    }
    // Deliberately **not** wrapped in a `Positioned`: the caller places this
    // over the picture, and a widget that positioned itself could not then sit
    // inside a builder that follows the playhead.
    return LayoutBuilder(
      builder: (context, constraints) => Stack(
        children: [
          // The marquee takes the pointer, so it sits *under* the dots'
          // painter — the painter draws and takes nothing.
          if (widget.selecting)
            Positioned.fill(
              child: MarqueeSelect(
                key: const ValueKey('viewer-track-marquee'),
                onSelect: _marquee,
                onClear: () => setState(_picked.clear),
                onTapAt: _tapAt,
              ),
            ),
          Positioned.fill(
            child: IgnorePointer(
              child: CustomPaint(
                key: const ValueKey('viewer-track-points'),
                painter: TrackPointPainter(
                  points: [
                    for (final p in _points)
                      (
                        at: _at(p),
                        depth: p.depth,
                        picked: _picked.contains(p.track),
                      ),
                  ],
                  picture: widget.fitted,
                  dot: widget.mark,
                  accent: widget.accent,
                ),
              ),
            ),
          ),
          if (_picked.isNotEmpty && widget.selecting)
            Positioned(
              left: _affordanceAt(constraints.biggest).dx,
              top: _affordanceAt(constraints.biggest).dy,
              child: _CreateBar(
                onNull: () => _create(solid: false),
                onSolid: () => _create(solid: true),
              ),
            ),
        ],
      ),
    );
  }
}

/// The engine's own answer — what the application always uses.
List<BridgeTrackPoint> _fromEngine(LayerReference layer, int frame) =>
    trackedPoints(layer: layer, frame: frame);

/// The two things a picked set of points can become. A small floating row
/// rather than a context menu: the gesture that made the selection is a drag on
/// the picture, and asking for a second, hidden gesture to act on it would be
/// the calmer-looking but slower answer.
class _CreateBar extends StatelessWidget {
  final VoidCallback onNull;
  final VoidCallback onSolid;

  const _CreateBar({required this.onNull, required this.onSolid});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border.all(color: t.hairlineStrong),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          HouseButton(
            key: const ValueKey('viewer-track-create-null'),
            small: true,
            frameless: true,
            onPressed: onNull,
            child: Text(l10n.trackCreateNull, style: t.small),
          ),
          const SizedBox(width: 4),
          HouseButton(
            key: const ValueKey('viewer-track-create-solid'),
            small: true,
            frameless: true,
            onPressed: onSolid,
            child: Text(l10n.trackCreateSolid, style: t.small),
          ),
        ],
      ),
    );
  }
}

/// The dots.
///
/// **Depth is a cue, not a number**: nearer points are drawn bigger and at full
/// strength, further ones smaller and faded, so the cloud reads as a room
/// rather than as a spray. The nearness itself arrives normalised over the
/// cloud on this frame — the engine's decision, not this painter's.
class TrackPointPainter extends CustomPainter {
  final List<({Offset at, double depth, bool picked})> points;

  /// The picture's rectangle: nothing is drawn outside it, for the reason the
  /// guides overlay clips too — at a high magnification most of the cloud is
  /// off the panel.
  final Rect picture;
  final Color dot;
  final Color accent;

  const TrackPointPainter({
    required this.points,
    required this.picture,
    required this.dot,
    required this.accent,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final area = picture.intersect(Offset.zero & size);
    if (area.isEmpty) return;
    canvas.save();
    canvas.clipRect(area);
    for (final p in points) {
      final near = p.depth.clamp(0.0, 1.0);
      final radius = _farDot + (_nearDot - _farDot) * near;
      if (p.picked) {
        canvas.drawCircle(p.at, radius, Paint()..color = accent);
        canvas.drawCircle(
          p.at,
          radius + 2.5,
          Paint()
            ..color = accent
            ..style = PaintingStyle.stroke
            ..strokeWidth = 1,
        );
      } else {
        canvas.drawCircle(
          p.at,
          radius,
          Paint()..color = dot.withValues(alpha: 0.25 + 0.55 * near),
        );
      }
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(TrackPointPainter old) =>
      old.picture != picture ||
      old.dot != dot ||
      old.accent != accent ||
      old.points.length != points.length ||
      // Short-circuits on the first dot that moved, and builds nothing to do
      // it: this runs on every repaint with a few hundred points in hand.
      points.indexed.any((p) => old.points[p.$1] != p.$2);
}
