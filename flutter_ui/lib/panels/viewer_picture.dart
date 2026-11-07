// The Viewer's picture: the engine's texture, shown where it lies on the card.
//
// The browser build has no external textures, so it imports
// viewer_picture_web.dart in this file's place, which shows the same picture
// as an image.

import 'package:flutter/widgets.dart';

/// The picture named [textureId].
Widget viewerPicture(int textureId,
        {FilterQuality filterQuality = FilterQuality.low}) =>
    Texture(textureId: textureId, filterQuality: filterQuality);

/// Take the newest picture the engine left for [view]. Only the browser build
/// has one to take.
Future<void> takeViewerPicture(int view) async {}
