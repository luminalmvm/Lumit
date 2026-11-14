// The status line's word for a laptop on battery or saving power.
//
// **In plain terms.** A laptop on battery, or with its power saver on, holds
// the graphics card at low clocks, and a frame that takes a few milliseconds
// plugged in can take ten times that. Somebody whose editor has gone slow
// rarely suspects the power plan, so the status line says it, with the way
// out in the tooltip. Nothing shows while the machine is plugged in and at
// full power, or where the system cannot be asked.

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../src/rust/api/system.dart' show powerSaving;
import '../widgets/controls.dart';

/// Asks the system every few seconds whether it is saving power, and shows
/// [wrap] of a short warning while it is.
class PowerSavingMark extends StatefulWidget {
  /// Puts the mark in the status line's own pill and divider.
  final Widget Function(Widget mark) wrap;

  /// The question, which the tests answer themselves.
  final bool Function() ask;

  const PowerSavingMark({super.key, required this.wrap, this.ask = powerSaving});

  @override
  State<PowerSavingMark> createState() => _PowerSavingMarkState();
}

class _PowerSavingMarkState extends State<PowerSavingMark> {
  Timer? _timer;
  bool _saving = false;

  @override
  void initState() {
    super.initState();
    _check();
    _timer = Timer.periodic(const Duration(seconds: 5), (_) => _check());
  }

  void _check() {
    bool saving;
    try {
      saving = widget.ask();
    } catch (_) {
      saving = false;
    }
    if (saving != _saving && mounted) setState(() => _saving = saving);
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (!_saving) return const SizedBox.shrink();
    final t = ThemeScope.of(context).theme;
    return widget.wrap(LumitTooltip(
      message: l10n.tipPowerSaving,
      child: Text(
        l10n.powerSaving,
        key: const ValueKey('status-power-saving'),
        style: t.small.copyWith(color: t.warning),
        maxLines: 1,
        softWrap: false,
      ),
    ));
  }
}
