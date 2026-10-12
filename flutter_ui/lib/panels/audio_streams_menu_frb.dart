// The Audio streams submenu: which of a file's audio streams a layer or a clip
// plays, and the two commands that split them onto rows of their own and
// merge them back. One file, because the layer Timeline, the Audio timeline
// and the Cut timeline all offer it and must not word it three ways.
//
// A recording can carry several sounds side by side, the game on one stream
// and the microphone on another. The engine decides what a split or a merge
// does; this is the list and the ticks.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/streams.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';

/// What one stream's row says: its number, counted from one, and what the
/// file calls it where it calls it anything. A stream with no name of its own
/// is told apart by its language where the file gives one.
String audioStreamLabel(BridgeAudioStream stream) {
  final number = stream.place + 1;
  final name = stream.title ?? stream.language;
  return name == null
      ? l10n.audioStreamNumbered(number)
      : l10n.audioStreamNamed(number, name);
}

/// The row that opens the list of streams, for a menu's own rows.
///
/// `streams` is what the layer or clip has to choose from; the row is only
/// worth showing when there is more than one. `onChoose` is handed the places
/// to play each time a tick changes, and the list stays open so several can
/// be ticked in one visit.
Widget audioStreamsSubmenu({
  required Key key,
  required VoidCallback closeParent,
  required BridgeAudioStreams streams,
  required void Function(List<int> places) onChoose,
}) =>
    SubmenuRow(
      key: key,
      closeParent: closeParent,
      submenu: (_) => FloatSurface(
        width: 210,
        child: _StreamTicks(streams: streams, onChoose: onChoose),
      ),
      child: Text(l10n.menuAudioStreams),
    );

/// The ticked list. It keeps its own copy of the ticks so a press shows at
/// once; the document follows through [onChoose].
class _StreamTicks extends StatefulWidget {
  final BridgeAudioStreams streams;
  final void Function(List<int> places) onChoose;

  const _StreamTicks({required this.streams, required this.onChoose});

  @override
  State<_StreamTicks> createState() => _StreamTicksState();
}

class _StreamTicksState extends State<_StreamTicks> {
  late final Set<int> _chosen = widget.streams.chosen.toSet();

  void _toggle(int place) {
    final next = {..._chosen};
    if (!next.remove(place)) next.add(place);
    // A layer plays at least one stream. Silence is the mute switch's job.
    if (next.isEmpty) return;
    setState(() {
      _chosen
        ..clear()
        ..addAll(next);
    });
    widget.onChoose(next.toList()..sort());
  }

  @override
  Widget build(BuildContext context) => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final stream in widget.streams.streams)
            MenuRow.option(
              key: ValueKey<String>('audio-stream-${stream.place}'),
              onPressed: () => _toggle(stream.place),
              child: Row(children: [
                menuTick(_chosen.contains(stream.place)),
                Expanded(
                  child: Text(audioStreamLabel(stream),
                      overflow: TextOverflow.ellipsis),
                ),
              ]),
            ),
        ],
      );
}
