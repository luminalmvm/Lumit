// The Lumit Pro window: what Pro is, what it costs, and the way to it.
//
// One window that reads the account and shows whichever of these is true.
// Nobody is signed in. Somebody is, without Pro. They have been sent to the
// shop and Lumit is waiting to hear. They have Pro.
//
// Buying happens in the browser, at the shop. Lumit never sees a card. It
// opens the shop's page for this account, then asks the server every few
// seconds whether Pro has arrived, and says thank you when it has.
//
// The window is Lumit's own surfaces and type. What sets it apart is the
// sweep, the brand's two keys run together, which is drawn nowhere else in
// the application but on things that are Pro.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/account.dart';
import '../state/external_links.dart';
import '../state/ui_state.dart';
import '../theme/brand.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'profile_chip.dart';
import 'sign_in_window.dart';
import 'wordmark.dart';

const double proWindowWidth = 460;

/// Open the Lumit Pro window.
Future<void> showProWindow(BuildContext context) {
  final ui = context.read<LumitUiState>();
  return showLumitModal<void>(
    context: context,
    id: 'lumit-pro',
    builder: (close) => _ProWindow(ui: ui, close: () => close(null)),
  );
}

/// A date as it is written out in the language in use.
String writtenDate(DateTime date) {
  try {
    // The English here is British, which the bare tag does not say.
    final tag = l10n.localeName;
    return DateFormat.yMMMMd(tag == 'en' ? 'en_GB' : tag).format(date);
  } catch (_) {
    // A language whose dates are not loaded gets the plain form.
    return DateFormat.yMMMMd().format(date);
  }
}

/// The same without the year when it is this one, for a day in a list: 4
/// October, and 4 October 2025 once that is last year.
String writtenDay(DateTime date) {
  if (date.year != DateTime.now().year) return writtenDate(date);
  try {
    final tag = l10n.localeName;
    return DateFormat.MMMMd(tag == 'en' ? 'en_GB' : tag).format(date);
  } catch (_) {
    return DateFormat.MMMMd().format(date);
  }
}

/// A size in bytes as it is written: 178 KB, 1.7 MB, 20 GB. In thousands,
/// which is how the server counts an account's room, and with one decimal
/// place only where it says something.
String writtenBytes(int bytes) {
  final (amount, unit) = bytes >= 1000000000
      ? (bytes / 1e9, l10n.unitGb)
      : bytes >= 1000000
          ? (bytes / 1e6, l10n.unitMb)
          : (bytes / 1e3, l10n.unitKb);
  final written = amount.toStringAsFixed(amount < 10 ? 1 : 0);
  final neat = written.endsWith('.0')
      ? written.substring(0, written.length - 2)
      : written;
  return '$neat $unit';
}

/// A price as the shop gave it, with its currency's own sign where there is
/// one.
String writtenPrice(CloudPrice price) {
  final amount = double.tryParse(price.amount);
  if (amount == null) return '${price.amount} ${price.currency}';
  try {
    return NumberFormat.simpleCurrency(name: price.currency).format(amount);
  } catch (_) {
    return '${price.amount} ${price.currency}';
  }
}

/// What a subscription has to say for itself, or null when it has nothing
/// that needs saying.
(String, bool)? subscriptionLine(CloudAccount account) {
  final plan = account.subscription;
  if (plan == null) return null;
  if (plan.status == 'past_due') return (l10n.proPastDue, true);
  if (plan.ends case final ends?) return (l10n.proEnds(writtenDate(ends)), false);
  if (plan.renews case final renews?) {
    return (l10n.proRenews(writtenDate(renews)), false);
  }
  return null;
}

class _ProWindow extends StatefulWidget {
  final LumitUiState ui;
  final VoidCallback close;
  const _ProWindow({required this.ui, required this.close});

  @override
  State<_ProWindow> createState() => _ProWindowState();
}

class _ProWindowState extends State<_ProWindow>
    with SingleTickerProviderStateMixin {
  bool _yearly = true;
  bool _busy = false, _waiting = false;
  String? _error;

  /// Whether the account had Pro when the window opened. If it did not and
  /// now does, that happened here, and the window says thank you for it.
  late final bool _hadPro;

  /// Plays once, when Pro arrives while the window is open.
  late final AnimationController _arrive = AnimationController(
      vsync: this, duration: const Duration(milliseconds: 900));

  AccountState get _account => widget.ui.account;

  @override
  void initState() {
    super.initState();
    _hadPro = _account.pro;
    _account.addListener(_changed);
    // The prices, and whether Pro is on sale at all, as they stand now.
    unawaited(_account.loadConfig());
    if (_account.signedIn) unawaited(_account.refreshAccount());
  }

  @override
  void dispose() {
    _account.removeListener(_changed);
    _arrive.dispose();
    super.dispose();
  }

  void _changed() {
    if (!mounted) return;
    if (_account.pro && !_hadPro && _arrive.status == AnimationStatus.dismissed) {
      final still =
          ThemeScope.of(context).animationLevel == AnimationLevel.none;
      still ? _arrive.value = 1 : _arrive.forward();
      _waiting = false;
    }
    setState(() {});
  }

  Future<void> _buy() async {
    if (!_account.signedIn) {
      if (!await showSignIn(context) || !mounted) return;
      // Signing in may have been all it took: an account that already has
      // Pro, or Pro being free for now.
      if (_account.pro) return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final url = await _account.checkoutUrl(yearly: _yearly);
      if (!await openExternalLink(url)) {
        throw const CloudError('no_browser');
      }
      if (!mounted) return;
      setState(() => _waiting = true);
      unawaited(_account.awaitPro().then((_) {
        if (mounted) setState(() => _waiting = false);
      }));
    } on CloudError catch (e) {
      if (mounted) setState(() => _error = cloudErrorText(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final account = _account.account;
    final config = _account.config;
    final pro = account?.pro ?? false;
    return DialogFrame(
      width: proWindowWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.proTitle,
          onClose: widget.close,
          keyPrefix: 'pro',
        ),
        ProHero(
          arrive: _arrive,
          headline: pro ? l10n.proThanksHeadline : l10n.proHeadline,
          lead: pro
              ? (account!.preview ? l10n.proPreviewNote : l10n.proThanksBody)
              : null,
          pro: pro,
          preview: pro ? account!.preview : (config?.preview ?? false),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(22, 4, 22, 18),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _Feature(
                index: 0,
                mark: LumitIcons.autoWire,
                title: l10n.proFeatureRelayTitle,
                body: l10n.proFeatureRelayBody,
                on: pro,
              ),
              _Feature(
                index: 1,
                // The link, which is what this wears everywhere else.
                mark: LumitIcons.link,
                title: l10n.proFeatureOnlineTitle,
                body: l10n.proFeatureOnlineBody,
                on: pro,
              ),
              _Feature(
                index: 2,
                mark: LumitIcons.comment,
                title: l10n.proFeatureReviewTitle,
                body: l10n.proFeatureReviewBody,
                on: pro,
              ),
              _Feature(
                index: 3,
                mark: LumitIcons.undo,
                title: l10n.proFeatureBackupTitle,
                body: l10n.proFeatureBackupBody,
                on: pro,
              ),
              _Feature(
                index: 4,
                mark: LumitIcons.loop,
                title: l10n.proFeatureSyncTitle,
                body: l10n.proFeatureSyncBody,
                on: pro,
              ),
              const SizedBox(height: 14),
              ..._foot(t, account, config),
            ],
          ),
        ),
      ],
    );
  }

  /// Under the features: the plans and the button, or what stands in for
  /// them once there is nothing left to buy.
  List<Widget> _foot(LumitTheme t, CloudAccount? account, CloudConfig? config) {
    Widget note(String words, {bool warning = false}) => Text(
          words,
          textAlign: TextAlign.center,
          style: t.small.copyWith(
              color: warning ? t.warning : t.textSecondary, height: 1.4),
        );
    Widget action(String id, String label, VoidCallback? onPressed,
            {bool primary = true}) =>
        SizedBox(
          height: 30,
          child: HouseButton(
            key: ValueKey<String>('pro-$id'),
            primary: primary,
            onPressed: onPressed,
            child: Text(label),
          ),
        );

    if (account?.pro ?? false) {
      final line = subscriptionLine(account!);
      return [
        if (line != null) ...[
          note(line.$1, warning: line.$2),
          const SizedBox(height: 12),
        ],
        if (account.onIphone) ...[
          note(l10n.proManagedOnIphone),
          const SizedBox(height: 12),
        ] else if (account.subscription != null) ...[
          action('manage', l10n.proManage,
              () => unawaited(openSubscriptionPage(_account)),
              primary: false),
          const SizedBox(height: 8),
        ],
        action('done', l10n.proClose, widget.close),
      ];
    }
    if (_waiting) {
      return [
        const Center(
            child: SizedBox(width: 20, height: 20, child: SyncRing())),
        const SizedBox(height: 10),
        note(l10n.proWaiting),
        const SizedBox(height: 10),
        action('not-now', l10n.proWaitingCancel,
            () => setState(() => _waiting = false),
            primary: false),
      ];
    }
    final preview = config?.preview ?? false;
    final onSale = config?.billing ?? false;
    return [
      if (onSale && config?.monthly != null && config?.yearly != null) ...[
        Row(
          children: [
            Expanded(
              child: _Plan(
                id: 'monthly',
                name: l10n.proMonthly,
                price: writtenPrice(config!.monthly!),
                per: l10n.proPerMonth,
                chosen: !_yearly,
                onPick: () => setState(() => _yearly = false),
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: _Plan(
                id: 'yearly',
                name: l10n.proYearly,
                price: writtenPrice(config.yearly!),
                per: l10n.proPerYear,
                saving: _saving(config.monthly!, config.yearly!),
                chosen: _yearly,
                onPick: () => setState(() => _yearly = true),
              ),
            ),
          ],
        ),
        const SizedBox(height: 14),
      ],
      if (preview)
        action(
            'buy',
            l10n.proSignInPreview,
            () => unawaited(showSignIn(context)))
      else if (onSale)
        action(
            'buy',
            account == null ? l10n.proSignInFirst : l10n.proBuy,
            _busy ? null : () => unawaited(_buy()))
      else
        note(config == null ? l10n.signInErrorOffline : l10n.proNotOnSale,
            warning: config == null),
      if (_error case final error?) ...[
        const SizedBox(height: 10),
        note(error, warning: true),
      ],
      if (preview) ...[
        const SizedBox(height: 12),
        Text(l10n.proPreviewNote,
            textAlign: TextAlign.center,
            style: t.caption.copyWith(color: t.textMuted, height: 1.4)),
      ],
    ];
  }

  /// How much less a year costs than twelve months, as a whole percentage,
  /// or null when it does not.
  int? _saving(CloudPrice monthly, CloudPrice yearly) {
    final month = double.tryParse(monthly.amount);
    final year = double.tryParse(yearly.amount);
    if (month == null || year == null || month <= 0) return null;
    final saved = (1 - year / (month * 12)) * 100;
    return saved >= 1 ? saved.round() : null;
  }
}

/// The top of the window: the mark, a headline and a line under it, standing
/// in front of a slow wash of the sweep.
class ProHero extends StatelessWidget {
  final Animation<double> arrive;
  final String headline;

  /// A line under the headline, once there is something to say there.
  final String? lead;
  final bool pro, preview;

  const ProHero({
    super.key,
    this.arrive = kAlwaysDismissedAnimation,
    required this.headline,
    this.lead,
    this.pro = false,
    this.preview = false,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ClipRect(
      child: Stack(
        children: [
          Positioned.fill(
            child: AnimatedBuilder(
              animation: arrive,
              builder: (context, _) => CustomPaint(
                painter: ProWashPainter(
                  t.surface1,
                  // Brighter for a moment as Pro arrives, then settling a
                  // little above where it began.
                  0.16 + 0.20 * math.sin(arrive.value * math.pi) +
                      (pro ? 0.06 : 0),
                ),
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(22, 26, 22, 18),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Row(
                  mainAxisAlignment: MainAxisAlignment.center,
                  children: [
                    LumitWordmark(height: 20, ground: t.surface1),
                    const SizedBox(width: 8),
                    ProBadge(preview: preview),
                  ],
                ),
                const SizedBox(height: 18),
                AnimatedSwitcher(
                  duration: ThemeScope.of(context).motion.swap.duration,
                  child: Text(
                    headline,
                    key: ValueKey<String>(headline),
                    textAlign: TextAlign.center,
                    style: t.heading.copyWith(fontSize: 20, height: 1.2),
                  ),
                ),
                if (lead case final lead?) ...[
                  const SizedBox(height: 8),
                  Text(
                    lead,
                    key: const ValueKey('pro-lead'),
                    textAlign: TextAlign.center,
                    style:
                        t.body.copyWith(color: t.textSecondary, height: 1.45),
                  ),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// What a window shows in place of something that comes with Lumit Pro: the
/// hero, with [headline] saying what it is, and the one button that opens
/// the Lumit Pro window.
class ProOffer extends StatelessWidget {
  final String headline;

  /// Whether Pro is free for everyone for now, which the badge says.
  final bool preview;

  const ProOffer({super.key, required this.headline, this.preview = false});

  @override
  Widget build(BuildContext context) => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          ProHero(headline: headline, preview: preview),
          Padding(
            padding: const EdgeInsets.fromLTRB(22, 6, 22, 22),
            child: SizedBox(
              height: 30,
              child: HouseButton(
                key: const ValueKey('pro-offer'),
                primary: true,
                onPressed: () => unawaited(showProWindow(context)),
                child: Text(l10n.menuLumitPro),
              ),
            ),
          ),
        ],
      );
}

/// Two soft lights, one of each key, falling away into the surface.
class ProWashPainter extends CustomPainter {
  final Color ground;
  final double strength;
  const ProWashPainter(this.ground, this.strength);

  @override
  void paint(Canvas canvas, Size size) {
    final box = Offset.zero & size;
    canvas.drawRect(box, Paint()..color = ground);
    void light(Alignment at, Color colour) {
      canvas.drawRect(
        box,
        Paint()
          ..shader = RadialGradient(
            center: at,
            radius: 0.9,
            colors: [
              colour.withValues(alpha: strength.clamp(0.0, 1.0)),
              colour.withValues(alpha: 0),
            ],
          ).createShader(box),
      );
    }

    light(const Alignment(-0.9, -1.4), brandProSweep.first);
    light(const Alignment(0.9, -1.4), brandProSweep.last);
  }

  @override
  bool shouldRepaint(ProWashPainter old) =>
      old.strength != strength || old.ground != ground;
}

/// One thing Pro does: a glyph in the sweep, a title and a sentence. Each
/// comes in a beat after the one above it.
class _Feature extends StatelessWidget {
  final int index;
  final String mark, title, body;

  /// Whether the account has it, which ticks it.
  final bool on;

  const _Feature({
    required this.index,
    required this.mark,
    required this.title,
    required this.body,
    required this.on,
  });

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final radius = BorderRadius.circular(
        math.max(t.tokens.controlRadius, t.tokens.sectionRadius / 2));
    final row = Padding(
      padding: const EdgeInsets.symmetric(vertical: 7),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 28,
            height: 28,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              borderRadius: radius,
              color: Color.alphaBlend(
                  brandKeyJade.withValues(alpha: 0.10), t.surface2),
              border: Border.all(color: t.hairline),
            ),
            child: ShaderMask(
              blendMode: BlendMode.srcIn,
              shaderCallback: (bounds) => proSweep.createShader(bounds),
              child: glyph.LumitIcon(mark, size: 18, colour: t.textPrimary),
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title, style: t.bodyStrong.copyWith(color: t.textPrimary)),
                const SizedBox(height: 2),
                Text(body,
                    style: t.small
                        .copyWith(color: t.textSecondary, height: 1.4)),
              ],
            ),
          ),
          if (on) ...[
            const SizedBox(width: 10),
            Padding(
              padding: const EdgeInsets.only(top: 6),
              child: glyph.LumitIcon(LumitIcons.tick,
                  size: 14, colour: t.success),
            ),
          ],
        ],
      ),
    );
    if (scope.animationLevel != AnimationLevel.all) return row;
    // Held back a beat for each row above it, then up and in.
    return TweenAnimationBuilder<double>(
      tween: Tween(begin: 0, end: 1),
      duration: Duration(milliseconds: 260 + index * 70),
      curve: Interval(index * 70 / (260 + index * 70), 1,
          curve: scope.motion.reveal.curve),
      builder: (context, shown, child) => Opacity(
        opacity: shown,
        child: Transform.translate(
            offset: Offset(0, (1 - shown) * 6), child: child),
      ),
      child: row,
    );
  }
}

/// One of the two ways to pay, as a card that is picked.
class _Plan extends StatefulWidget {
  final String id, name, price, per;
  final int? saving;
  final bool chosen;
  final VoidCallback onPick;

  const _Plan({
    required this.id,
    required this.name,
    required this.price,
    required this.per,
    this.saving,
    required this.chosen,
    required this.onPick,
  });

  @override
  State<_Plan> createState() => _PlanState();
}

class _PlanState extends State<_Plan> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final radius = BorderRadius.circular(
        math.max(t.tokens.controlRadius, t.tokens.sectionRadius));
    final edge = widget.chosen
        ? brandProSweep
        : [
            for (final _ in brandProSweep)
              _hover ? t.hairlineStrong : t.hairline,
          ];
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        key: ValueKey<String>('pro-plan-${widget.id}'),
        behavior: HitTestBehavior.opaque,
        onTap: widget.onPick,
        child: AnimatedContainer(
          duration: scope.motion.hoverIn.duration,
          padding: const EdgeInsets.all(1),
          decoration: BoxDecoration(
            borderRadius: radius,
            gradient: LinearGradient(
              begin: Alignment.topLeft,
              end: Alignment.bottomRight,
              colors: edge,
            ),
          ),
          child: Container(
            padding: const EdgeInsets.fromLTRB(12, 10, 12, 11),
            decoration: BoxDecoration(
              borderRadius: radius,
              color: widget.chosen
                  ? Color.alphaBlend(
                      brandKeyJade.withValues(alpha: 0.07), t.surface2)
                  : t.surface2,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Text(widget.name,
                        style: widget.chosen ? t.kickerOn : t.kicker),
                    const Spacer(),
                    if (widget.saving case final saving?)
                      Text(l10n.proSaving(saving),
                          style: t.caption.copyWith(color: t.success)),
                  ],
                ),
                const SizedBox(height: 8),
                Row(
                  crossAxisAlignment: CrossAxisAlignment.baseline,
                  textBaseline: TextBaseline.alphabetic,
                  children: [
                    Text(widget.price,
                        style: t.heading.copyWith(fontSize: 18, height: 1)),
                    const SizedBox(width: 5),
                    Flexible(
                      child: Text(widget.per,
                          style: t.small.copyWith(color: t.textMuted),
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
