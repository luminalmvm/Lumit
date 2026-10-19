// A script's window: After Effects' ScriptUI dialogs, drawn with Lumit's own
// controls.
//
// The engine sends the window as data and waits. A press, or a change the
// script listens for, goes back with every control's value, and the window
// comes back as the script's handlers left it, until the script closes it.

import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/script.dart';

import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';

/// The window of one script run.
class ScriptDialogs {
  /// Where the window opens. Null, or gone, answers every window as closed.
  final BuildContext? context;

  /// Hands the engine what was done, as JSON, or null for closed.
  final void Function(String? event) answer;

  ScriptDialogs(this.context, {this.answer = _answer});

  static void _answer(String? event) => answerScriptDialog(event: event);

  final ValueNotifier<Map<String, dynamic>?> _tree = ValueNotifier(null);
  void Function(Object?)? _close;

  /// Whether the script is busy with the last thing done, so the next waits.
  bool _waiting = false;

  /// Show the window as [tree] describes it, or show it again as it now is.
  void show(String tree) {
    final Map<String, dynamic> parsed;
    try {
      parsed = jsonDecode(tree) as Map<String, dynamic>;
    } catch (_) {
      answer(null);
      return;
    }
    _tree.value = parsed;
    _waiting = false;
    if (_close != null) return;
    final context = this.context;
    if (context == null || !context.mounted) {
      answer(null);
      return;
    }
    var taken = false;
    showLumitModal<Object?>(
      context: context,
      id: 'script-dialog',
      builder: (close) {
        _close = (v) {
          taken = true;
          close(v);
        };
        return ValueListenableBuilder<Map<String, dynamic>?>(
          valueListenable: _tree,
          builder: (context, tree, _) => tree == null
              ? const SizedBox.shrink()
              : ScriptWindow(tree: tree, onEvent: _send, onDismiss: dismiss),
        );
      },
    ).then((_) {
      _close = null;
      // Escape or a click beside it: Cancel, as After Effects does.
      if (!taken) _dismissed(_tree.value);
    });
  }

  /// Take the window down, as the script has.
  void close() {
    _close?.call(null);
    _close = null;
    _tree.value = null;
  }

  /// The close mark: the same as Escape.
  void dismiss() {
    final tree = _tree.value;
    close();
    _dismissed(tree);
  }

  /// Gone without the script saying so: its Cancel button, if it has one.
  void _dismissed(Map<String, dynamic>? tree) {
    final cancel = _find(tree, (c) => c['role'] == 'cancel');
    if (cancel != null) {
      _send(cancel['id'] as String, _valuesOf(tree));
    } else if (!_waiting) {
      _waiting = true;
      answer(null);
    }
  }

  void _send(String id, Map<String, Object?> values) {
    if (_waiting) return;
    _waiting = true;
    answer(jsonEncode({'id': id, 'values': values}));
  }
}

Map<String, dynamic>? _find(
    Map<String, dynamic>? c, bool Function(Map<String, dynamic>) test) {
  if (c == null) return null;
  if (test(c)) return c;
  for (final k in (c['children'] as List? ?? const [])) {
    final found = _find(k as Map<String, dynamic>, test);
    if (found != null) return found;
  }
  return null;
}

Map<String, Object?> _valuesOf(Map<String, dynamic>? tree) {
  final out = <String, Object?>{};
  void walk(Map<String, dynamic> c) {
    switch (c['type']) {
      case 'edittext':
        out[c['id'] as String] = c['text'];
      case 'checkbox' || 'radiobutton' || 'slider':
        out[c['id'] as String] = c['value'];
      case 'dropdownlist':
        out[c['id'] as String] = c['selection'];
    }
    for (final k in (c['children'] as List? ?? const [])) {
      walk(k as Map<String, dynamic>);
    }
  }

  if (tree != null) walk(tree);
  return out;
}

/// The window [tree] describes, with what the person has changed kept here
/// until the script hears of it.
class ScriptWindow extends StatefulWidget {
  final Map<String, dynamic> tree;

  /// A press or change: the control's id, and every control's value.
  final void Function(String id, Map<String, Object?> values) onEvent;
  final VoidCallback onDismiss;

  const ScriptWindow(
      {super.key,
      required this.tree,
      required this.onEvent,
      required this.onDismiss});

  @override
  State<ScriptWindow> createState() => _ScriptWindowState();
}

class _ScriptWindowState extends State<ScriptWindow> {
  late Map<String, Object?> _values = _values0();
  final Map<String, TextEditingController> _texts = {};

  Map<String, Object?> _values0() => _valuesOf(widget.tree);

  @override
  void didUpdateWidget(ScriptWindow old) {
    super.didUpdateWidget(old);
    if (identical(old.tree, widget.tree)) return;
    _values = _values0();
    // The script may have changed what a field says.
    for (final e in _texts.entries) {
      final said = _values[e.key];
      if (said is String && e.value.text != said) e.value.text = said;
    }
  }

  @override
  void dispose() {
    for (final c in _texts.values) {
      c.dispose();
    }
    super.dispose();
  }

  TextEditingController _text(String id) => _texts.putIfAbsent(
      id, () => TextEditingController(text: (_values[id] as String?) ?? ''));

  Map<String, Object?> _all() => {
        ..._values,
        for (final e in _texts.entries) e.key: e.value.text,
      };

  void _set(String id, Object? v, {bool tell = false}) {
    setState(() => _values[id] = v);
    if (tell) widget.onEvent(id, _all());
  }

  void _press(String id) => widget.onEvent(id, _all());

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final tree = widget.tree;
    final ok = _find(tree, (c) => c['role'] == 'ok' && c['enabled'] != false);
    return Focus(
      autofocus: true,
      onKeyEvent: (_, event) {
        if (event is! KeyDownEvent || ok == null) return KeyEventResult.ignored;
        if (event.logicalKey != LogicalKeyboardKey.enter &&
            event.logicalKey != LogicalKeyboardKey.numpadEnter) {
          return KeyEventResult.ignored;
        }
        _press(ok['id'] as String);
        return KeyEventResult.handled;
      },
      child: ConstrainedBox(
        constraints: const BoxConstraints(minWidth: 240),
        child: IntrinsicWidth(
          child: DialogFrame(
            key: const ValueKey('script-dialog'),
            children: [
              dialogTitleBar(t,
                  title: tree['text'] as String? ?? '',
                  onClose: widget.onDismiss,
                  keyPrefix: 'script-dialog'),
              _holder(t, tree),
            ],
          ),
        ),
      ),
    );
  }

  /// A window, group or panel: its controls in a row or a column.
  Widget _holder(LumitTheme t, Map<String, dynamic> c) {
    final row = c['orientation'] == 'row';
    final along = (c['alignChildren'] as List?)?.cast<String>() ??
        const ['center', 'center'];
    final spacing = (c['spacing'] as num?)?.toDouble() ?? 10;
    final kids = [
      for (final k in (c['children'] as List? ?? const []))
        if ((k as Map<String, dynamic>)['visible'] != false) k,
    ];
    final placed = <Widget>[];
    for (final k in kids) {
      if (placed.isNotEmpty) {
        placed.add(SizedBox(width: row ? spacing : 0, height: row ? 0 : spacing));
      }
      final own = (k['alignment'] as List?)?.cast<String>();
      final side = (own ?? along)[row ? 1 : 0];
      final control = _control(t, k);
      placed.add(row
          ? control
          : switch (side) {
              'fill' => control,
              'left' => Align(alignment: Alignment.centerLeft, child: control),
              'right' =>
                Align(alignment: Alignment.centerRight, child: control),
              _ => Align(child: control),
            });
    }
    final body = row
        ? Row(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: switch (along[1]) {
              'top' => CrossAxisAlignment.start,
              'bottom' => CrossAxisAlignment.end,
              _ => CrossAxisAlignment.center,
            },
            children: placed,
          )
        : Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: placed,
          );
    if (c['type'] == 'panel') {
      return dialogGroup(t, c['text'] as String? ?? '', [body],
          key: ValueKey('script-${c['id']}'));
    }
    final m = (c['margins'] as List?)?.cast<num>() ?? const [0, 0, 0, 0];
    return Padding(
      padding: EdgeInsets.fromLTRB(m[0].toDouble(), m[1].toDouble(),
          m[2].toDouble(), m[3].toDouble()),
      child: body,
    );
  }

  Widget _control(LumitTheme t, Map<String, dynamic> c) {
    final id = c['id'] as String;
    final on = c['enabled'] != false;
    final tells = c['handles'] == true;
    final width = (c['width'] as num?)?.toDouble();
    final text = c['text'] as String? ?? '';
    final label = Text(text,
        style: t.small.copyWith(color: on ? t.textPrimary : t.textDisabled));
    Widget made;
    switch (c['type']) {
      case 'group' || 'panel':
        made = _holder(t, c);
      case 'statictext':
        made = SizedBox(
          width: width,
          child: Text(text,
              style: t.small.copyWith(color: t.textSecondary),
              softWrap: c['multiline'] == true || width != null),
        );
      case 'edittext':
        final field = _text(id);
        made = HouseTextField(
          key: ValueKey('script-$id'),
          controller: field,
          width: width ?? 200,
          multiline: c['multiline'] == true,
          // Otherwise what was typed goes with the next press.
          onSubmitted: (_) {
            if (tells && field.text != c['text']) _press(id);
          },
        );
      case 'checkbox':
        final v = _values[id] == true;
        made = _choice(
          label,
          HouseCheckbox(
            key: ValueKey('script-$id'),
            value: v,
            onChanged: on ? (v) => _set(id, v, tell: tells) : null,
          ),
          on ? () => _set(id, !v, tell: tells) : null,
        );
      case 'radiobutton':
        void pick() {
          // One of a group: the others in it go off.
          final parent = _find(widget.tree, (p) =>
              (p['children'] as List? ?? const [])
                  .any((k) => (k as Map)['id'] == id));
          for (final k in (parent?['children'] as List? ?? const [])) {
            final m = k as Map<String, dynamic>;
            if (m['type'] == 'radiobutton') _values[m['id'] as String] = false;
          }
          _set(id, true, tell: tells);
        }
        made = _choice(
          label,
          HouseRadio(
            key: ValueKey('script-$id'),
            selected: _values[id] == true,
            enabled: on,
            onChanged: pick,
          ),
          on ? pick : null,
        );
      case 'dropdownlist':
        final items = (c['items'] as List? ?? const []).cast<String>();
        final picked = _values[id] as int?;
        made = SizedBox(
          width: width ?? 160,
          child: BareDropdown<int>(
            key: ValueKey('script-$id'),
            value: picked ?? -1,
            options: [for (var i = 0; i < items.length; i++) i],
            label: (i) => i >= 0 && i < items.length ? items[i] : '',
            onChanged: on ? (i) => _set(id, i, tell: tells) : null,
          ),
        );
      case 'slider':
        final min = (c['min'] as num?)?.toDouble() ?? 0;
        final max = (c['max'] as num?)?.toDouble() ?? 100;
        final v = ((_values[id] as num?)?.toDouble() ?? min)
            .clamp(min, max < min ? min : max);
        made = HouseSlider(
          key: ValueKey('script-$id'),
          value: v,
          min: min,
          max: max < min ? min : max,
          width: width ?? 140,
          commitOnRelease: true,
          onChanged: (v) => _set(id, v, tell: tells),
        );
      case 'button':
        final role = c['role'];
        made = HouseButton(
          key: ValueKey('script-$id'),
          primary: role == 'ok',
          onPressed: on && (tells || role != null) ? () => _press(id) : null,
          child: Text(text),
        );
      default:
        made = const SizedBox.shrink();
    }
    if (!on && c['type'] != 'button' && c['type'] != 'checkbox') {
      made = IgnorePointer(child: Opacity(opacity: 0.5, child: made));
    }
    final help = c['help'] as String?;
    return help == null || help.isEmpty
        ? made
        : LumitTooltip(message: help, child: made);
  }

  /// A tick or a dot with its words, the words as clickable as the mark.
  Widget _choice(Widget label, Widget mark, VoidCallback? onTap) =>
      GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [mark, const SizedBox(width: 8), Flexible(child: label)],
        ),
      );
}
