import 'package:flutter/foundation.dart';
import 'package:uuid/uuid.dart' show UuidValue;

/// The captions picked out on a caption layer. The Captions panel and the
/// Timeline's captions row share this one choice, so picking in either shows
/// in both.
///
/// Captions are named by their place in the layer's own list, as the bridge
/// names them.
class CaptionSelection extends ChangeNotifier {
  /// The caption layer the choice is on.
  UuidValue? layer;

  /// The chosen captions.
  Set<int> cues = const {};

  /// The caption to open for typing, set with [typeRequest].
  int? typing;

  /// Bumped to ask the Captions panel to come to the front and open [typing]
  /// for typing, from the Timeline or the Viewer.
  final ValueNotifier<int> typeRequest = ValueNotifier(0);

  /// Choose [cues] on [layer].
  void choose(UuidValue? layer, Set<int> cues) {
    if (this.layer == layer && setEquals(this.cues, cues)) return;
    this.layer = layer;
    this.cues = Set.unmodifiable(cues);
    notifyListeners();
  }

  /// Choose caption [cue] on [layer] and ask for it to be typed into.
  void requestTyping(UuidValue layer, int cue) {
    choose(layer, {cue});
    typing = cue;
    typeRequest.value++;
  }

  @override
  void dispose() {
    typeRequest.dispose();
    super.dispose();
  }
}
