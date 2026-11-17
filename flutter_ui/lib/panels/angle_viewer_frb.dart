// The angle viewer: the Viewer's source side showing every angle of a
// multicam comp in a grid, where it otherwise shows one footage item.
//
// **In plain terms.** The engine draws the grid as one picture, and it is the
// same surface as the footage view: the same header, the same strip and bar
// under it. This draws what the picture cannot say over it, the numbers, the
// names and the marks on the picture and sound angles, and says what a click
// means. While the playhead stands on a multicam clip of the fronted comp the
// viewer follows it: the grid is the clip's comp at the clip's moment, a click
// cuts to the angle, and there is no strip or bar because its time is the
// program's. Off a multicam clip it stands on its own frame, with the strip,
// the marks and Insert and Overwrite as the footage view has them, and a
// click picks the angle to place.
//
// **What a frame costs.** While playing, the clip under the playhead is read
// again every frame, so the marks move every frame. Only the overlay painter
// and the two small layers over it listen for that; the stage itself is
// rebuilt when what it is made of changes, which is rarely.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart' show ValueListenable, listEquals;
import 'package:flutter/services.dart' show HardwareKeyboard, KeyEvent;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/cut/multicam.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeRational;
import 'package:lumit_flutter/src/rust/api/footage.dart' show BridgeMediaInfo;
import 'package:lumit_flutter/src/rust/api/multicam.dart';
import 'package:lumit_flutter/src/rust/api/multicam/grid.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/welcome_frb.dart' show EmptyStageFrb;
import '../state/app_state.dart' show LumitState;
import '../state/timecode.dart';
import '../state/ui_state.dart';
import '../state/viewer_views.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart' show workAreaFrames;
import 'timeline_razor.dart' show razorCursorSize;
import 'viewer_menus.dart';
import 'viewer_panel_frb.dart';
import 'viewer_picture.dart'
    if (dart.library.js_interop) 'viewer_picture_web.dart';

/// The longest side the grid is asked for, in pixels.
const int _mostPixels = 2048;

/// The band under each angle's picture that carries its number and name.
const double _bandHeight = 18;

/// How wide the header has to be to show the pager's numbers.
const double _roomForPageNumbers = 360;

/// The speaker in the corner of an angle that is heard.
const double _speakerSize = 14;

class AngleStageFrb extends StatefulWidget {
  final ViewerSurface view;

  const AngleStageFrb({super.key, required this.view});

  @override
  State<AngleStageFrb> createState() => _AngleStageFrbState();
}

class _AngleStageFrbState extends State<AngleStageFrb>
    with SingleTickerProviderStateMixin {
  LumitUiState? _ui;
  StreamSubscription<Object?>? _changes;

  /// The comp's angles and facts, read when the document moves or the view
  /// shows another comp, and the grid they are laid out in on this page.
  BridgeMulticam? _info;
  String? _infoFor;
  BridgeAngleGrid? _grid;

  /// Whether the viewer was following a clip when the stage was last built:
  /// the one thing about the clip under the playhead that changes what the
  /// stage is made of.
  bool _wasFollowing = false;

  /// The outline of the angle just cut to, fading back over 180 ms.
  late final AnimationController _flash = AnimationController(
      vsync: this, duration: const Duration(milliseconds: 180));
  late final Animation<double> _fade =
      CurvedAnimation(parent: _flash, curve: Curves.easeOut);
  final ValueNotifier<UuidValue?> _flashed = ValueNotifier(null);

  /// The cell under the pointer, where it is, and whether Alt is held, which
  /// turns a cut into a switch and the blade back into the arrow. Notifiers,
  /// so a pointer move repaints the overlay and nothing else.
  final ValueNotifier<int?> _hover = ValueNotifier(null);
  final ValueNotifier<Offset?> _pointer = ValueNotifier(null);
  final ValueNotifier<bool> _alt = ValueNotifier(false);

  ViewerSurface get view => widget.view;

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_onKey);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (identical(ui, _ui)) return;
    _unbind();
    _ui = ui
      ..multicam.under.addListener(_onUnder)
      ..multicam.mode.addListener(_rebuild)
      ..multicam.page.addListener(_onPage)
      ..multicam.lastCut.addListener(_onCut)
      ..playing.addListener(_onPlaying)
      ..views.addListener(_onViews);
    _changes?.cancel();
    _changes = Provider.of<LumitState>(context, listen: false)
        .onChange
        .listen((_) => _readInfo());
    _readInfo();
  }

  void _unbind() {
    _ui
      ?..multicam.under.removeListener(_onUnder)
      ..multicam.mode.removeListener(_rebuild)
      ..multicam.page.removeListener(_onPage)
      ..multicam.lastCut.removeListener(_onCut)
      ..playing.removeListener(_onPlaying)
      ..views.removeListener(_onViews);
    _ui = null;
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    _unbind();
    _changes?.cancel();
    _flash.dispose();
    _flashed.dispose();
    _hover.dispose();
    _pointer.dispose();
    _alt.dispose();
    super.dispose();
  }

  void _rebuild() {
    if (mounted) setState(() {});
  }

  /// The clip under the playhead moved, as it does every frame while
  /// playing. The overlay listens for itself; the stage is rebuilt only when
  /// following starts or stops, which brings the strip and bar in or out.
  void _onUnder() {
    final ui = _ui;
    if (ui == null) return;
    if ((ui.anglesFollowing(view) != null) != _wasFollowing) _rebuild();
  }

  /// Another comp in the view is another set of angles to read.
  void _onViews() {
    if (_infoFor != view.itemId) _readInfo();
  }

  /// The comp's angles, read off the engine on an event and held for the
  /// paint: nothing is asked for in a build.
  void _readInfo() {
    final ui = _ui;
    if (ui == null) return;
    BridgeMulticam? info;
    try {
      info = ui.multicamOf(view)?.multicam();
    } catch (_) {
      info = null;
    }
    _infoFor = view.itemId;
    _layOut(info, ui.multicam.page.value);
  }

  void _onPage() {
    final ui = _ui;
    if (ui == null) return;
    _layOut(_info, ui.multicam.page.value);
    ui.askAngles(view);
  }

  void _layOut(BridgeMulticam? info, int page) {
    final grid =
        info == null ? null : angleGrid(angles: info.angles.length, page: page);
    if (info == _info && grid == _grid) return;
    if (!mounted) return;
    setState(() {
      _info = info;
      _grid = grid;
    });
  }

  /// While playing the engine draws the grid beside every frame; once it
  /// stops the viewer asks for the frame it stands on.
  void _onPlaying() {
    final ui = _ui;
    if (ui != null && !ui.playing.value) ui.askAngles(view, force: true);
  }

  void _onCut() {
    final cut = _ui?.multicam.lastCut.value;
    if (cut != null) _flashCell(cut.$1);
  }

  /// Flash [angle]'s outline, as a cut does. Nothing under reduced motion,
  /// the system's or Lumit's own None.
  void _flashCell(UuidValue angle) {
    if (!mounted ||
        MediaQuery.disableAnimationsOf(context) ||
        ThemeScope.of(context).motion.isStill) {
      return;
    }
    _flashed.value = angle;
    _flash.forward(from: 0);
  }

  /// Alt pressed or let go while the pointer rests on a cell changes what a
  /// click would do, so the pointer changes with it.
  bool _onKey(KeyEvent event) {
    _alt.value = HardwareKeyboard.instance.isAltPressed;
    return false;
  }

  /// The angles on this page, in reading order: what the digits reach.
  List<BridgeAngle> _pageAngles(BridgeMulticam info, BridgeAngleGrid grid) {
    final angles = info.angles;
    final start = grid.first.clamp(0, angles.length);
    return angles.sublist(start, (start + grid.count).clamp(start, angles.length));
  }

  /// A click on an angle. Following a clip it cuts, switches with Alt, paints
  /// the work area with Shift, or with Mod+Alt hears the angle as well. Over
  /// ordinary footage that one angle holds it puts another angle in step on
  /// the row above. On its own it picks the angle to place.
  void _tap(LumitUiState ui, BridgeAngle angle, BridgeMulticamAt? following) {
    final keys = HardwareKeyboard.instance;
    if (following != null) {
      if (following.plain) {
        if (keys.isAltPressed || keys.isShiftPressed || angle.sound) return;
        if (angle.id == following.picture) return;
        ui.multicam.overwriteAbove(angle.id);
        _flashCell(angle.id);
      } else if (keys.isAltPressed &&
          (keys.isControlPressed || keys.isMetaPressed)) {
        ui.multicam.hear(angle.id);
      } else if (keys.isShiftPressed) {
        final comp = ui.selectedComp;
        if (comp == null) return;
        final work = workAreaFrames(comp);
        ui.multicam.paint(angle.id, work.start, work.end);
      } else if (keys.isAltPressed) {
        ui.multicam.switchTo(angle.id);
      } else {
        ui.multicam.cutTo(angle.id);
      }
      return;
    }
    final mode = ui.multicam.mode.value;
    // A sound angle has no picture, so it can only be the sound pick.
    if (angle.sound || mode == BridgeAngleSwitch.sound) {
      view.angleSound = angle.id;
    } else if (mode == BridgeAngleSwitch.picture) {
      view.anglePicture = angle.id;
    } else {
      view.anglePicture = angle.id;
      view.angleSound = angle.id;
    }
    ui.views.touch();
  }

  /// The multicam comp as the strip and bar know an item: its size, rate and
  /// length, silent, counted from nought.
  BridgeMediaInfo _facts(BridgeMulticam info, int frames) => BridgeMediaInfo(
        width: info.width,
        height: info.height,
        fpsNum: info.rateNum,
        fpsDen: info.rateDen,
        duration: BridgeRational(num: frames * info.rateDen, den: info.rateNum),
        channels: 0,
        sampleRate: 0,
        audioStreams: 0,
        isStill: false,
        startFrame: 0,
        startDropFrame: false,
      );

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context);
    final info = _info;
    final grid = _grid;
    // Gone from the project, or not a multicam comp any more.
    if (info == null || grid == null || info.width <= 0 || info.height <= 0) {
      return const EmptyStageFrb();
    }
    final t = ThemeScope.of(context).theme;
    final round = t.tokens.roomed;
    final following = ui.anglesFollowing(view);
    _wasFollowing = following != null;
    final mode = ui.multicam.mode.value;
    final page = ui.multicam.page.value;
    final angles = _pageAngles(info, grid);
    final frames = info.frames < 1 ? 1 : info.frames;

    Widget modeMark(BridgeAngleSwitch m, LumitIcon icon, String tip,
            String action) =>
        viewerBarMark(
          key: ValueKey('angles-mode-${m.name}'),
          icon: icon,
          colour: mode == m ? t.accent : t.textMuted,
          onPressed: () => ui.multicam.mode.value = m,
          tip: tip,
          action: action,
        );
    final header = Container(
      key: const ValueKey('angles-view-header'),
      height: viewerStripHeightFor(t),
      decoration: viewerStripDecoration(t, round),
      padding: EdgeInsets.symmetric(horizontal: viewerStripPaddingFor(t)),
      child: LayoutBuilder(builder: (context, box) {
        final numbers = box.maxWidth >= _roomForPageNumbers;
        return Row(
          children: [
            Text(t.kickerCase(l10n.anglesTitle), style: t.kickerOn),
            const SizedBox(width: viewerBarGap),
            Expanded(
              child: Text(
                info.name,
                style: t.small.copyWith(color: t.textSecondary),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            // Which halves a click switches: both, the picture, the sound.
            modeMark(BridgeAngleSwitch.both, LumitIcon.footage,
                l10n.multicamBoth, 'multicam.mode.both'),
            viewerBarGapBox(viewerHeaderGap),
            modeMark(BridgeAngleSwitch.picture, LumitIcon.eye,
                l10n.multicamPicture, 'multicam.mode.picture'),
            viewerBarGapBox(viewerHeaderGap),
            modeMark(BridgeAngleSwitch.sound, LumitIcon.audio,
                l10n.multicamSound, 'multicam.mode.sound'),
            if (grid.pages > 1) ...[
              const SizedBox(width: viewerBarGap),
              viewerBarMark(
                key: const ValueKey('angles-page-prev'),
                icon: LumitIcon.previousFrame,
                colour: page > 0 ? t.textMuted : t.textDisabled,
                onPressed: page > 0
                    ? () =>
                        ui.multicam.turnPage(-1, angles: info.angles.length)
                    : null,
                tip: l10n.anglesPrevious,
                action: 'multicam.page.prev',
              ),
              if (numbers)
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 4),
                  child: Text(
                    '${page + 1}/${grid.pages}',
                    style: t.mono.copyWith(
                        fontSize: viewerTimecodeSize, color: t.textSecondary),
                  ),
                )
              else
                viewerBarGapBox(viewerHeaderGap),
              viewerBarMark(
                key: const ValueKey('angles-page-next'),
                icon: LumitIcon.nextFrame,
                colour:
                    page < grid.pages - 1 ? t.textMuted : t.textDisabled,
                onPressed: page < grid.pages - 1
                    ? () => ui.multicam.turnPage(1, angles: info.angles.length)
                    : null,
                tip: l10n.anglesNext,
                action: 'multicam.page.next',
              ),
            ],
            // Let the angles go and the composition has the pane back.
            if (ui.sourceBesideComposition(view)) ...[
              const SizedBox(width: viewerBarGap),
              LumitTooltip(
                message: l10n.close,
                child: HouseButton(
                  key: const ValueKey('angles-view-close'),
                  small: true,
                  frameless: true,
                  padding: EdgeInsets.zero,
                  onPressed: () => ui.clearSourceView(view),
                  child: SizedBox(
                    width: iconSize + 4,
                    height: iconSize + 4,
                    child: Center(
                      child: glyph.LumitIcon(LumitIcons.close,
                          colour: t.textSecondary),
                    ),
                  ),
                ),
              ),
            ],
          ],
        );
      }),
    );

    // The marks: the clip's angles while following, else the picks, else the
    // comp's own starting angles. Read when painted, not when built, since
    // the clip under the playhead moves every frame while playing.
    _Marks marks() {
      final at = ui.anglesFollowing(view);
      if (at != null) {
        final times = <UuidValue, BridgeAngleTime>{};
        final list = at.multicam.angles;
        for (var i = 0; i < math.min(list.length, at.times.length); i++) {
          final time = at.times[i];
          if (time != null) times[list[i].id] = time;
        }
        return _Marks(
          picture: at.picture,
          sound: at.sound,
          also: at.also,
          plain: at.plain,
          following: true,
          times: times,
        );
      }
      return _Marks(
        picture: view.anglePicture ?? info.picture,
        sound: view.angleSound ?? info.sound,
        also: const [],
        plain: false,
        following: false,
        times: const {},
      );
    }

    final picture = ColoredBox(
      color: viewerSurroundFor(t, themed: ui.workspace.themedViewerSurround),
      child: LayoutBuilder(
        key: const ValueKey('angles-view-picture'),
        builder: (context, box) {
          final size = Size(box.maxWidth, box.maxHeight);
          // The grid fills the area, drawn at the area's own pixels, capped.
          final ratio = MediaQuery.devicePixelRatioOf(context);
          var w = size.width * ratio, h = size.height * ratio;
          final longer = math.max(w, h);
          if (longer > _mostPixels) {
            w *= _mostPixels / longer;
            h *= _mostPixels / longer;
          }
          final pixels = (w.round(), h.round());
          // Reported after the frame: an ask from a layout would rebuild the
          // tree it is measuring.
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (mounted) ui.reportAnglesCanvas(view, pixels.$1, pixels.$2);
          });
          final cells = _cellsOf(size, grid);
          final pictures = [
            for (final cell in cells) _letterbox(cell, info.width, info.height),
          ];
          final shown = math.min(cells.length, angles.length);
          int? cellAt(Offset p) {
            for (var i = 0; i < shown; i++) {
              if (cells[i].contains(p)) return i;
            }
            return null;
          }

          return Stack(
            children: [
              Positioned.fill(
                child: ValueListenableBuilder<int?>(
                  valueListenable: ui.textureOf(view.engineId),
                  builder: (context, textureId, _) => textureId == null
                      ? const SizedBox.expand()
                      : pictureChannelFilter(
                          view.channel, viewerPicture(textureId)),
                ),
              ),
              Positioned.fill(
                child: IgnorePointer(
                  child: CustomPaint(
                    painter: _AngleGridPainter(
                      cells: cells,
                      pictures: pictures,
                      angles: angles,
                      marks: marks,
                      mode: mode,
                      hover: _hover,
                      flashed: _flashed,
                      fade: _fade,
                      under: ui.multicam.under,
                      timecodes: ui.multicam.timecodes,
                      theme: t,
                    ),
                  ),
                ),
              ),
              // The speaker glyphs, from the icon set: muted in the middle of
              // a sound angle's dark cell, which is what the cell is, and on a
              // chip in the corner of every angle that is heard. Rebuilt with
              // the clip under the playhead, which is this layer alone.
              Positioned.fill(
                child: IgnorePointer(
                  child: ValueListenableBuilder<BridgeMulticamAt?>(
                    valueListenable: ui.multicam.under,
                    builder: (context, _, __) {
                      final m = marks();
                      final strong = mode != BridgeAngleSwitch.picture;
                      return Stack(
                        children: [
                          for (var i = 0; i < shown; i++) ...[
                            if (angles[i].sound)
                              Positioned.fromRect(
                                rect: Rect.fromCenter(
                                    center: cells[i].center,
                                    width: iconSize * 2,
                                    height: iconSize * 2),
                                child: glyph.LumitIcon(LumitIcons.audio,
                                    size: iconSize * 2, colour: t.textMuted),
                              ),
                            if (m.hears(angles[i].id))
                              Positioned(
                                right: size.width - pictures[i].right + 4,
                                top: pictures[i].top + 4,
                                child: DecoratedBox(
                                  decoration: BoxDecoration(
                                    color: t.scrim,
                                    borderRadius: BorderRadius.circular(
                                        t.tokens.controlRadius),
                                  ),
                                  child: Padding(
                                    padding: const EdgeInsets.all(3),
                                    child: glyph.LumitIcon(LumitIcons.audio,
                                        size: _speakerSize,
                                        colour: strong
                                            ? t.accent
                                            : t.textPrimary),
                                  ),
                                ),
                              ),
                          ],
                        ],
                      );
                    },
                  ),
                ),
              ),
              // The pointer: where it is, which cell it is on, and the blade
              // where a click would cut, as the Cut timeline draws its razor.
              // Its own layer, rebuilt as the pointer moves and nothing else.
              Positioned.fill(
                child: ListenableBuilder(
                  listenable: Listenable.merge(
                      [_hover, _pointer, _alt, ui.multicam.under]),
                  builder: (context, _) {
                    final at = ui.anglesFollowing(view);
                    final cuts = at != null &&
                        !at.plain &&
                        !_alt.value &&
                        _hover.value != null;
                    final pointer = _pointer.value;
                    return MouseRegion(
                      // The blade is drawn below, so the arrow goes while it
                      // shows.
                      cursor: cuts
                          ? SystemMouseCursors.none
                          : MouseCursor.defer,
                      onHover: (e) {
                        _pointer.value = e.localPosition;
                        _hover.value = cellAt(e.localPosition);
                        _alt.value = HardwareKeyboard.instance.isAltPressed;
                      },
                      onExit: (_) {
                        _pointer.value = null;
                        _hover.value = null;
                      },
                      child: GestureDetector(
                        behavior: HitTestBehavior.opaque,
                        onTapUp: (up) {
                          final i = cellAt(up.localPosition);
                          if (i != null) _tap(ui, angles[i], at);
                        },
                        child: Stack(
                          children: [
                            if (cuts && pointer != null)
                              Positioned(
                                left: pointer.dx - razorCursorSize / 2,
                                top: pointer.dy - razorCursorSize / 2,
                                child: IgnorePointer(
                                  // The halo copy under the ink, so it reads
                                  // over any picture.
                                  child: Stack(
                                    children: [
                                      Transform.translate(
                                        offset: const Offset(1, 1),
                                        child: lumitIcon(LumitIcon.razor,
                                            size: razorCursorSize,
                                            color: t.surface0),
                                      ),
                                      lumitIcon(LumitIcon.razor,
                                          size: razorCursorSize,
                                          color: t.textPrimary),
                                    ],
                                  ),
                                ),
                              ),
                          ],
                        ),
                      ),
                    );
                  },
                ),
              ),
              const Positioned.fill(child: ViewerGpuMismatchNotice()),
            ],
          );
        },
      ),
    );

    // The multicam comp's own rows, then the source side's.
    Widget menu(Widget child) => ViewerRightClick(
          rows: (close) => [
            viewerRow(l10n.multicamOpen, () {
              final mc = ui.multicamOf(view);
              if (mc != null) ui.setSelectedComp(mc);
            }, close, id: 'angles-menu-open'),
            viewerRow(l10n.anglesTimecode, () {
              final timecodes = ui.multicam.timecodes;
              timecodes.value = !timecodes.value;
              // The clip under the playhead is read again with the times.
              ui.multicam.follow();
            }, close,
                id: 'angles-menu-timecode',
                ticked: ui.multicam.timecodes.value),
            const MenuDivider(),
            ...viewerSourceRows(ui, view, close),
          ],
          child: child,
        );

    // Following a clip its time is the program's: no strip, no bar.
    final strip = following != null
        ? null
        : SourceStrip(
            view: view,
            frames: frames,
            detached: round,
            onSeek: (frame) => ui.seekFootageView(view, frame),
          );
    final bar = following != null
        ? null
        : SourceBar(view: view, facts: _facts(info, frames), detached: round);
    if (!round) {
      return menu(Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          header,
          Expanded(child: picture),
          if (strip != null) strip,
          if (bar != null) bar,
        ],
      ));
    }
    final gap = SizedBox(height: t.tokens.tileGap);
    return menu(ColoredBox(
      color: t.room,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          header,
          gap,
          Expanded(
            child: Container(
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(t.tokens.cardRadius),
                boxShadow: t.tokens.cardShadow,
              ),
              clipBehavior: Clip.antiAlias,
              child: picture,
            ),
          ),
          if (strip != null) ...[gap, strip],
          if (bar != null) ...[gap, bar],
        ],
      ),
    ));
  }
}

/// Which angles the marks go on, as the overlay reads them when it paints.
class _Marks {
  final UuidValue? picture;
  final UuidValue? sound;

  /// Angles heard as well as [sound].
  final List<UuidValue> also;

  /// Whether the clip under the playhead is ordinary footage that one angle
  /// holds, [picture] being that angle.
  final bool plain;
  final bool following;

  /// Each angle's own time at the moment shown, when asked for.
  final Map<UuidValue, BridgeAngleTime> times;

  const _Marks({
    required this.picture,
    required this.sound,
    required this.also,
    required this.plain,
    required this.following,
    required this.times,
  });

  bool hears(UuidValue angle) => angle == sound || also.contains(angle);
}

/// The cells of the grid over [size], equal and row-major, as the engine
/// draws them.
List<Rect> _cellsOf(Size size, BridgeAngleGrid grid) {
  final columns = math.max(grid.columns, 1);
  final rows = math.max(grid.rows, 1);
  final w = size.width / columns, h = size.height / rows;
  return [
    for (var i = 0; i < grid.count; i++)
      Rect.fromLTWH((i % columns) * w, (i ~/ columns) * h, w, h),
  ];
}

/// Where the comp's picture lands inside [cell], its shape kept.
Rect _letterbox(Rect cell, int width, int height) {
  final scale = math.min(cell.width / width, cell.height / height);
  if (!scale.isFinite || scale <= 0) return cell;
  return Rect.fromCenter(
      center: cell.center, width: width * scale, height: height * scale);
}

/// What the picture cannot say: each angle's number and name on a band
/// under its picture, its timecode when asked, the outline on the picture
/// angle, the mark on the sound angle, the wash under the pointer and the
/// flash of a cut.
///
/// Repaints on its own when the clip under the playhead, the pointer or the
/// flash moves: the stage above is not rebuilt for any of them.
class _AngleGridPainter extends CustomPainter {
  final List<Rect> cells;
  final List<Rect> pictures;
  final List<BridgeAngle> angles;
  final _Marks Function() marks;
  final BridgeAngleSwitch mode;
  final ValueListenable<int?> hover;
  final ValueListenable<UuidValue?> flashed;
  final Animation<double> fade;
  final ValueListenable<BridgeMulticamAt?> under;
  final ValueListenable<bool> timecodes;
  final LumitTheme theme;

  _AngleGridPainter({
    required this.cells,
    required this.pictures,
    required this.angles,
    required this.marks,
    required this.mode,
    required this.hover,
    required this.flashed,
    required this.fade,
    required this.under,
    required this.timecodes,
    required this.theme,
  }) : super(
            repaint:
                Listenable.merge([fade, under, hover, flashed, timecodes]));

  @override
  void paint(Canvas canvas, Size size) {
    final t = theme;
    final m = marks();
    final count = math.min(cells.length, angles.length);
    final strongPicture = mode != BridgeAngleSwitch.sound;
    final strongSound = mode != BridgeAngleSwitch.picture;
    final flash = fade.isCompleted || fade.isDismissed ? 0.0 : 1 - fade.value;
    final number = t.mono.copyWith(fontSize: 10, color: t.textPrimary);
    final name = t.small.copyWith(color: t.textPrimary);
    final clock = t.mono.copyWith(fontSize: 10, color: t.textMuted);
    final showTimes = timecodes.value && m.following;
    for (var i = 0; i < count; i++) {
      final angle = angles[i];
      final cell = cells[i];
      final box = pictures[i];
      // A sound angle has no picture: its cell is dark, its glyph is drawn
      // over this as a widget.
      if (angle.sound) {
        canvas.drawRect(cell, Paint()..color = t.surface0);
      }
      // The band, the number in its cap, then the name in what is left, and
      // the angle's own time at the right when asked and there is room.
      final band = Rect.fromLTRB(
          box.left, math.max(box.top, box.bottom - _bandHeight), box.right,
          box.bottom);
      canvas.drawRect(band, Paint()..color = t.scrim);
      final digits = TextPainter(
        text: TextSpan(text: '${i + 1}', style: number),
        textDirection: TextDirection.ltr,
      )..layout();
      final capWidth = math.max(digits.width + 6, _bandHeight - 6);
      final cap = Rect.fromLTWH(band.left + 3, band.top + 3, capWidth,
          _bandHeight - 6);
      canvas.drawRRect(
        RRect.fromRectAndRadius(
            cap, Radius.circular(t.tokens.controlRadius)),
        Paint()..color = t.surface2,
      );
      canvas.drawRRect(
        RRect.fromRectAndRadius(
            cap, Radius.circular(t.tokens.controlRadius)),
        Paint()
          ..color = t.hairlineStrong
          ..style = PaintingStyle.stroke,
      );
      digits.paint(
          canvas,
          Offset(cap.center.dx - digits.width / 2,
              cap.center.dy - digits.height / 2));
      digits.dispose();
      var room = band.right - cap.right - 8;
      final time = showTimes ? m.times[angle.id] : null;
      if (time != null) {
        final reading = TextPainter(
          text: TextSpan(
              text: timecodeOfRate(time.frame, time.fpsNum, time.fpsDen,
                  dropFrame: time.dropFrame),
              style: clock),
          textDirection: TextDirection.ltr,
        )..layout();
        // The time goes before the name does: a narrow cell keeps the name.
        if (reading.width + 32 <= room) {
          reading.paint(
              canvas,
              Offset(band.right - 4 - reading.width,
                  band.center.dy - reading.height / 2));
          room -= reading.width + 6;
        }
        reading.dispose();
      }
      if (room > 8) {
        final label = TextPainter(
          text: TextSpan(text: angle.name, style: name),
          textDirection: TextDirection.ltr,
          maxLines: 1,
          ellipsis: '…',
        )..layout(maxWidth: room);
        label.paint(canvas,
            Offset(cap.right + 4, band.center.dy - label.height / 2));
        label.dispose();
      }
      // The marks. The picture angle's outline, two wide in the accent and
      // one in grey while the sound is what a click switches; a thin outline
      // on an angle heard that is not the picture.
      final isPicture = angle.id == m.picture;
      if (isPicture) {
        _outline(canvas, box, strongPicture ? t.accent : t.textSecondary,
            strongPicture ? 2 : 1);
      } else if (m.hears(angle.id) && strongSound) {
        _outline(canvas, box, t.textSecondary, 1);
      }
      if (flash > 0 && angle.id == flashed.value) {
        _outline(canvas, box, t.accent.withValues(alpha: flash), 2 + 2 * flash);
      }
      if (i == hover.value) {
        canvas.drawRect(cell, Paint()..color = hoverTint(t));
      }
    }
  }

  void _outline(Canvas canvas, Rect box, Color colour, double width) {
    canvas.drawRect(
      box.deflate(width / 2),
      Paint()
        ..color = colour
        ..style = PaintingStyle.stroke
        ..strokeWidth = width,
    );
  }

  @override
  bool shouldRepaint(_AngleGridPainter old) =>
      !listEquals(old.cells, cells) ||
      !listEquals(old.angles, angles) ||
      old.mode != mode ||
      old.theme != theme ||
      old.marks != marks;
}
