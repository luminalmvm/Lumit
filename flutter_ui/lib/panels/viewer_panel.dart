// The Viewer (phase F2): decoded frames on the exactly-neutral pasteboard, a
// generated colour-bars slate for missing footage, and the transport row.
//
// In plain terms: this shows the actual picture the playhead is over. It reads
// the shared CPU frame source (preview_source.dart) — which decodes one footage
// layer's frame through the engine bridge — and blits it fit-to-panel on the
// neutral surround. The real composited comp (all layers, transforms, effects)
// still lives in the egui crate and is not available here yet, so this is a
// single-layer preview, labelled honestly. Play advances the playhead on a
// Ticker at the comp's frame rate and loops the composition, mirroring the egui
// transport (its playback loops the work area).

import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

import '../bridge/bridge.dart';
import '../icons/icons.dart';
import '../state/app_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'preview_source.dart';
import 'slate.dart';
import 'viewer_overlays.dart';
import 'viewer_toolbar.dart';

/// The next playhead frame during playback — looping the work area when one is
/// set, else the whole comp. Mirrors the egui transport (playback.rs
/// `comp_cached_tick`): the loop span is `[waStart, waEnd)` (out edge exclusive),
/// a playhead scrubbed outside the area snaps back to its start, and the wrap is
/// modular so a large [advance] still lands inside the span. Pure, so it
/// unit-tests without a ticker. [workArea] is `[inFrame, outFrame]` or null for
/// the whole comp.
int workAreaLoopFrame({
  required int current,
  required int advance,
  required int frameCount,
  List<int>? workArea,
}) {
  if (frameCount <= 0) return 0;
  final waStart = (workArea != null ? workArea[0] : 0).clamp(0, frameCount - 1);
  final waEnd =
      (workArea != null ? workArea[1] : frameCount).clamp(waStart + 1, frameCount);
  final span = waEnd - waStart;
  if (current < waStart || current >= waEnd) {
    return waStart; // scrubbed outside the area — clamp back in
  }
  final next = current + advance;
  if (next >= waEnd) {
    return waStart + ((next - waStart) % span); // loop the work area
  }
  return next;
}

class ViewerPanel extends StatefulWidget {
  final AppStateStub app;
  const ViewerPanel({super.key, required this.app});

  @override
  State<ViewerPanel> createState() => _ViewerPanelState();
}

class _ViewerPanelState extends State<ViewerPanel>
    with SingleTickerProviderStateMixin {
  late final Ticker _ticker;
  Duration _lastTick = Duration.zero;
  double _frameAccum = 0;

  AppStateStub get app => widget.app;
  PreviewSource get source => app.previewSource;

  @override
  void initState() {
    super.initState();
    _ticker = createTicker(_onTick);
    app.addListener(_onAppChanged);
    _syncTicker();
  }

  @override
  void dispose() {
    app.removeListener(_onAppChanged);
    _ticker.dispose();
    super.dispose();
  }

  void _onAppChanged() => _syncTicker();

  /// Run the ticker exactly while the transport is playing.
  void _syncTicker() {
    if (app.playing && !_ticker.isActive) {
      _lastTick = Duration.zero;
      _frameAccum = 0;
      _ticker.start();
    } else if (!app.playing && _ticker.isActive) {
      _ticker.stop();
    }
  }

  /// Advance the playhead at the comp's rational fps, accumulating elapsed time
  /// into whole frames and looping at the composition's end (the egui transport
  /// loops the work area; without a work area in the snapshot, it loops the
  /// whole comp).
  void _onTick(Duration elapsed) {
    final comp = app.frontComp;
    if (comp == null) return;
    final fps = comp.fps.fps;
    final frameCount = comp.frameCount;
    if (fps <= 0 || frameCount <= 0) return;

    final dt = _lastTick == Duration.zero
        ? Duration.zero
        : elapsed - _lastTick;
    _lastTick = elapsed;
    _frameAccum += dt.inMicroseconds / 1e6 * fps;
    if (_frameAccum < 1) return;

    final advance = _frameAccum.floor();
    _frameAccum -= advance;
    app.advancePlayback(workAreaLoopFrame(
      current: app.previewFrame,
      advance: advance,
      frameCount: frameCount,
      workArea: comp.workArea,
    ));
    // Under Auto resolution, refresh the realtime tier readout on the playback
    // cadence (a quiet no-op off Auto, and it only notifies on a tier change).
    app.pollPlaybackTier();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      color: t.viewerSurround,
      child: Column(
        children: [
          // The tool row above the stage (Select / Hand / Shape / Pen).
          ViewerToolbar(app: app),
          Expanded(
            child: ListenableBuilder(
              listenable: Listenable.merge([app, source]),
              builder: (context, _) => Stack(
                fit: StackFit.expand,
                children: [
                  _buildStage(context, t),
                  _buildInteraction(context),
                ],
              ),
            ),
          ),
          ListenableBuilder(
            listenable: app,
            builder: (context, _) => _buildTransport(context, t),
          ),
        ],
      ),
    );
  }

  /// The interaction overlay (shape-drag, transform gizmo, eyedropper), sized to
  /// the stage and given the fitted picture's rectangle. It draws nothing when
  /// there is no picture or no front comp to map against.
  Widget _buildInteraction(BuildContext context) {
    final comp = app.frontComp;
    if (comp == null || comp.width <= 0 || comp.height <= 0) {
      return const SizedBox.expand();
    }
    // The aspect the stage fits to: the shared texture's, else the shown image's.
    double? aspect;
    if (source.sharedActive) {
      aspect = source.sharedAspect;
    } else {
      final img = source.image;
      if (img != null && img.height > 0) aspect = img.width / img.height;
    }
    aspect ??= comp.width / comp.height;
    final a = aspect;
    return LayoutBuilder(
      builder: (context, constraints) {
        final rect = _fittedRect(
            Size(constraints.maxWidth, constraints.maxHeight), a);
        return ViewerInteractionLayer(
          app: app,
          source: source,
          imageRect: rect,
          compWidth: comp.width,
          compHeight: comp.height,
        );
      },
    );
  }

  Widget _buildStage(BuildContext context, LumitTheme t) {
    // The zero-copy shared-texture path (K-177): the engine drew the whole comp
    // into a GPU texture Flutter samples directly — show it with a `Texture`
    // widget, fit to the panel. Like the comp path it is self-contained (any
    // missing layer is slated inside the frame), so no separate slate applies.
    final texId = source.textureId;
    if (source.sharedActive && texId != null) {
      return Center(
        child: AspectRatio(
          aspectRatio: source.sharedAspect ?? (16 / 9),
          child: Texture(textureId: texId),
        ),
      );
    }

    final target = source.target;

    // On the composited-comp path there is no single-layer [target]: a missing
    // layer is already slated as colour bars INSIDE the engine-rendered frame
    // (verified — the compositor draws `slate::colour_bars` for missing footage
    // and composites it like any source), so the Viewer just blits the image and
    // shows no separate slate. The slate branches below apply only to the
    // single-layer fallback, where the whole preview IS one footage layer.
    if (!source.compActive) {
      // Missing footage → the generated colour-bars slate with the item's path.
      if (target != null && target.item.status == BridgeMediaStatus.missing) {
        return _MissingSlate(path: target.item.name, textStyle: t.small);
      }
      // Present but unreadable → a dark slate with the egui "unreadable" wording.
      if (target != null && target.item.status == BridgeMediaStatus.failed) {
        return _FailedSlate(name: target.item.name, theme: t);
      }
    }

    final image = source.image;
    if (image != null) {
      return _FittedImage(image: image);
    }

    // Nothing to show yet (frame still decoding, or nothing under the playhead):
    // the quiet film-icon placeholder. The wording drops the "single-layer"
    // caveat once the composited-comp path is live.
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          lumitIcon(LumitIcon.film, size: 32, color: t.textDisabled),
          const SizedBox(height: 8),
          Text(
            source.compActive
                ? 'Rendering the composited comp…'
                : 'Single-layer preview — the composited comp arrives when the '
                    'compositor leaves the egui crate',
            style: t.small,
            textAlign: TextAlign.center,
          ),
        ],
      ),
    );
  }

  Widget _buildTransport(BuildContext context, LumitTheme t) {
    final comp = app.frontComp;
    final fps = comp?.fps.fps ?? 0;
    return Container(
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
          // The frame + timecode readout is the one part of the transport that
          // moves every playhead tick, so it alone watches the fine-grained
          // playhead notifier — the play button and fps stay on the app
          // notifier above (perf pass).
          ValueListenableBuilder<int>(
            valueListenable: app.playheadFrame,
            builder: (context, frame, _) => Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text('frame $frame', style: t.small),
                const SizedBox(width: 8),
                Text(_timecode(frame, fps), style: t.small),
              ],
            ),
          ),
          const Spacer(),
          // The resolution (preview scale) picker — Auto / Full / Half / Third /
          // Quarter (egui's option set, overlays.rs). Under Auto the preview
          // renders at the realtime controller's live tier; a manual pick
          // overrides. The tooltip is honest that this is a preview downsample,
          // not a lower-quality export.
          LumitTooltip(
            message: 'Preview resolution (downsamples the preview render; the '
                'export is always full quality). Auto adapts to playback load',
            child: BareDropdown<PreviewScale?>(
              key: const ValueKey('resolution-picker'),
              value: app.previewAutoRes ? null : app.previewScale,
              options: const [null, ...PreviewScale.values],
              label: (s) => s == null ? 'Auto' : s.label,
              onChanged: (s) =>
                  s == null ? app.setPreviewAuto() : app.setPreviewScale(s),
            ),
          ),
          // Under Auto, the live tier the realtime controller settled on (egui's
          // trailing `%` readout) — polled on the playback cadence, so it reads
          // "Auto: Half" as the engine drops under strain, and just "Auto" at
          // rest.
          if (app.previewAutoRes)
            Padding(
              padding: const EdgeInsets.only(left: 6),
              child: Text(
                app.playing ? 'Auto: ${app.autoTier.label}' : 'Auto',
                style: t.small.copyWith(color: t.textMuted),
              ),
            ),
        ],
      ),
    );
  }
}

/// The centred, aspect-preserved rectangle a picture of [aspect] fits into a
/// [stage] — the same fit the `_FittedImage`/`Texture` uses, so the interaction
/// overlay lands exactly over the shown pixels.
Rect _fittedRect(Size stage, double aspect) {
  if (stage.width <= 0 || stage.height <= 0 || aspect <= 0) {
    return Rect.zero;
  }
  var w = stage.width;
  var h = w / aspect;
  if (h > stage.height) {
    h = stage.height;
    w = h * aspect;
  }
  final left = (stage.width - w) / 2;
  final top = (stage.height - h) / 2;
  return Rect.fromLTWH(left, top, w, h);
}

/// Frame → SMPTE non-drop timecode (HH:MM:SS:FF) at [fps]. A zero/degenerate
/// rate yields a bare frame count so the readout never divides by zero.
String _timecode(int frame, double fps) {
  final rate = fps.round();
  if (rate <= 0) return '00:00:00:00';
  final ff = frame % rate;
  final totalSeconds = frame ~/ rate;
  final ss = totalSeconds % 60;
  final mm = (totalSeconds ~/ 60) % 60;
  final hh = totalSeconds ~/ 3600;
  String two(int n) => n.toString().padLeft(2, '0');
  return '${two(hh)}:${two(mm)}:${two(ss)}:${two(ff)}';
}

/// The decoded frame, blitted fit-to-panel with aspect preserved on the neutral
/// surround (medium filtering). A findable [RawImage] so widget tests can assert
/// the Viewer painted a picture.
class _FittedImage extends StatelessWidget {
  final ui.Image image;
  const _FittedImage({required this.image});

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final iw = image.width.toDouble();
        final ih = image.height.toDouble();
        if (iw <= 0 || ih <= 0) return const SizedBox.expand();
        final scale =
            math.min(constraints.maxWidth / iw, constraints.maxHeight / ih);
        return Center(
          child: RawImage(
            image: image,
            width: iw * scale,
            height: ih * scale,
            fit: BoxFit.fill,
            filterQuality: FilterQuality.medium,
          ),
        );
      },
    );
  }
}

/// The missing-footage slate: colour bars filling the stage, with the item's
/// (relative) path on a translucent strip along the bottom.
class _MissingSlate extends StatelessWidget {
  final String path;
  final TextStyle textStyle;
  const _MissingSlate({required this.path, required this.textStyle});

  @override
  Widget build(BuildContext context) {
    return Stack(
      fit: StackFit.expand,
      children: [
        const CustomPaint(painter: SlatePainter()),
        Align(
          alignment: Alignment.bottomCenter,
          child: Container(
            width: double.infinity,
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
            color: documentColourStrip,
            child: Text(
              'Missing footage — $path',
              style: textStyle.copyWith(color: documentColourStripText),
              textAlign: TextAlign.center,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ),
      ],
    );
  }
}

/// The unreadable-footage slate: a dark field with the egui "unreadable" wording
/// (docs/07 §3.3 — a present-but-unreadable file shows no picture).
class _FailedSlate extends StatelessWidget {
  final String name;
  final LumitTheme theme;
  const _FailedSlate({required this.name, required this.theme});

  @override
  Widget build(BuildContext context) {
    return Container(
      color: documentColourFailBg,
      alignment: Alignment.center,
      padding: const EdgeInsets.symmetric(horizontal: 16),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          lumitIcon(LumitIcon.unlink, size: 28, color: theme.warning),
          const SizedBox(height: 8),
          Text('unreadable — $name', style: theme.small, textAlign: TextAlign.center),
        ],
      ),
    );
  }
}
