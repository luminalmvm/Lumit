// The 3D handles on a 3D layer, a light or a camera: an arrow a drag moves it
// along, a square that moves it across a plane, and with the Rotation tool a
// ring that turns it about one axis (docs/07 §2.3).
//
// Everything here is worked out in the space Position lives in, the parent's,
// and drawn through the same rows the picture is drawn through. An arrow drag
// finds the point on the axis nearest the pointer as seen through the camera,
// a square drag meets the pointer's ray with its plane, and a ring drag
// measures the angle swept round the ring on its own plane. So a drag lands
// where the pointer says however the camera looks, parents and all, and
// nothing crosses the bridge while the pointer moves.

import 'dart:math' as math;

import 'package:flutter/painting.dart';

import '../state/tools.dart' show GizmoAxes;
import '../theme/theme.dart' show ScopeColours;
import 'viewer_camera.dart' show CameraPose;
import 'viewer_layer_map.dart';

typedef Vec3 = (double, double, double);

Vec3 _plus(Vec3 a, Vec3 b) => (a.$1 + b.$1, a.$2 + b.$2, a.$3 + b.$3);
Vec3 _times(Vec3 a, double k) => (a.$1 * k, a.$2 * k, a.$3 * k);
double _dot(Vec3 a, Vec3 b) => a.$1 * b.$1 + a.$2 * b.$2 + a.$3 * b.$3;
Vec3 _cross(Vec3 a, Vec3 b) => (
      a.$2 * b.$3 - a.$3 * b.$2,
      a.$3 * b.$1 - a.$1 * b.$3,
      a.$1 * b.$2 - a.$2 * b.$1,
    );
Vec3 _unit(Vec3 a) {
  final l = math.sqrt(_dot(a, a));
  return l < 1e-12 ? a : _times(a, 1 / l);
}

/// How long an arrow is on screen, and how far out a ring is drawn, in screen
/// pixels whatever the zoom or the depth.
const double gizmoArrowLength = 84;
const double gizmoRingRadius = 72;

/// How near a handle a press has to be to take it, in screen pixels.
const double gizmoReach = 7;

/// The ring round the handles' centre that moves across the screen, in
/// screen pixels: outside the anchor's own reach, inside where the arrows
/// start.
const double gizmoScreenRing = 13;

/// How much deeper than where it started a drag may push the handles, and
/// how deep at least, in camera zooms: past this an axis seen nearly end on
/// runs off to its vanishing point.
const double _deepest = 10;

/// x, y and z, in the Scopes panel's red, green and blue.
final List<Color> gizmoAxisColours = [
  ScopeColours.standard.red,
  ScopeColours.standard.green,
  ScopeColours.standard.blue,
];

/// Row [r] of three rows of four, times `(p, w)`.
double _row(List<double> rows, int r, Vec3 p, double w) =>
    rows[r * 4] * p.$1 +
    rows[r * 4 + 1] * p.$2 +
    rows[r * 4 + 2] * p.$3 +
    rows[r * 4 + 3] * w;

/// Where a plane's square sits between its two arrows, as parts of an arrow.
const double _squareFrom = 0.2;
const double _squareTo = 0.45;

const int _ringSteps = 64;

/// The layer's own axes, the columns of its turn Ry · Rx · Rz: the camera
/// tools' own arithmetic.
List<Vec3> localAxes(Vec3 deg) {
  final (:right, :up, :forward) =
      CameraPose(position: (0, 0, 0), rotation: deg, zoom: 1).axes;
  return [right, up, forward];
}

/// The same turn, row by row.
List<double> turnOf(Vec3 deg) {
  final [x, y, z] = localAxes(deg);
  return [x.$1, y.$1, z.$1, x.$2, y.$2, z.$2, x.$3, y.$3, z.$3];
}

/// The angles in degrees of a turn built as Ry · Rx · Rz, the port of
/// `lumit_core::camera::euler_yxz`.
Vec3 eulerYxz(List<double> m) {
  double deg(double r) => r * 180 / math.pi;
  final sinX = (-m[5]).clamp(-1.0, 1.0);
  final rx = math.asin(sinX);
  final cosX = math.sqrt(math.max(0, 1 - sinX * sinX));
  if (cosX < 1e-9) return (deg(rx), deg(math.atan2(-m[6], m[0])), 0);
  return (deg(rx), deg(math.atan2(m[2], m[8])), deg(math.atan2(m[3], m[4])));
}

/// After Effects' turn Rx · Ry · Rz, row by row: what Orientation turns by,
/// outside X, Y and Z Rotation.
List<double> turnOfAe(Vec3 deg) {
  double r(double d) => d * math.pi / 180;
  final (sx, cx) = (math.sin(r(deg.$1)), math.cos(r(deg.$1)));
  final (sy, cy) = (math.sin(r(deg.$2)), math.cos(r(deg.$2)));
  final (sz, cz) = (math.sin(r(deg.$3)), math.cos(r(deg.$3)));
  return [
    cy * cz, -cy * sz, sy, //
    cx * sz + sx * sy * cz, cx * cz - sx * sy * sz, -sx * cy,
    sx * sz - cx * sy * cz, sx * cz + cx * sy * sz, cx * cy,
  ];
}

/// The angles in degrees of a turn built as Rx · Ry · Rz, the port of
/// `lumit_core::camera::euler_xyz`, the one of its two ways of writing it
/// nearer [near], wound on by whole turns.
Vec3 eulerXyz(List<double> m, Vec3 near) {
  double deg(double r) => r * 180 / math.pi;
  final sinY = m[2].clamp(-1.0, 1.0);
  final ry = math.asin(sinY);
  final cosY = math.sqrt(math.max(0, 1 - sinY * sinY));
  final Vec3 one = cosY < 1e-9
      ? (deg(math.atan2(m[7], m[4])), deg(ry), 0)
      : (deg(math.atan2(-m[5], m[8])), deg(ry), deg(math.atan2(-m[1], m[0])));
  final other = (one.$1 + 180, 180 - one.$2, one.$3 + 180);
  return _nearer(one, other, near);
}

/// [one] or [other], each wound on by whole turns towards [near], whichever
/// is nearer it.
Vec3 _nearer(Vec3 one, Vec3 other, Vec3 near) {
  double wind(double a, double n) => a - ((a - n) / 360).round() * 360;
  Vec3 wound(Vec3 v) =>
      (wind(v.$1, near.$1), wind(v.$2, near.$2), wind(v.$3, near.$3));
  double gap(Vec3 v) =>
      math.pow(v.$1 - near.$1, 2) +
      math.pow(v.$2 - near.$2, 2) +
      math.pow(v.$3 - near.$3, 2).toDouble();
  final (a, b) = (wound(one), wound(other));
  return gap(b) < gap(a) ? b : a;
}

/// `a · b`, both row by row.
List<double> mul3(List<double> a, List<double> b) => [
      for (var row = 0; row < 3; row++)
        for (var col = 0; col < 3; col++)
          a[row * 3] * b[col] +
              a[row * 3 + 1] * b[3 + col] +
              a[row * 3 + 2] * b[6 + col],
    ];

/// A turn undone: its rows and columns swapped.
List<double> transpose3(List<double> m) =>
    [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]];

/// [v] turned by [m], row by row.
Vec3 turned3(List<double> m, Vec3 v) => _apply(m, v);

/// The turn of [radians] about [axis], row by row.
List<double> turnAbout(Vec3 axis, double radians) {
  final (x, y, z) = _unit(axis);
  final s = math.sin(radians), c = math.cos(radians), k = 1 - c;
  return [
    c + x * x * k, x * y * k - z * s, x * z * k + y * s, //
    y * x * k + z * s, c + y * y * k, y * z * k - x * s,
    z * x * k - y * s, z * y * k + x * s, c + z * z * k,
  ];
}

/// The Orientation that turns a layer by [radians] more about [axis] in its
/// parent's space, leaving X, Y and Z Rotation as they are. [outside] is the
/// whole turn outside those, the Orientation and an aimed camera's aim, which
/// stays where it is. Nearest [near], or [orientation] itself.
Vec3 orientedAbout(List<double> outside, Vec3 orientation, Vec3 axis,
    double radians, {Vec3? near}) {
  final aim = mul3(outside, transpose3(turnOfAe(orientation)));
  final turned = mul3(transpose3(aim), mul3(turnAbout(axis, radians), outside));
  return eulerXyz(turned, near ?? orientation);
}

/// The axes X, Y and Z Rotation each turn about, as the compositor applies
/// them: Y Rotation about the parent's y, X Rotation about x after that,
/// and Rotation about the layer's own z. A ring drawn round one of these and
/// a drag on it change that one value and nothing else.
List<Vec3> rotationAxes(Vec3 deg, {bool ae = false}) {
  if (ae) {
    // After Effects' order: X about the parent's x, Y after it, Z last.
    final m = turnOfAe((deg.$1, deg.$2, 0));
    final x = deg.$1 * math.pi / 180;
    return [(1, 0, 0), (0, math.cos(x), math.sin(x)), (m[2], m[5], m[8])];
  }
  final y = deg.$2 * math.pi / 180;
  return [
    (math.cos(y), 0, -math.sin(y)),
    (0, 1, 0),
    localAxes((deg.$1, deg.$2, 0))[2],
  ];
}

/// X, Y and Z Rotation's turn, row by row, in Lumit's order or, on a layer
/// brought from After Effects, in its order.
List<double> rowsTurn(Vec3 deg, {bool ae = false}) =>
    ae ? turnOfAe(deg) : turnOf(deg);

/// X, Y and Z Rotation turned by [radians] more about [axis] in the parent's
/// space, [outside] (the Orientation and any aim) staying as it is. Nearest
/// [near], or [rows] itself.
Vec3 rowsAbout(List<double> outside, Vec3 rows, Vec3 axis, double radians,
    {bool ae = false, Vec3? near}) {
  // The same turn seen inside the outside turn.
  final inside = turned3(transpose3(outside), axis);
  if (!ae) return turnedAbout(rows, inside, radians, near: near);
  return eulerXyz(mul3(turnAbout(inside, radians), turnOfAe(rows)), near ?? rows);
}

/// [deg] turned by [radians] about [axis] in the parent's space, as X, Y and
/// Z Rotation again. A turn can be written two ways, and the one nearer
/// [near] (the drag's last answer), wound on by whole turns, comes back, so
/// the values never jump part way through a drag.
Vec3 turnedAbout(Vec3 deg, Vec3 axis, double radians, {Vec3? near}) {
  final one = eulerYxz(mul3(turnAbout(axis, radians), turnOf(deg)));
  final other = (180 - one.$1, one.$2 + 180, one.$3 + 180);
  return _nearer(one, other, near ?? deg);
}

/// The turn and scale [map]'s parents put on its Position space, row by
/// row: nothing with no parent.
List<double> _parentLinear(ViewerLayerMap map) {
  final m = map.depth?.parent?.storage;
  if (m == null) return const [1, 0, 0, 0, 1, 0, 0, 0, 1];
  return [m[0], m[4], m[8], m[1], m[5], m[9], m[2], m[6], m[10]];
}

Vec3 _apply(List<double> m, Vec3 v) => (
      m[0] * v.$1 + m[1] * v.$2 + m[2] * v.$3,
      m[3] * v.$1 + m[4] * v.$2 + m[5] * v.$3,
      m[6] * v.$1 + m[7] * v.$2 + m[8] * v.$3,
    );

/// A move [d] in [map]'s parent space, as the move it makes in the comp.
Vec3 parentToComp(ViewerLayerMap map, Vec3 d) => _apply(_parentLinear(map), d);

/// A move [d] in the comp, as the move it takes in [map]'s parent space.
/// Null where a parent is scaled to nothing.
Vec3? compToParent(ViewerLayerMap map, Vec3 d) {
  final inv = _inverse3(_parentLinear(map));
  return inv == null ? null : _apply(inv, d);
}

/// The X, Y and Z Rotation that turn a layer under [placed]'s parents to face
/// the way a layer under [surface]'s parents, turned by [deg], faces in the
/// comp. Each side's Orientation and rotation order count, as the compositor
/// places them. The parents' scale is left out, and of the two ways to write
/// the turn the one nearer [near] comes back.
Vec3 rotationMatching(
    ViewerLayerMap surface, Vec3 deg, ViewerLayerMap placed, Vec3 near) {
  List<double> turnOnly(List<double> m) {
    final out = [...m];
    for (var col = 0; col < 3; col++) {
      final l = math.sqrt(m[col] * m[col] +
          m[3 + col] * m[3 + col] +
          m[6 + col] * m[6 + col]);
      if (l < 1e-12) return const [1, 0, 0, 0, 1, 0, 0, 0, 1];
      for (var row = 0; row < 3; row++) {
        out[row * 3 + col] = m[row * 3 + col] / l;
      }
    }
    return out;
  }

  List<double> times(List<double> a, List<double> b) => [
        for (var row = 0; row < 3; row++)
          for (var col = 0; col < 3; col++)
            a[row * 3] * b[col] +
                a[row * 3 + 1] * b[3 + col] +
                a[row * 3 + 2] * b[6 + col],
      ];
  List<double> outside(ViewerLayerMap map) =>
      map.depth?.outside ?? const [1, 0, 0, 0, 1, 0, 0, 0, 1];
  final world = times(
      times(turnOnly(_parentLinear(surface)), outside(surface)),
      rowsTurn(deg, ae: surface.depth?.aeOrder ?? false));
  final p = times(turnOnly(_parentLinear(placed)), outside(placed));
  final back = [p[0], p[3], p[6], p[1], p[4], p[7], p[2], p[5], p[8]];
  final rows = times(back, world);
  if (placed.depth?.aeOrder ?? false) return eulerXyz(rows, near);
  final one = eulerYxz(rows);
  final other = (180 - one.$1, one.$2 + 180, one.$3 + 180);
  double wind(double a, double to) => a - ((a - to) / 360).round() * 360;
  Vec3 wound(Vec3 v) =>
      (wind(v.$1, near.$1), wind(v.$2, near.$2), wind(v.$3, near.$3));
  double gap(Vec3 v) =>
      math.pow(v.$1 - near.$1, 2) +
      math.pow(v.$2 - near.$2, 2) +
      math.pow(v.$3 - near.$3, 2).toDouble();
  final (a, b) = (wound(one), wound(other));
  return gap(b) < gap(a) ? b : a;
}

/// What part of the handles a press is on: the centre ring moves across
/// the screen.
enum GizmoKind { arrow, plane, ring, screen }

/// A part and its axis: 0, 1, 2 for x, y and z. A plane is named for the
/// axis it does not move along, so plane 2 is the XY square.
typedef GizmoPick = ({GizmoKind kind, int axis});

/// One set of 3D handles: where it sits, which way its axes point, and how
/// the picture sees it.
class Gizmo3d {
  /// Parent space `(x, y, z, 1)` to comp pixels `(X, Y, W)`, three rows of
  /// four, and the placement of comp pixels on screen.
  final List<double> rows;
  final Offset origin;
  final double viewScale;

  /// The handles' centre, in parent space.
  final Vec3 at;

  /// The arrows' directions, and the axes the rings turn about, unit long in
  /// parent space.
  final List<Vec3> axes;
  final List<Vec3> turns;

  /// The screen's right, down and away, in parent space.
  final Vec3 viewX, viewY, viewZ;

  /// How far in parent space an arrow reaches.
  final double unit;

  Gizmo3d._(this.rows, this.origin, this.viewScale, this.at, this.axes,
      this.turns, this.viewX, this.viewY, this.viewZ, this.unit);

  /// The handles of [map]'s layer at [at] in its parent's space, null where
  /// that point is behind the eye. [local] are the axes Local mode points
  /// along, and [turns] the ones the rings turn about there.
  static Gizmo3d? of(
    ViewerLayerMap map, {
    required Vec3 at,
    required List<Vec3> local,
    List<Vec3>? turns,
    required GizmoAxes mode,
  }) {
    if (!map.projected) return null;
    final rows = map.parentRows;
    final view = screenAxes(rows, at);
    if (view == null) return null;
    final (vx, vy, vz, perScreen) = view;
    final perUnit = map.viewScale * perScreen;
    if (perUnit < 1e-12) return null;
    final axes = switch (mode) {
      GizmoAxes.local => local,
      GizmoAxes.world => _worldAxes(map),
      GizmoAxes.view => [vx, vy, vz],
    };
    return Gizmo3d._(
      rows,
      map.origin,
      map.viewScale,
      at,
      [for (final a in axes) _unit(a)],
      mode == GizmoAxes.local
          ? [for (final a in turns ?? local) _unit(a)]
          : [for (final a in axes) _unit(a)],
      vx,
      vy,
      vz,
      gizmoArrowLength / perUnit,
    );
  }

  /// How the screen's right, down and away run through [at] in the space
  /// [rows] take in, unit long, and how many comp pixels on screen a unit to
  /// the right is. Through a camera right and down keep the depth, so a move
  /// across the view stays the same size, and away is the camera's forward.
  /// Flat rows, held while the engine cannot say, have no forward, so there
  /// the axes are measured from how the picture moves round [at]. Null behind
  /// the eye.
  static (Vec3, Vec3, Vec3, double)? screenAxes(List<double> rows, Vec3 at) {
    final w = _row(rows, 2, at, 1);
    if (w <= viewerNear) return null;
    final x = _row(rows, 0, at, 1) / w, y = _row(rows, 1, at, 1) / w;
    Vec3 row(int r) => (rows[r * 4], rows[r * 4 + 1], rows[r * 4 + 2]);
    final (r0, r1, r2) = (row(0), row(1), row(2));
    // How the picture's x and y change as [at] moves.
    final j0 = _times(_plus(r0, _times(r2, -x)), 1 / w);
    final j1 = _times(_plus(r1, _times(r2, -y)), 1 / w);
    Vec3 signed(Vec3 v, Vec3 by) => _dot(v, by) < 0 ? _times(v, -1) : v;
    final deep = _dot(r2, r2) > 1e-18;
    final vz = deep ? _unit(r2) : _unit(_cross(j0, j1));
    final vx = signed(_unit(deep ? _cross(r1, r2) : _cross(j1, vz)), j0);
    final vy = signed(_unit(deep ? _cross(r2, r0) : _cross(vz, j0)), j1);
    return (vx, vy, vz, _dot(j0, vx).abs());
  }

  /// The comp's own axes in the parent's space.
  static List<Vec3> _worldAxes(ViewerLayerMap map) => [
        for (final axis in const [
          (1.0, 0.0, 0.0),
          (0.0, 1.0, 0.0),
          (0.0, 0.0, 1.0)
        ])
          compToParent(map, axis) ?? axis,
      ];

  /// [p] in parent space on screen, and its depth over the zoom.
  (Offset, double) project(Vec3 p) {
    final w = _row(rows, 2, p, 1);
    final safe = w.abs() < 1e-9 ? 1e-9 : w;
    return (
      origin +
          Offset(_row(rows, 0, p, 1) / safe, _row(rows, 1, p, 1) / safe) *
              viewScale,
      w
    );
  }

  /// How far in front of the eye the handles' centre is.
  late final double _depth = _row(rows, 2, at, 1);

  /// The deepest a drag may take them: ten times as deep, and never less
  /// than ten camera zooms, so a layer near the eye can still go far.
  late final double _limit = _deepest * math.max(_depth, 1);

  late final Offset centre = project(at).$1;

  /// Each arrow's tip, null where the arrow points at the eye or past it.
  late final List<Offset?> tips = [
    for (final a in axes)
      switch (project(_plus(at, _times(a, unit)))) {
        (final tip, final w)
            when w > viewerNear &&
                (tip - centre).distance > gizmoArrowLength * 0.2 =>
          tip,
        _ => null,
      },
  ];

  /// Each plane's square, null where it is seen edge on.
  late final List<List<Offset>?> squares = [
    for (var i = 0; i < 3; i++) _square(i),
  ];

  List<Offset>? _square(int normal) {
    final a = axes[(normal + 1) % 3], b = axes[(normal + 2) % 3];
    final corners = <Offset>[];
    for (final (u, v) in const [
      (_squareFrom, _squareFrom),
      (_squareTo, _squareFrom),
      (_squareTo, _squareTo),
      (_squareFrom, _squareTo),
    ]) {
      final (p, w) =
          project(_plus(at, _plus(_times(a, u * unit), _times(b, v * unit))));
      if (w <= viewerNear) return null;
      corners.add(p);
    }
    var area = 0.0;
    for (var i = 0; i < 4; i++) {
      final p = corners[i], q = corners[(i + 1) % 4];
      area += p.dx * q.dy - q.dx * p.dy;
    }
    const side = gizmoArrowLength * (_squareTo - _squareFrom);
    return area.abs() / 2 < side * side * 0.15 ? null : corners;
  }

  /// The plane a ring lies in, two unit axes whose turn from the first to
  /// the second is a positive turn about the ring's axis.
  (Vec3, Vec3) _ringPlane(int axis) {
    final a = turns[axis];
    var e1 = axes[(axis + 1) % 3];
    e1 = _plus(e1, _times(a, -_dot(a, e1)));
    if (_dot(e1, e1) < 1e-9) {
      e1 = axes[(axis + 2) % 3];
      e1 = _plus(e1, _times(a, -_dot(a, e1)));
    }
    e1 = _unit(e1);
    return (e1, _cross(a, e1));
  }

  double get _ringReach => unit * gizmoRingRadius / gizmoArrowLength;

  /// Ring [axis]'s point at [theta] radians round it, in parent space.
  Vec3 ringPoint(int axis, double theta) {
    final (e1, e2) = _ringPlane(axis);
    final r = _ringReach;
    return _plus(
        at,
        _plus(
            _times(e1, r * math.cos(theta)), _times(e2, r * math.sin(theta))));
  }

  /// The same on screen, null behind the eye.
  Offset? _ringAt(int axis, double theta) {
    final (p, w) = project(ringPoint(axis, theta));
    return w > viewerNear ? p : null;
  }

  /// Each ring as the runs of it in front of the eye.
  late final List<List<List<Offset>>> rings = [
    for (var i = 0; i < 3; i++) _ring(i),
  ];

  List<List<Offset>> _ring(int axis) {
    final runs = <List<Offset>>[];
    var run = <Offset>[];
    for (var k = 0; k <= _ringSteps; k++) {
      final p = _ringAt(axis, k * 2 * math.pi / _ringSteps);
      if (p == null) {
        if (run.length > 1) runs.add(run);
        run = [];
      } else {
        run.add(p);
      }
    }
    if (run.length > 1) runs.add(run);
    return runs;
  }

  /// The part under [point]: a ring with the Rotation tool, otherwise a
  /// square before an arrow, as the squares sit between the arrows.
  GizmoPick? pick(Offset point, {required bool turning}) {
    GizmoPick? best;
    var nearest = gizmoReach;
    if (turning) {
      for (var i = 0; i < 3; i++) {
        for (final run in rings[i]) {
          for (var k = 0; k + 1 < run.length; k++) {
            final d = _toSegment(point, run[k], run[k + 1]);
            if (d < nearest) {
              nearest = d;
              best = (kind: GizmoKind.ring, axis: i);
            }
          }
        }
      }
      return best;
    }
    for (var i = 0; i < 3; i++) {
      final square = squares[i];
      if (square != null && _inside(point, square)) {
        return (kind: GizmoKind.plane, axis: i);
      }
    }
    if (((point - centre).distance - gizmoScreenRing).abs() < 4) {
      return (kind: GizmoKind.screen, axis: 2);
    }
    for (var i = 0; i < 3; i++) {
      final tip = tips[i];
      if (tip == null) continue;
      final d = _toSegment(point, centre + (tip - centre) * 0.25, tip);
      if (d < nearest) {
        nearest = d;
        best = (kind: GizmoKind.arrow, axis: i);
      }
    }
    return best;
  }

  /// How far along [d] from [at] the screen point [screen] is, as the point
  /// on the axis line nearest it on screen. Null where the line runs into
  /// the eye, or off towards its vanishing point past [_limit].
  double? _along(Vec3 d, Offset screen) {
    final c = (screen - origin) / viewScale;
    final a = (_row(rows, 0, at, 1), _row(rows, 1, at, 1), _depth);
    final b = (_row(rows, 0, d, 0), _row(rows, 1, d, 0), _row(rows, 2, d, 0));
    // The line on screen, and the point on it nearest the pointer.
    final l = _cross(a, b);
    final den = l.$1 * l.$1 + l.$2 * l.$2;
    if (den < 1e-18) return null;
    final off = (l.$1 * c.dx + l.$2 * c.dy + l.$3) / den;
    final qx = c.dx - l.$1 * off, qy = c.dy - l.$2 * off;
    // Then how far along the line that is: q · (a.w + t b.w) = a + t b.
    final mx = b.$1 - qx * b.$3, my = b.$2 - qy * b.$3;
    final nx = qx * a.$3 - a.$1, ny = qy * a.$3 - a.$2;
    final mm = mx * mx + my * my;
    if (mm < 1e-18) return null;
    final t = (mx * nx + my * ny) / mm;
    final w = a.$3 + t * b.$3;
    return w > viewerNear && w <= _limit ? t : null;
  }

  /// Where the pointer's ray at [screen] meets the plane through [at] across
  /// [e1] and [e2], as amounts of each, null where it meets it behind the
  /// eye, not at all, or off towards its horizon past [_limit].
  (double, double)? _onPlane(Vec3 e1, Vec3 e2, Offset screen) {
    final c = (screen - origin) / viewScale;
    final inv = _inverse3([
      for (var r = 0; r < 3; r++) ...[
        _row(rows, r, e1, 0),
        _row(rows, r, e2, 0),
        _row(rows, r, at, 1)
      ],
    ]);
    if (inv == null) return null;
    final u = inv[0] * c.dx + inv[1] * c.dy + inv[2];
    final v = inv[3] * c.dx + inv[4] * c.dy + inv[5];
    // s is one over the depth there.
    final s = inv[6] * c.dx + inv[7] * c.dy + inv[8];
    if (s <= 1e-12 || 1 / s > _limit) return null;
    return (u / s, v / s);
  }

  /// The move a drag from [from] to [to] on arrow [axis] makes, in parent
  /// space, in whole steps of [step] when one is given.
  Vec3? along(int axis, Offset from, Offset to, {double? step}) {
    final a = _along(axes[axis], from), b = _along(axes[axis], to);
    if (a == null || b == null) return null;
    return _times(axes[axis], _snapped(b - a, step));
  }

  /// The move a drag from [from] to [to] on square [normal] makes.
  Vec3? across(int normal, Offset from, Offset to, {double? step}) =>
      _across(axes[(normal + 1) % 3], axes[(normal + 2) % 3], from, to,
          step: step);

  /// The move a drag makes across the screen at the handles' depth.
  Vec3? onScreen(Offset from, Offset to) => _across(viewX, viewY, from, to);

  Vec3? _across(Vec3 e1, Vec3 e2, Offset from, Offset to, {double? step}) {
    final a = _onPlane(e1, e2, from), b = _onPlane(e1, e2, to);
    if (a == null || b == null) return null;
    return _plus(_times(e1, _snapped(b.$1 - a.$1, step)),
        _times(e2, _snapped(b.$2 - a.$2, step)));
  }

  /// The angle round ring [axis] the screen point [screen] is at, on the
  /// ring's own plane.
  double? ringAngle(int axis, Offset screen) {
    final (e1, e2) = _ringPlane(axis);
    final p = _onPlane(e1, e2, screen);
    return p == null ? null : math.atan2(p.$2, p.$1);
  }

  /// Whether ring [axis] is seen so nearly edge on that its plane is no use
  /// to measure on.
  bool ringEdgeOn(int axis) {
    final a = _ringAt(axis, 0), b = _ringAt(axis, math.pi / 2);
    if (a == null || b == null) return true;
    final u = a - centre, v = b - centre;
    return (u.dx * v.dy - u.dy * v.dx).abs() <
        gizmoRingRadius * gizmoRingRadius * 0.2;
  }

  /// Which way ring [axis] runs on screen near [screen], as a positive turn
  /// goes, one pixel long.
  Offset ringTangent(int axis, Offset screen) {
    var best = 0.0;
    var nearest = double.infinity;
    for (var k = 0; k < _ringSteps; k++) {
      final theta = k * 2 * math.pi / _ringSteps;
      final p = _ringAt(axis, theta);
      if (p == null) continue;
      final d = (p - screen).distance;
      if (d < nearest) {
        nearest = d;
        best = theta;
      }
    }
    final a = _ringAt(axis, best - 0.05), b = _ringAt(axis, best + 0.05);
    if (a == null || b == null) return Offset.zero;
    final t = b - a;
    return t.distance < 1e-9 ? Offset.zero : t / t.distance;
  }
}

double _snapped(double v, double? step) =>
    step == null ? v : (v / step).roundToDouble() * step;

/// A drag round one ring: the angle swept so far, in radians. Measured on the
/// ring's own plane while it can be seen, so the grabbed point stays under
/// the pointer, and along the ring's run on screen when it is edge on.
class RingTurn {
  final Gizmo3d gizmo;
  final int axis;
  final Offset from;
  late final bool _edgeOn = gizmo.ringEdgeOn(axis);
  late final Offset _tangent = gizmo.ringTangent(axis, from);
  late double? _last = gizmo.ringAngle(axis, from);
  double _swept = 0;

  RingTurn(this.gizmo, this.axis, this.from);

  double sweptTo(Offset to) {
    if (_edgeOn || _last == null) {
      final d = to - from;
      return (d.dx * _tangent.dx + d.dy * _tangent.dy) / gizmoRingRadius;
    }
    final now = gizmo.ringAngle(axis, to);
    if (now != null) {
      // Step by step, so a drag can go round more than once.
      var step = now - _last!;
      if (step > math.pi) step -= 2 * math.pi;
      if (step < -math.pi) step += 2 * math.pi;
      _swept += step;
      _last = now;
    }
    return _swept;
  }
}

/// The distance from [p] to the segment [a]-[b].
double _toSegment(Offset p, Offset a, Offset b) {
  final ab = b - a;
  final len = ab.dx * ab.dx + ab.dy * ab.dy;
  final t = len < 1e-12
      ? 0.0
      : (((p.dx - a.dx) * ab.dx + (p.dy - a.dy) * ab.dy) / len).clamp(0.0, 1.0);
  return (a + ab * t - p).distance;
}

/// Whether [p] is inside the convex quad [q].
bool _inside(Offset p, List<Offset> q) {
  var sign = 0;
  for (var i = 0; i < q.length; i++) {
    final a = q[i], b = q[(i + 1) % q.length];
    final cross = (b.dx - a.dx) * (p.dy - a.dy) - (b.dy - a.dy) * (p.dx - a.dx);
    final s = cross > 0 ? 1 : (cross < 0 ? -1 : 0);
    if (s == 0) continue;
    if (sign == 0) {
      sign = s;
    } else if (s != sign) {
      return false;
    }
  }
  return true;
}

/// The inverse of a three by three matrix, row by row, or null.
List<double>? _inverse3(List<double> m) {
  final a = m[4] * m[8] - m[5] * m[7];
  final b = m[5] * m[6] - m[3] * m[8];
  final c = m[3] * m[7] - m[4] * m[6];
  final det = m[0] * a + m[1] * b + m[2] * c;
  if (det.abs() < 1e-18) return null;
  final k = 1 / det;
  return [
    a * k,
    (m[2] * m[7] - m[1] * m[8]) * k,
    (m[1] * m[5] - m[2] * m[4]) * k,
    b * k,
    (m[0] * m[8] - m[2] * m[6]) * k,
    (m[2] * m[3] - m[0] * m[5]) * k,
    c * k,
    (m[1] * m[6] - m[0] * m[7]) * k,
    (m[0] * m[4] - m[1] * m[3]) * k,
  ];
}

/// Draw [gizmo]: its arrows, squares and centre ring, or with [turning] its
/// rings, each axis in its colour and the centre ring in [neutral]. [hot] is
/// the part under the pointer, drawn stronger, and [held] the one being
/// dragged, which is then all that is drawn.
void paintGizmo3d(
  Canvas canvas,
  Gizmo3d gizmo, {
  required bool turning,
  required Color neutral,
  required Color outline,
  GizmoPick? hot,
  GizmoPick? held,
}) {
  bool shows(GizmoKind kind, int axis) =>
      held == null || (held.kind == kind && held.axis == axis);
  bool lit(GizmoKind kind, int axis) =>
      (hot?.kind == kind && hot?.axis == axis) ||
      (held?.kind == kind && held?.axis == axis);
  Paint stroke(Color colour, double width) => Paint()
    ..color = colour
    ..style = PaintingStyle.stroke
    ..strokeWidth = width
    ..strokeCap = StrokeCap.round
    ..strokeJoin = StrokeJoin.round;

  if (turning) {
    for (var i = 0; i < 3; i++) {
      if (!shows(GizmoKind.ring, i)) continue;
      final width = lit(GizmoKind.ring, i) ? 3.0 : 1.6;
      for (final run in gizmo.rings[i]) {
        final path = Path()..addPolygon(run, false);
        canvas.drawPath(path, stroke(outline, width + 1.8));
        canvas.drawPath(path, stroke(gizmoAxisColours[i], width));
      }
    }
    return;
  }

  for (var i = 0; i < 3; i++) {
    final square = gizmo.squares[i];
    if (square == null || !shows(GizmoKind.plane, i)) continue;
    final path = Path()..addPolygon(square, true);
    canvas.drawPath(
        path,
        Paint()
          ..color = gizmoAxisColours[i]
              .withValues(alpha: lit(GizmoKind.plane, i) ? 0.7 : 0.35));
    canvas.drawPath(path, stroke(gizmoAxisColours[i], 1));
  }
  if (shows(GizmoKind.screen, 2)) {
    final width = lit(GizmoKind.screen, 2) ? 2.6 : 1.4;
    canvas.drawCircle(
        gizmo.centre, gizmoScreenRing, stroke(outline, width + 1.8));
    canvas.drawCircle(gizmo.centre, gizmoScreenRing, stroke(neutral, width));
  }
  // The arrow seen most end on is drawn first, so the others cross over it.
  final order = [0, 1, 2]
    ..sort((a, b) => _lengthOf(gizmo, a).compareTo(_lengthOf(gizmo, b)));
  for (final i in order) {
    final tip = gizmo.tips[i];
    if (tip == null || !shows(GizmoKind.arrow, i)) continue;
    final width = lit(GizmoKind.arrow, i) ? 3.0 : 2.0;
    final run = tip - gizmo.centre;
    final dir = run / run.distance;
    final side = Offset(-dir.dy, dir.dx);
    final base = tip - dir * 10;
    final head = Path()
      ..moveTo(tip.dx, tip.dy)
      ..lineTo(base.dx + side.dx * 4.5, base.dy + side.dy * 4.5)
      ..lineTo(base.dx - side.dx * 4.5, base.dy - side.dy * 4.5)
      ..close();
    canvas.drawLine(gizmo.centre, base, stroke(outline, width + 1.8));
    canvas.drawPath(head, stroke(outline, 1.8));
    canvas.drawLine(gizmo.centre, base, stroke(gizmoAxisColours[i], width));
    canvas.drawPath(head, Paint()..color = gizmoAxisColours[i]);
  }
}

double _lengthOf(Gizmo3d gizmo, int axis) {
  final tip = gizmo.tips[axis];
  return tip == null ? 0 : (tip - gizmo.centre).distance;
}

/// The numbers a drag has changed, by the pointer: one line a value, on a
/// small plate so they read over any picture.
void paintGizmoReadout(
  Canvas canvas,
  Offset pointer,
  List<String> lines, {
  required TextStyle style,
  required Color plate,
}) {
  if (lines.isEmpty) return;
  final text = TextPainter(
    text: TextSpan(text: lines.join('\n'), style: style),
    textDirection: TextDirection.ltr,
  )..layout();
  final at = pointer + const Offset(16, 16);
  final rect = Rect.fromLTWH(at.dx, at.dy, text.width + 10, text.height + 6);
  canvas.drawRRect(RRect.fromRectAndRadius(rect, const Radius.circular(3)),
      Paint()..color = plate);
  text.paint(canvas, at + const Offset(5, 3));
  text.dispose();
}

/// A change in a readout, signed, to one decimal.
String gizmoAmount(double v, {String unit = ''}) =>
    '${v < 0 ? '−' : '+'}${v.abs().toStringAsFixed(1)}$unit';
