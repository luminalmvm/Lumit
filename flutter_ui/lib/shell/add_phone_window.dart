// Add a phone: a code for the phone to scan, and how long it has left.
//
// The code is the server's, asked for as the window opens, and a fresh one
// replaces it. The window closes itself once a phone has taken it, which the
// account's room says ([PhoneLine.justPaired]).

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';
import 'package:qr/qr.dart';

import '../l10n/strings.dart';
import '../state/account.dart';
import '../state/app_state.dart';
import '../state/phone_line_frb.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'profile_chip.dart' show SyncRing;
import 'sign_in_window.dart' show cloudErrorText;

/// The side of the square the code is drawn in.
const double _square = 200;

/// Open the Add a phone window. Lumit says which phone it was paired with.
Future<void> showAddPhone(BuildContext context) async {
  final ui = context.read<LumitUiState>();
  final app = context.read<LumitState>();
  final name = await showLumitModal<String>(
    context: context,
    id: 'add-phone',
    builder: (close) => _AddPhone(line: ui.phones, close: close),
  );
  if (name != null) app.postNotice(l10n.phonePaired(name));
}

class _AddPhone extends StatefulWidget {
  final PhoneLine line;
  final void Function(String?) close;
  const _AddPhone({required this.line, required this.close});

  @override
  State<_AddPhone> createState() => _AddPhoneState();
}

class _AddPhoneState extends State<_AddPhone> {
  QrImage? _code;
  DateTime? _expires;
  String? _error;
  bool _asking = false;
  Timer? _tick;

  @override
  void initState() {
    super.initState();
    // A pairing from before the window opened is not this one.
    widget.line.justPaired = null;
    widget.line.addListener(_paired);
    _tick = Timer.periodic(const Duration(seconds: 1), (_) {
      if (_expires != null) setState(() {});
    });
    unawaited(_ask());
  }

  @override
  void dispose() {
    _tick?.cancel();
    widget.line.removeListener(_paired);
    super.dispose();
  }

  void _paired() {
    final name = widget.line.justPaired;
    if (name == null) return;
    widget.line.justPaired = null;
    widget.close(name);
  }

  Future<void> _ask() async {
    setState(() {
      _asking = true;
      _error = null;
    });
    try {
      final code = await widget.line.pair();
      if (!mounted) return;
      setState(() {
        _code = QrImage(QrCode.fromData(
            data: code.url, errorCorrectLevel: QrErrorCorrectLevel.M));
        _expires = code.expires;
      });
    } on CloudError catch (e) {
      if (mounted) setState(() => _error = cloudErrorText(e));
    }
    if (mounted) setState(() => _asking = false);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final left = _expires?.difference(DateTime.now());
    final live = _code != null && left != null && !left.isNegative;
    return DialogFrame(
      width: 280,
      children: [
        dialogTitleBar(
          t,
          title: l10n.phoneAdd,
          onClose: () => widget.close(null),
          keyPrefix: 'add-phone',
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(22, 20, 22, 18),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Center(
                child: ClipRRect(
                  borderRadius: BorderRadius.circular(t.tokens.sectionRadius),
                  child: SizedBox.square(
                    dimension: _square,
                    child: live
                        ? CustomPaint(
                            key: const ValueKey('add-phone-code'),
                            painter: _QrPainter(_code!, t.qrInk, t.qrPaper))
                        : ColoredBox(
                            color: t.surface2,
                            child: Center(child: _instead(t)),
                          ),
                  ),
                ),
              ),
              const SizedBox(height: 12),
              if (live)
                Text(
                  l10n.phoneCodeLeft('${left.inMinutes}:'
                      '${(left.inSeconds % 60).toString().padLeft(2, '0')}'),
                  key: const ValueKey('add-phone-left'),
                  textAlign: TextAlign.center,
                  style: t.mono,
                ),
              const SizedBox(height: 6),
              Text(
                l10n.phoneAddScan,
                textAlign: TextAlign.center,
                style: t.small.copyWith(color: t.textSecondary, height: 1.4),
              ),
            ],
          ),
        ),
      ],
    );
  }

  /// What the square holds while there is no code to scan: one on its way,
  /// or why there is none and the way to another.
  Widget _instead(LumitTheme t) {
    if (_asking) {
      return const SizedBox(width: 18, height: 18, child: SyncRing());
    }
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16),
          child: Text(
            _error ?? l10n.phoneCodeExpired,
            key: const ValueKey('add-phone-note'),
            textAlign: TextAlign.center,
            style: t.small.copyWith(
                color: _error == null ? t.textSecondary : t.warning,
                height: 1.4),
          ),
        ),
        const SizedBox(height: 10),
        HouseButton(
          key: const ValueKey('add-phone-again'),
          small: true,
          onPressed: () => unawaited(_ask()),
          child: Text(l10n.phoneCodeNew, style: t.small),
        ),
      ],
    );
  }
}

/// The code's modules, dark on light, with the four modules of quiet ground
/// round them a camera needs to find its edge.
class _QrPainter extends CustomPainter {
  final QrImage code;
  final Color ink, paper;
  const _QrPainter(this.code, this.ink, this.paper);

  @override
  void paint(Canvas canvas, Size size) {
    final cell = size.shortestSide / (code.moduleCount + 8);
    canvas.drawRect(Offset.zero & size, Paint()..color = paper);
    // One path, so neighbouring squares fill without a seam between them.
    final dark = Path();
    for (var y = 0; y < code.moduleCount; y++) {
      for (var x = 0; x < code.moduleCount; x++) {
        if (code.isDark(y, x)) {
          dark.addRect(
              Rect.fromLTWH((x + 4) * cell, (y + 4) * cell, cell, cell));
        }
      }
    }
    canvas.drawPath(dark, Paint()..color = ink);
  }

  @override
  bool shouldRepaint(_QrPainter old) =>
      old.code != code || old.ink != ink || old.paper != paper;
}
