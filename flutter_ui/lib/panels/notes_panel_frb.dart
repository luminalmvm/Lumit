// The Notes panel: one page of text kept in the project file.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../src/rust/api/project.dart';
import '../state/app_state.dart';
import '../widgets/controls.dart';

class NotesPanelFrb extends StatefulWidget {
  const NotesPanelFrb({super.key});

  @override
  State<NotesPanelFrb> createState() => _NotesPanelFrbState();
}

class _NotesPanelFrbState extends State<NotesPanelFrb> {
  late final LumitState _app = Provider.of<LumitState>(context, listen: false);
  final _text = TextEditingController();
  final _focus = FocusNode();

  /// The project the text belongs to, and what it last held there.
  ProjectReference? _project;
  String _sent = '';

  /// Typing goes to the project after a pause, so a sentence is one undo
  /// step rather than one per letter.
  Timer? _pause;

  @override
  void initState() {
    super.initState();
    _read();
    _app.addListener(_read);
    _app.heldTyping.add(_send);
    _text.addListener(_typed);
    _focus.addListener(() {
      if (!_focus.hasFocus) _send();
    });
  }

  @override
  void dispose() {
    // Nothing is told: the tree is being taken down.
    _flush();
    _app.heldTyping.remove(_send);
    _app.removeListener(_read);
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  /// Pick up another project, or an undo or someone else's edit here.
  void _read() {
    final project = _app.project;
    if (project != _project) {
      _send();
      _project = project;
    } else if (_pause != null) {
      // Mid-sentence: what is being typed wins.
      return;
    }
    String notes;
    try {
      notes = project?.notes() ?? '';
    } catch (_) {
      notes = '';
    }
    _sent = notes;
    if (_text.text != notes) _text.text = notes;
  }

  void _typed() {
    if (_text.text == _sent) return;
    _pause?.cancel();
    _pause = Timer(const Duration(milliseconds: 500), _send);
  }

  void _send() {
    if (_flush()) _app.notifyDocumentChanged();
  }

  /// Sends the text, and says whether the project took it.
  bool _flush() {
    _pause?.cancel();
    _pause = null;
    final text = _text.text;
    if (text == _sent) return false;
    _sent = text;
    try {
      _project?.setNotes(notes: text);
      return _project != null;
    } catch (_) {
      // A project closed under the panel has nowhere to keep it.
      return false;
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      color: t.surface0,
      padding: const EdgeInsets.all(8),
      child: HouseTextField(
        key: const ValueKey('notes-text'),
        controller: _text,
        focusNode: _focus,
        multiline: true,
        topAlign: true,
        frameless: true,
        width: double.infinity,
      ),
    );
  }
}
