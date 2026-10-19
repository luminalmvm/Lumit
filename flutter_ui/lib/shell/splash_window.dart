// The boot splash: a picture with Lumit's words on it, shown in a window of
// its own while everything Lumit loads at start-up is loaded.
//
// The window is the picture's shape. Its see-through pixels are see-through on
// the desktop too, so a picture can reach outside the card. Dart paints the
// whole thing and hands the runner finished pixels (lumit/window "splash"), so
// the three runners only have to put a bitmap on screen, and the picture and
// the words are drawn once, here.
//
// Lumit's own picture is a card lit by the brand's green and blue, painted
// here, or by the theme's accent when Appearance asks for it. A `splash.png`
// beside the settings file is used in its place, at the template's size (640
// by 500, or a multiple). The words always go in the same places, so a
// home-made picture leaves those parts quiet.

import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../state/workspace.dart';
import '../theme/brand.dart';
import '../theme/theme.dart';
import 'wordmark.dart';

/// The template, in logical pixels. The picture fills the canvas, and the card
/// it is drawn round sits at 110, 110, 480 by 360.
const ui.Size splashCanvas = ui.Size(640, 500);
const ui.Rect _card = ui.Rect.fromLTWH(110, 110, 480, 360);
const ui.Offset _wordmarkAt = ui.Offset(144, 356);
const double _wordmarkHeight = 46;
const double _versionRight = 564, _versionBaseline = 144;
const double _lineLeft = 144, _lineRight = 556, _lineBaseline = 434;
const double _barTop = 444, _barHeight = 3;
const double _textSize = 12;

/// The splash window, while it is up.
class BootSplash {
  BootSplash._(
      this._scale, this._art, this._wordmark, this._version, ui.Color? accent)
      : _lightA = accent ?? brandKeyJade,
        _lightB = accent == null
            ? brandKeyBlueLight
            : ui.Color.lerp(accent, brandWordmarkPaper, 0.45)!;

  static const _channel = MethodChannel('lumit/window');

  /// The splash on screen, if there is one.
  static BootSplash? _current;

  final double _scale;
  final ui.Image? _art;
  final PictureInfo? _wordmark;
  final String _version;

  /// The two colours the card is lit with, and the bar fills with.
  final ui.Color _lightA, _lightB;

  String _step = '';
  String _detail = '';
  double _fraction = 0;

  bool _drawing = false, _again = false, _closed = false;
  DateTime _last = DateTime.fromMillisecondsSinceEpoch(0);
  Timer? _waiting;

  /// Put the splash up, or null where the runner has no splash window (a
  /// start into the tray, or a platform that can't) and Lumit starts without
  /// one.
  ///
  /// [accent] lights the card in place of Lumit's own green and blue.
  static Future<BootSplash?> open(
      {required String version, required String step, ui.Color? accent}) async {
    try {
      final scale = await _channel.invokeMethod<double>('splashScale');
      if (scale == null || scale <= 0) return null;
      final splash = BootSplash._(
          scale, await _picture(), await _lettering(), version, accent)
        .._step = step;
      if (!await splash._draw()) return null;
      return _current = splash;
    } catch (_) {
      // No splash is not a reason not to start.
      return null;
    }
  }

  /// Say what is being loaded now. [fraction] is how far the whole start-up
  /// has got, 0 to 1, and only ever moves forwards.
  void say(String step, {String detail = '', double? fraction}) {
    _step = step;
    _detail = detail;
    if (fraction != null && fraction > _fraction) {
      _fraction = fraction.clamp(0, 1);
    }
    _schedule();
  }

  /// Take the splash down, if there is one, and let the main window show.
  /// Said whether or not a splash went up: the macOS window waits for it.
  static Future<void> handOver() async {
    final splash = _current;
    _current = null;
    splash?._closed = true;
    splash?._waiting?.cancel();
    try {
      await _channel.invokeMethod<void>('splashClose');
    } catch (_) {}
  }

  /// At most about 30 pictures a second, however fast the lines come.
  void _schedule() {
    if (_closed || _waiting != null) return;
    if (_drawing) {
      _again = true;
      return;
    }
    final wait =
        const Duration(milliseconds: 33) - DateTime.now().difference(_last);
    _waiting = Timer(wait.isNegative ? Duration.zero : wait, () {
      _waiting = null;
      unawaited(_draw());
    });
  }

  /// Paint the splash and hand it to the runner. False if it didn't show.
  Future<bool> _draw() async {
    if (_closed) return false;
    _drawing = true;
    _last = DateTime.now();
    try {
      final width = (splashCanvas.width * _scale).round();
      final height = (splashCanvas.height * _scale).round();
      final picture = _paint();
      final image = await picture.toImage(width, height);
      picture.dispose();
      // Premultiplied RGBA, which is what every runner's window wants.
      final bytes = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
      image.dispose();
      if (bytes == null || _closed) return false;
      return await _channel.invokeMethod<bool>('splash', {
            'width': width,
            'height': height,
            'bytes': bytes.buffer
                .asUint8List(bytes.offsetInBytes, bytes.lengthInBytes),
          }) ??
          false;
    } finally {
      _drawing = false;
      if (_again) {
        _again = false;
        _schedule();
      }
    }
  }

  ui.Picture _paint() {
    final recorder = ui.PictureRecorder();
    final canvas = ui.Canvas(recorder)..scale(_scale);
    final art = _art;
    if (art != null) {
      canvas.drawImageRect(
          art,
          ui.Rect.fromLTWH(0, 0, art.width.toDouble(), art.height.toDouble()),
          ui.Offset.zero & splashCanvas,
          ui.Paint()..filterQuality = ui.FilterQuality.medium);
    } else {
      _paintCard(canvas);
    }

    final wordmark = _wordmark;
    if (wordmark != null) {
      canvas.save();
      canvas.translate(_wordmarkAt.dx, _wordmarkAt.dy);
      canvas.scale(_wordmarkHeight / wordmark.size.height);
      canvas.drawPicture(wordmark.picture);
      canvas.restore();
    }

    final quiet = brandWordmarkPaper.withValues(alpha: 0.62);
    final version =
        _text(_version, quiet, ui.FontWeight.w400, 200, ui.TextAlign.right);
    canvas.drawParagraph(
        version,
        ui.Offset(_versionRight - 200,
            _versionBaseline - version.alphabeticBaseline));

    final step = _text(_step, brandWordmarkPaper, ui.FontWeight.w500,
        _lineRight - _lineLeft, ui.TextAlign.left);
    final stepWidth = step.longestLine;
    canvas.drawParagraph(
        step, ui.Offset(_lineLeft, _lineBaseline - step.alphabeticBaseline));
    final room = _lineRight - _lineLeft - stepWidth - 24;
    if (_detail.isNotEmpty && room > 0) {
      final detail = _text(_detail, brandWordmarkPaper.withValues(alpha: 0.6),
          ui.FontWeight.w400, room, ui.TextAlign.right);
      canvas.drawParagraph(
          detail,
          ui.Offset(
              _lineRight - room, _lineBaseline - detail.alphabeticBaseline));
    }

    const track =
        ui.Rect.fromLTRB(_lineLeft, _barTop, _lineRight, _barTop + _barHeight);
    const round = ui.Radius.circular(_barHeight / 2);
    canvas.drawRRect(ui.RRect.fromRectAndRadius(track, round),
        ui.Paint()..color = brandWordmarkPaper.withValues(alpha: 0.1));
    if (_fraction > 0) {
      final fill = ui.Rect.fromLTWH(
          track.left, track.top, track.width * _fraction, track.height);
      canvas.drawRRect(
          ui.RRect.fromRectAndRadius(fill, round),
          ui.Paint()
            ..shader = ui.Gradient.linear(
                track.centerLeft, track.centerRight, [_lightA, _lightB]));
    }
    return recorder.endRecording();
  }

  /// Lumit's own picture: a dark card with a soft shadow, lit from its top
  /// left by one colour and its bottom right by the other.
  void _paintCard(ui.Canvas canvas) {
    final dark = LumitTheme.dark();
    final shape =
        ui.RRect.fromRectAndRadius(_card, const ui.Radius.circular(14));
    canvas.drawRRect(
        shape.shift(const ui.Offset(0, 10)),
        ui.Paint()
          ..color = brandWordmarkInk.withValues(alpha: 0.55)
          ..maskFilter = const ui.MaskFilter.blur(ui.BlurStyle.normal, 18));
    canvas.drawRRect(
        shape,
        ui.Paint()
          ..shader = ui.Gradient.linear(_card.topLeft, _card.bottomRight,
              [dark.surface2, dark.surface0]));
    canvas.save();
    canvas.clipRRect(shape);
    for (final (centre, radius, colour, alpha) in [
      (const ui.Offset(150, 130), 380.0, _lightA, 0.30),
      (const ui.Offset(600, 480), 420.0, _lightB, 0.26),
    ]) {
      canvas.drawCircle(
          centre,
          radius,
          ui.Paint()
            ..shader = ui.Gradient.radial(centre, radius, [
              colour.withValues(alpha: alpha),
              colour.withValues(alpha: 0),
            ]));
    }
    canvas.restore();
    canvas.drawRRect(
        shape.deflate(0.5),
        ui.Paint()
          ..style = ui.PaintingStyle.stroke
          ..strokeWidth = 1
          ..color = brandWordmarkPaper.withValues(alpha: 0.07));
  }

  static ui.Paragraph _text(String text, ui.Color colour, ui.FontWeight weight,
      double width, ui.TextAlign align) {
    final builder = ui.ParagraphBuilder(ui.ParagraphStyle(
      textAlign: align,
      maxLines: 1,
      ellipsis: '…',
      fontFamily: LumitTheme.fontFamily,
      fontSize: _textSize,
      fontWeight: weight,
    ))
      ..pushStyle(ui.TextStyle(
          color: colour,
          fontFamily: LumitTheme.fontFamily,
          fontFamilyFallback: LumitTheme.fontFamilyFallback,
          fontSize: _textSize,
          fontWeight: weight))
      ..addText(text);
    return builder.build()..layout(ui.ParagraphConstraints(width: width));
  }

  /// The person's own picture if there is one that reads. Null draws
  /// Lumit's card.
  static Future<ui.Image?> _picture() async {
    final own = File('${Workspace.storeFile().parent.path}'
        '${Platform.pathSeparator}splash.png');
    try {
      if (own.existsSync()) return await _decode(await own.readAsBytes());
    } catch (_) {
      // A file that is not a picture falls back to Lumit's own.
    }
    return null;
  }

  static Future<ui.Image> _decode(Uint8List bytes) async {
    final codec = await ui.instantiateImageCodec(bytes);
    return (await codec.getNextFrame()).image;
  }

  static Future<PictureInfo?> _lettering() async {
    try {
      return await vg.loadPicture(
          const SvgAssetLoader(lumitWordmarkAsset,
              theme: SvgTheme(currentColor: brandWordmarkPaper)),
          null);
    } catch (_) {
      return null;
    }
  }
}
