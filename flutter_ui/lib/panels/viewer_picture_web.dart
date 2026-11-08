// What viewer_picture.dart shows, for the browser build.
//
// The engine draws each picture onto a canvas and leaves it on the page as an
// ImageBitmap, at `lumitViewer[view]`. Taking it hands it to Flutter as an
// image without copying it off the card.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:ui' as ui;
import 'dart:ui_web' as ui_web;

import 'package:flutter/widgets.dart';

final Map<int, ValueNotifier<ui.Image?>> _pictures = {};

ValueNotifier<ui.Image?> _pictureOf(int view) =>
    _pictures.putIfAbsent(view, () => ValueNotifier<ui.Image?>(null));

/// The picture for view [textureId]. In the browser build a view's texture is
/// named by the view itself.
Widget viewerPicture(int textureId,
        {FilterQuality filterQuality = FilterQuality.low}) =>
    ValueListenableBuilder<ui.Image?>(
      valueListenable: _pictureOf(textureId),
      builder: (context, image, _) => RawImage(
        image: image,
        fit: BoxFit.fill,
        filterQuality: filterQuality,
      ),
    );

/// Take the newest picture the engine left for [view].
Future<void> takeViewerPicture(int view) async {
  final shelf = globalContext['lumitViewer'];
  if (shelf == null || !shelf.isA<JSObject>()) return;
  final bitmap = (shelf as JSObject)['$view'];
  if (bitmap == null) return;
  shelf['$view'] = null;
  final image = await ui_web.createImageFromImageBitmap(bitmap);
  final slot = _pictureOf(view);
  final old = slot.value;
  slot.value = image;
  old?.dispose();
}
