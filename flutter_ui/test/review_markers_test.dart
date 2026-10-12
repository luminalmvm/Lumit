// Comments on a review link become markers on the composition they came
// from, or on the layer they point at. A mistake there puts a note on the
// wrong frame, or leaves one out without a word, so this checks where they
// land and what they say.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/reviews.dart';

ReviewComment _comment(String id, int frame, String name, String text,
        {int? end, String? parent, bool resolved = false}) =>
    ReviewComment(
      id: id,
      frame: frame,
      endFrame: end,
      name: name,
      text: text,
      parent: parent,
      resolved: resolved,
    );

void main() {
  test('comments land on the composition frame they were made on', () {
    final markers = commentMarkers(
      [
        _comment('a', 10, 'Ana', 'Hold this\n a beat longer'),
        _comment('b', 10, 'Ben', 'Agreed'),
        _comment('c', 40, 'Ana', 'Trim', end: 49),
        _comment('d', 90, 'Ana', 'Past the end', end: 400),
        _comment('e', 20, 'Ana', 'Done already', resolved: true),
        _comment('f', 10, 'Mack', 'Will do', parent: 'a'),
        // A like is not a note, so it is nobody's marker.
        const ReviewComment(
            id: 'g', frame: 55, name: 'Jun', text: '', reaction: 'like'),
      ],
      // The video began at frame 100 of a composition 180 frames long.
      startFrame: 100,
      compFrames: 180,
      // One of them points at a layer.
      layers: {'c': 'title'},
    );

    expect([for (final m in markers) m.frame], [110, 140, 179],
        reason: 'offset by where the export began, and held inside the '
            'composition');
    // Two comments on one frame share its one marker, on one line.
    expect(markers[0].label, 'Ana: Hold this a beat longer · Ben: Agreed');
    expect(markers[0].ids, ['a', 'b']);
    expect(markers[0].duration, isNull);
    // A range is a region, its last frame counted.
    expect(markers[1].label, 'Ana: Trim');
    expect(markers[1].duration, 10);
    // The one that points at a layer is that layer's own, to move with it.
    expect([for (final m in markers) m.layer], [null, 'title', null]);
    expect(markers[2].duration, 1, reason: 'cut off at the last frame');
    // The same person is the same colour every time, and it is a label's.
    expect(markers[0].colour, markers[1].colour);
    expect(markers[0].colour, inInclusiveRange(1, 8));
    expect(markers[0].colour, commenterColour(' ana '));

    // What is already on the Timeline is not added twice.
    final comments = [
      _comment('a', 10, 'Ana', 'One'),
      _comment('b', 10, 'Ben', 'Two'),
      _comment('c', 30, 'Ana', 'Three'),
    ];
    List<int> frames(Set<String> added) => [
          for (final m in commentMarkers(comments,
              startFrame: 0, compFrames: 100, added: added))
            m.frame,
        ];
    expect(frames({'a', 'b', 'c'}), isEmpty);
    expect(frames({'a', 'b'}), [30]);
    // A new comment on a frame that has a marker remakes that marker whole.
    final again = commentMarkers(comments,
        startFrame: 0, compFrames: 100, added: {'a', 'c'});
    expect(again.single.label, 'Ana: One · Ben: Two');
    // Two on one frame share a marker only where they share a place.
    final apart = commentMarkers(comments,
        startFrame: 0, compFrames: 100, layers: {'b': 'title'});
    expect([
      for (final m in apart) (m.frame, m.layer, m.label)
    ], [
      (10, null, 'Ana: One'),
      (10, 'title', 'Ben: Two'),
      (30, null, 'Ana: Three'),
    ]);
  });
}
