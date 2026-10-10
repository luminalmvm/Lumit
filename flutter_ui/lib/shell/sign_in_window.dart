// Signing in to a Lumit account.
//
// One window with a few pages. An email and a password, then a line that
// says "or", then Google and Discord. Creating an account, typing the code
// that was emailed and choosing a new password are the same window turned to
// another page, so nobody is sent somewhere else to do them.
//
// Discord and Google are done in the person's own browser, where they are
// already signed in and where the address bar shows who is asking. The window
// waits here while that happens.
//
// Only what the server has switched on is offered ([CloudConfig]). The window
// asks it once when it opens.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/account.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'profile_chip.dart' show SyncRing;
import 'wordmark.dart';

/// The window's width, and the height of a field and a button in it. A
/// little taller than a dialogue's own controls: this is a form to type in,
/// not a row of settings.
const double signInWidth = 340;
const double _control = 28;

/// Open the sign-in window. True when it closed with somebody signed in.
Future<bool> showSignIn(BuildContext context) async {
  final ui = context.read<LumitUiState>();
  final signedIn = await showLumitModal<bool>(
    context: context,
    id: 'sign-in',
    builder: (close) => _SignIn(account: ui.account, close: close),
  );
  // Whatever was left waiting on a browser goes with the window.
  ui.account.cancelSignIn();
  return signedIn ?? false;
}

/// What a refusal is called in the person's own language.
String cloudErrorText(CloudError e) => switch (e.code) {
      'offline' => l10n.signInErrorOffline,
      'bad_credentials' => l10n.signInErrorCredentials,
      'bad_code' || 'expired' => l10n.signInErrorCode,
      'slow_down' => l10n.signInErrorSlow,
      'bad_email' => l10n.signInErrorEmail,
      'bad_name' => l10n.signInErrorName,
      'unverified_email' => l10n.signInErrorProvider,
      'timed_out' => l10n.signInErrorTimedOut,
      'no_browser' => l10n.signInErrorBrowser,
      _ => l10n.signInErrorGeneric,
    };

enum _Page { signIn, create, code, reset, browser }

class _SignIn extends StatefulWidget {
  final AccountState account;
  final void Function(bool?) close;
  const _SignIn({required this.account, required this.close});

  @override
  State<_SignIn> createState() => _SignInState();
}

class _SignInState extends State<_SignIn> {
  final _email = TextEditingController();
  final _password = TextEditingController();
  final _name = TextEditingController();
  final _code = TextEditingController();

  _Page _page = _Page.signIn;

  /// Whether the code page is finishing a new account or a new password.
  bool _resetting = false;
  bool _busy = false, _resent = false, _asked = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    // What the server offers may have changed since Lumit started, and may
    // never have been heard at all if it started offline.
    unawaited(widget.account.loadConfig().then((_) {
      if (mounted) setState(() => _asked = true);
    }));
  }

  @override
  void dispose() {
    _email.dispose();
    _password.dispose();
    _name.dispose();
    _code.dispose();
    super.dispose();
  }

  void _go(_Page page) => setState(() {
        _page = page;
        _error = null;
        _resent = false;
      });

  /// Run one of the calls, with the window saying it is busy, and show what
  /// came of it.
  Future<void> _try(Future<void> Function() call,
      {VoidCallback? then}) async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await call();
      if (!mounted) return;
      if (then != null) {
        setState(then);
      } else if (widget.account.signedIn) {
        widget.close(true);
      }
    } on CloudError catch (e) {
      if (!mounted) return;
      // An account made but not yet confirmed is sent its code again, and
      // this is the page to type it on.
      if (e.code == 'unverified') {
        setState(() {
          _resetting = false;
          _page = _Page.code;
        });
      } else if (e.code != 'cancelled') {
        setState(() => _error = cloudErrorText(e));
      }
    } catch (_) {
      if (mounted) setState(() => _error = l10n.signInErrorGeneric);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  /// What is wrong with what was typed, before the server is troubled.
  String? _unfit({bool name = false, bool password = true}) {
    final email = _email.text.trim();
    if (!RegExp(r'^[^@\s]+@[^@\s]+\.[^@\s]+$').hasMatch(email)) {
      return l10n.signInErrorEmail;
    }
    if (name && _name.text.trim().isEmpty) return l10n.signInErrorName;
    if (password && _password.text.length < shortestPassword) {
      return l10n.signInErrorPassword(shortestPassword);
    }
    return null;
  }

  void _submit() {
    final account = widget.account;
    final email = _email.text;
    switch (_page) {
      case _Page.signIn:
        // No length check here: a password chosen before the rule was what
        // it is still has to open its account.
        final unfit = _unfit(password: false);
        if (unfit != null || _password.text.isEmpty) {
          setState(() => _error = unfit ?? l10n.signInErrorCredentials);
          return;
        }
        unawaited(_try(() => account.signIn(email, _password.text)));
      case _Page.create:
        final unfit = _unfit(name: true);
        if (unfit != null) {
          setState(() => _error = unfit);
          return;
        }
        unawaited(_try(
            () => account.signUp(email, _password.text, _name.text),
            then: () {
          _resetting = false;
          _page = _Page.code;
        }));
      case _Page.code:
        unawaited(
            _try(() => account.verify(email, _code.text, _password.text)));
      case _Page.reset:
        if (_password.text.length < shortestPassword) {
          setState(
              () => _error = l10n.signInErrorPassword(shortestPassword));
          return;
        }
        unawaited(_try(
            () => account.resetFinish(email, _code.text, _password.text)));
      case _Page.browser:
    }
  }

  void _forgot() {
    final unfit = _unfit(password: false);
    if (unfit != null) {
      setState(() => _error = unfit);
      return;
    }
    unawaited(_try(() => widget.account.resetStart(_email.text), then: () {
      _resetting = true;
      _password.clear();
      _code.clear();
      _page = _Page.reset;
    }));
  }

  void _resend() => unawaited(_try(
      () => _resetting
          ? widget.account.resetStart(_email.text)
          : widget.account.signUp(_email.text, _password.text, _name.text),
      then: () => _resent = true));

  Future<void> _with(SignInWith provider) async {
    _go(_Page.browser);
    try {
      await widget.account.signInWith(provider);
      if (mounted) widget.close(true);
    } on CloudError catch (e) {
      if (!mounted) return;
      setState(() {
        _page = _Page.signIn;
        _error = e.code == 'cancelled' ? null : cloudErrorText(e);
      });
    } catch (_) {
      if (!mounted) return;
      setState(() {
        _page = _Page.signIn;
        _error = l10n.signInErrorGeneric;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final config = widget.account.config;
    return DialogFrame(
      width: signInWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.signInTitle,
          onClose: () => widget.close(false),
          keyPrefix: 'sign-in',
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(22, 20, 22, 18),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Center(child: LumitWordmark(height: 18, ground: t.surface1)),
              const SizedBox(height: 16),
              Text(
                switch (_page) {
                  _Page.create => l10n.signInCreateHeading,
                  _Page.code => l10n.signInCodeHeading,
                  _Page.reset => l10n.signInResetHeading,
                  _ => l10n.signInHeading,
                },
                key: const ValueKey('sign-in-heading'),
                textAlign: TextAlign.center,
                style: t.heading,
              ),
              // Only the pages that have something to say: where the code
              // went, or that the browser is waiting.
              if (switch (_page) {
                _Page.code => l10n.signInCodeLead(_email.text.trim()),
                _Page.reset => l10n.signInResetLead(_email.text.trim()),
                _Page.browser => l10n.signInBrowser,
                _ => null,
              }
                  case final words?) ...[
                const SizedBox(height: 6),
                Text(
                  words,
                  textAlign: TextAlign.center,
                  style:
                      t.small.copyWith(color: t.textSecondary, height: 1.4),
                ),
              ],
              const SizedBox(height: 16),
              if (config == null && !_asked)
                const Padding(
                  padding: EdgeInsets.symmetric(vertical: 18),
                  child: Center(
                      child: SizedBox(width: 18, height: 18, child: SyncRing())),
                )
              else if (config == null)
                _note(t, l10n.signInErrorOffline, warning: true)
              else if (!config.anySignIn)
                _note(t, l10n.signInUnavailable)
              else
                ..._pageRows(t, config),
              if (_error case final error?) ...[
                const SizedBox(height: 10),
                _note(t, error, warning: true),
              ],
            ],
          ),
        ),
      ],
    );
  }

  Widget _note(LumitTheme t, String words, {bool warning = false}) => Text(
        words,
        key: warning ? const ValueKey('sign-in-error') : null,
        textAlign: TextAlign.center,
        style: t.small.copyWith(
            color: warning ? t.warning : t.textSecondary, height: 1.4),
      );

  List<Widget> _pageRows(LumitTheme t, CloudConfig config) => switch (_page) {
        _Page.signIn => [
            if (config.email) ...[
              _field(t, 'email', _email, l10n.signInEmail, autofocus: true),
              const SizedBox(height: 8),
              _field(t, 'password', _password, l10n.signInPassword,
                  obscure: true),
              const SizedBox(height: 12),
              _action(t, l10n.signInAction),
              const SizedBox(height: 10),
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  _link(t, 'forgot', l10n.signInForgot, _forgot),
                  _link(t, 'create', l10n.signInNoAccount,
                      () => _go(_Page.create)),
                ],
              ),
            ],
            ..._others(t, config),
          ],
        _Page.create => [
            _field(t, 'name', _name, l10n.signInName, autofocus: true),
            const SizedBox(height: 8),
            _field(t, 'email', _email, l10n.signInEmail),
            const SizedBox(height: 8),
            _field(t, 'password', _password, l10n.signInPassword,
                obscure: true),
            const SizedBox(height: 12),
            _action(t, l10n.signInCreateAction),
            const SizedBox(height: 10),
            Center(
              child: _link(t, 'have', l10n.signInHaveAccount,
                  () => _go(_Page.signIn)),
            ),
            ..._others(t, config),
          ],
        _Page.code || _Page.reset => [
            _field(t, 'code', _code, l10n.signInCode,
                autofocus: true, centred: true),
            if (_page == _Page.reset) ...[
              const SizedBox(height: 8),
              _field(t, 'password', _password, l10n.signInNewPassword,
                  obscure: true),
            ],
            const SizedBox(height: 12),
            _action(t, l10n.signInContinue),
            const SizedBox(height: 10),
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                _link(t, 'back', l10n.signInBack, () => _go(_Page.signIn)),
                _resent
                    ? Text(l10n.signInResent,
                        style: t.small.copyWith(color: t.textMuted))
                    : _link(t, 'resend', l10n.signInResend, _resend),
              ],
            ),
          ],
        _Page.browser => [
            const Padding(
              padding: EdgeInsets.symmetric(vertical: 10),
              child: Center(
                  child: SizedBox(width: 22, height: 22, child: SyncRing())),
            ),
            const SizedBox(height: 6),
            Center(
              child: HouseButton(
                key: const ValueKey('sign-in-cancel'),
                small: true,
                onPressed: () {
                  widget.account.cancelSignIn();
                  _go(_Page.signIn);
                },
                child: Text(l10n.cancel, style: t.small),
              ),
            ),
          ],
      };

  /// The line that says "or", and the two buttons under it.
  List<Widget> _others(LumitTheme t, CloudConfig config) {
    if (!config.discord && !config.google) return const [];
    return [
      if (config.email)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 14),
          child: Row(
            children: [
              Expanded(child: Container(height: 1, color: t.hairline)),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 10),
                child: Text(l10n.signInOr,
                    style: t.small.copyWith(color: t.textMuted)),
              ),
              Expanded(child: Container(height: 1, color: t.hairline)),
            ],
          ),
        ),
      if (config.google)
        _provider(t, SignInWith.google, l10n.signInGoogle,
            'assets/brand/google.svg'),
      if (config.discord && config.google) const SizedBox(height: 8),
      if (config.discord)
        _provider(t, SignInWith.discord, l10n.signInDiscord,
            'assets/brand/discord.svg',
            tinted: true),
    ];
  }

  Widget _field(LumitTheme t, String id, TextEditingController controller,
          String hint,
          {bool obscure = false, bool autofocus = false, bool centred = false}) =>
      SizedBox(
        height: _control,
        child: HouseTextField(
          key: ValueKey<String>('sign-in-$id'),
          controller: controller,
          width: double.infinity,
          padding: const EdgeInsets.symmetric(horizontal: 10),
          hint: hint,
          obscure: obscure,
          autofocus: autofocus,
          textAlign: centred ? TextAlign.center : TextAlign.start,
          style: centred
              ? t.mono.copyWith(color: t.textPrimary, letterSpacing: 4)
              : null,
          onSubmitted: (_) => _submit(),
        ),
      );

  /// The one filled button. While a password is being stretched it says so,
  /// since that is a couple of seconds of nothing else happening.
  Widget _action(LumitTheme t, String label) => SizedBox(
        height: _control,
        child: HouseButton(
          key: const ValueKey('sign-in-action'),
          primary: true,
          onPressed: _busy ? null : _submit,
          child: _busy
              ? Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const SizedBox(width: 12, height: 12, child: SyncRing()),
                    const SizedBox(width: 8),
                    Text(l10n.signInWorking),
                  ],
                )
              : Text(label),
        ),
      );

  Widget _provider(LumitTheme t, SignInWith provider, String label,
          String asset,
          {bool tinted = false}) =>
      SizedBox(
        height: _control,
        child: HouseButton(
          key: ValueKey<String>('sign-in-${provider.name}'),
          onPressed: _busy ? null : () => _with(provider),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              SvgPicture.asset(
                asset,
                width: 14,
                height: 14,
                // Discord's mark takes the text colour. Google's keeps its
                // own four, which are the mark.
                theme: SvgTheme(
                    currentColor: tinted ? t.textPrimary : t.textPrimary),
              ),
              const SizedBox(width: 8),
              Text(label),
            ],
          ),
        ),
      );

  Widget _link(LumitTheme t, String id, String label, VoidCallback onTap) =>
      _Link(key: ValueKey<String>('sign-in-$id'), label: label, onTap: onTap);
}

/// A line of text that is pressed: quiet until the pointer is on it.
class _Link extends StatefulWidget {
  final String label;
  final VoidCallback onTap;
  const _Link({super.key, required this.label, required this.onTap});

  @override
  State<_Link> createState() => _LinkState();
}

class _LinkState extends State<_Link> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: widget.onTap,
        child: Text(
          widget.label,
          style: t.small.copyWith(color: _hover ? t.textPrimary : t.accent),
        ),
      ),
    );
  }
}
