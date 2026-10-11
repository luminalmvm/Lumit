// The profile button on the bottom strip, and the card it opens.
//
// The button says whose settings these are: a picture or an initial, and a
// name. The card is everything about that in one place. Who this is and
// whether they are signed in, what Lumit Pro is doing for them or would,
// the other profiles on this machine, and the way to Settings.
//
// Nothing here asks the server anything. It reads what [AccountState] and
// [ProfilesState] already hold, and is redrawn when they change.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/account.dart';
import '../state/external_links.dart';
import '../state/profiles.dart';
import '../state/ui_state.dart';
import '../theme/brand.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'pro_window.dart';
import 'settings_window_frb.dart';
import 'sign_in_window.dart';

/// How wide the card is drawn.
const double profileCardWidth = 288;

/// A profile's name, or the stand-in for one that has none.
String profileName(Profile profile) =>
    profile.name.trim().isEmpty ? l10n.profileUnnamed : profile.name.trim();

/// The sweep that marks Lumit Pro, corner to corner.
const LinearGradient proSweep = LinearGradient(
  begin: Alignment.topLeft,
  end: Alignment.bottomRight,
  colors: brandProSweep,
);

/// A profile's picture: the one its account came with, or its initial on
/// its own colour. With [pro] it stands inside the Pro ring.
class ProfileAvatar extends StatelessWidget {
  final Profile profile;
  final double size;
  final bool pro;

  const ProfileAvatar(this.profile,
      {super.key, required this.size, this.pro = false});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final tint = t.personColour(profile.colour);
    final picture = profile.account?.avatar;
    // The ring is a tenth of the disc and never thinner than a pixel and a
    // half, with the same again of the surface showing between the two.
    final ring = pro ? math.max(1.5, size / 14) : 0.0;
    final inner = size - ring * 4;
    final called = profileName(profile);
    final initial = Container(
      width: inner,
      height: inner,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: Color.alphaBlend(tint.withValues(alpha: 0.28), t.surface3),
        border: Border.all(color: tint.withValues(alpha: 0.6)),
      ),
      child: Text(
        String.fromCharCode(called.runes.first).toUpperCase(),
        style: t.bodyStrong.copyWith(
          color: t.textPrimary,
          fontSize: inner * 0.46,
          height: 1,
        ),
      ),
    );
    final face = picture == null
        ? initial
        : ClipOval(
            child: Image.network(
              picture,
              width: inner,
              height: inner,
              fit: BoxFit.cover,
              // A picture that will not load is the initial, not a hole.
              errorBuilder: (_, __, ___) => initial,
              frameBuilder: (_, child, frame, __) =>
                  frame == null ? initial : child,
            ),
          );
    if (!pro) return SizedBox(width: size, height: size, child: Center(child: face));
    return Container(
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: const BoxDecoration(shape: BoxShape.circle, gradient: proSweep),
      child: Container(
        width: size - ring * 2,
        height: size - ring * 2,
        alignment: Alignment.center,
        decoration: BoxDecoration(shape: BoxShape.circle, color: t.surface1),
        child: face,
      ),
    );
  }
}

/// The small badge that says Pro, lettered in the sweep.
class ProBadge extends StatelessWidget {
  final bool preview;
  const ProBadge({super.key, this.preview = false});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final radius = BorderRadius.circular(
        t.tokens.actionRadius == 0 ? 0 : math.max(3, t.tokens.controlRadius));
    return Container(
      padding: const EdgeInsets.all(1),
      decoration: BoxDecoration(gradient: proSweep, borderRadius: radius),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
        decoration: BoxDecoration(color: t.surface1, borderRadius: radius),
        child: ShaderMask(
          blendMode: BlendMode.srcIn,
          shaderCallback: (bounds) => proSweep.createShader(bounds),
          child: Text(
            (preview ? l10n.proPreviewTag : l10n.proTag).toUpperCase(),
            style: t.kicker.copyWith(color: t.textPrimary, height: 1.2),
          ),
        ),
      ),
    );
  }
}

/// The button on the bottom strip.
class ProfileChip extends StatefulWidget {
  const ProfileChip({super.key});

  @override
  State<ProfileChip> createState() => _ProfileChipState();
}

class _ProfileChipState extends State<ProfileChip> {
  bool _open = false;

  Future<void> _show(LumitUiState ui) async {
    final box = context.findRenderObject()! as RenderBox;
    setState(() => _open = true);
    // Anchored at the button's own corner. The card is taller than the room
    // under that, so the popup's own placing lifts it clear of the window's
    // edge, and the gap it carries under itself stands it off the strip.
    await showLumitPopup<void>(
      context: context,
      position: box.localToGlobal(Offset.zero),
      builder: (close) => Padding(
        padding: EdgeInsets.only(bottom: box.size.height + 10),
        child: _ProfileCard(ui: ui, close: () => close(null), host: context),
      ),
    );
    if (mounted) setState(() => _open = false);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = context.read<LumitUiState>();
    return ListenableBuilder(
      listenable: Listenable.merge([ui.profiles, ui.account]),
      builder: (context, _) {
        final profile = ui.profiles.current;
        final account = ui.account.account;
        return LumitTooltip(
          message: l10n.profileButtonTip,
          child: HouseButton(
            key: const ValueKey('status-profile'),
            frameless: true,
            small: true,
            active: _open,
            padding: const EdgeInsets.symmetric(horizontal: 4),
            onPressed: () => _show(ui),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                ProfileAvatar(profile, size: 14, pro: account?.pro ?? false),
                const SizedBox(width: 6),
                ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 120),
                  child: Text(profileName(profile),
                      style: t.small.copyWith(color: t.textPrimary),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _ProfileCard extends StatelessWidget {
  final LumitUiState ui;
  final VoidCallback close;

  /// A context that outlives the card, for the windows its rows open once
  /// the card itself has gone.
  final BuildContext host;

  const _ProfileCard(
      {required this.ui, required this.close, required this.host});

  /// Close the card, then do [then] from the strip's own place in the tree.
  void _leave(void Function(BuildContext context) then) {
    close();
    if (host.mounted) then(host);
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    return ListenableBuilder(
      listenable: Listenable.merge([ui.profiles, ui.account]),
      builder: (context, _) {
        final profiles = ui.profiles;
        final account = ui.account.account;
        final rule = Container(
          height: 1,
          margin: const EdgeInsets.symmetric(vertical: 4),
          color: t.hairline,
        );
        return FloatSurface(
          width: profileCardWidth,
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 4),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                _who(t, profiles.current, account),
                Padding(
                  padding: const EdgeInsets.fromLTRB(10, 2, 10, 6),
                  child: account?.pro ?? false
                      ? _SyncLine(ui: ui)
                      : _ProInvite(onSee: () => _leave(showProWindow)),
                ),
                rule,
                Padding(
                  padding: const EdgeInsets.fromLTRB(10, 4, 10, 2),
                  child: Text(l10n.profileProfiles, style: t.kicker),
                ),
                for (final profile in profiles.all)
                  MenuRow(
                    key: ValueKey<String>('profile-row-${profile.id}'),
                    onPressed: () {
                      close();
                      unawaited(profiles.switchTo(profile));
                    },
                    child: Row(
                      children: [
                        ProfileAvatar(profile,
                            size: 16, pro: profile.account?.pro ?? false),
                        const SizedBox(width: 8),
                        Expanded(
                          child: Text(profileName(profile),
                              maxLines: 1, overflow: TextOverflow.ellipsis),
                        ),
                        menuTick(profile == profiles.current),
                      ],
                    ),
                  ),
                MenuRow(
                  key: const ValueKey('profile-add'),
                  onPressed: () => _leave((context) async {
                    final name = await askProfileName(context,
                        title: l10n.profileAddTitle,
                        hint: l10n.profileAddHint,
                        confirm: l10n.profileAddConfirm);
                    if (name == null) return;
                    await profiles.switchTo(profiles.add(name));
                  }),
                  child: Text(l10n.profileAdd,
                      style: t.bodyPrimary.copyWith(color: t.textSecondary)),
                ),
                rule,
                MenuRow(
                  key: const ValueKey('profile-settings'),
                  onPressed: () => _leave((context) => showSettingsWindowFrb(
                      context,
                      initialPage: SettingsPage.account)),
                  child: Text(l10n.profileAccountSettings),
                ),
                if (account == null)
                  MenuRow(
                    key: const ValueKey('profile-sign-in'),
                    onPressed: () => _leave(showSignIn),
                    child: Text(l10n.profileSignIn),
                  )
                else ...[
                  if (account.subscription != null)
                    MenuRow(
                      key: const ValueKey('profile-manage'),
                      onPressed: () {
                        close();
                        unawaited(openSubscriptionPage(ui.account));
                      },
                      child: Text(l10n.proManage),
                    ),
                  MenuRow(
                    key: const ValueKey('profile-sign-out'),
                    onPressed: () {
                      close();
                      unawaited(ui.account.signOut());
                    },
                    child: Text(l10n.profileSignOut),
                  ),
                ],
              ],
            ),
          ),
        );
      },
    );
  }

  /// Who this is: the picture, the name, and the account or the lack of one.
  Widget _who(LumitTheme t, Profile profile, CloudAccount? account) => Padding(
        padding: const EdgeInsets.fromLTRB(10, 8, 10, 8),
        child: Row(
          children: [
            ProfileAvatar(profile, size: 36, pro: account?.pro ?? false),
            const SizedBox(width: 10),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Row(
                    children: [
                      Flexible(
                        child: Text(profileName(profile),
                            style: t.bodyStrong.copyWith(
                                color: t.textPrimary, fontSize: 13),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis),
                      ),
                      if (account?.pro ?? false) ...[
                        const SizedBox(width: 6),
                        ProBadge(preview: account!.preview),
                      ],
                    ],
                  ),
                  const SizedBox(height: 2),
                  Text(account?.email ?? l10n.profileLocal,
                      style: t.small.copyWith(color: t.textMuted),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis),
                ],
              ),
            ),
          ],
        ),
      );
}

/// Open the shop's page for the account's subscription, in the browser.
Future<void> openSubscriptionPage(AccountState account) async {
  try {
    await openExternalLink(await account.portalUrl());
  } on CloudError {
    // Nothing to open. The Lumit Pro window says why when it is asked.
  }
}

/// What a person without Pro sees in the card: a quiet invitation, edged in
/// the sweep.
class _ProInvite extends StatefulWidget {
  final VoidCallback onSee;
  const _ProInvite({required this.onSee});

  @override
  State<_ProInvite> createState() => _ProInviteState();
}

class _ProInviteState extends State<_ProInvite> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final radius = BorderRadius.circular(
        math.max(t.tokens.controlRadius, t.tokens.sectionRadius));
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        key: const ValueKey('profile-pro'),
        behavior: HitTestBehavior.opaque,
        onTap: widget.onSee,
        child: AnimatedContainer(
          duration: scope.motion.hoverIn.duration,
          curve: scope.motion.hoverIn.curve,
          padding: const EdgeInsets.all(1),
          decoration: BoxDecoration(
            borderRadius: radius,
            gradient: LinearGradient(
              begin: Alignment.topLeft,
              end: Alignment.bottomRight,
              colors: [
                for (final stop in brandProSweep)
                  stop.withValues(alpha: _hover ? 0.9 : 0.45),
              ],
            ),
          ),
          child: Container(
            padding: const EdgeInsets.fromLTRB(10, 8, 10, 9),
            decoration: BoxDecoration(
              borderRadius: radius,
              color: Color.alphaBlend(
                  brandKeyJade.withValues(alpha: _hover ? 0.10 : 0.06),
                  t.surface2),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Text(l10n.proCardTitle,
                        style: t.bodyStrong.copyWith(color: t.textPrimary)),
                    const Spacer(),
                    Text(l10n.proCardAction,
                        style: t.small.copyWith(
                            color: _hover ? t.textPrimary : t.textSecondary)),
                  ],
                ),
                const SizedBox(height: 3),
                Text(l10n.proCardLine,
                    style: t.small.copyWith(color: t.textSecondary)),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// What a person with Pro sees in its place: whether their settings are in
/// step, and a ring that turns while they are being brought there.
class _SyncLine extends StatelessWidget {
  final LumitUiState ui;
  const _SyncLine({required this.ui});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final status = ui.profiles.status;
    final (words, colour) = switch (status) {
      SyncStatus.syncing => (l10n.syncStatusSyncing, t.textSecondary),
      SyncStatus.failed => (l10n.syncStatusFailed, t.warning),
      SyncStatus.off => (l10n.syncStatusOff, t.textMuted),
      SyncStatus.idle => (l10n.syncStatusSynced, t.textSecondary),
    };
    return Row(
      children: [
        SizedBox(
          width: 14,
          height: 14,
          child: status == SyncStatus.syncing
              ? const SyncRing()
              : glyph.LumitIcon(
                  status == SyncStatus.idle ? LumitIcons.tick : LumitIcons.loop,
                  size: 14,
                  colour: status == SyncStatus.idle ? t.success : colour),
        ),
        const SizedBox(width: 8),
        Expanded(
          child: Text(words,
              key: const ValueKey('profile-sync'),
              style: t.small.copyWith(color: colour)),
        ),
      ],
    );
  }
}

/// A ring that turns while something is under way. Still, and whole, when
/// animation is switched off.
class SyncRing extends StatefulWidget {
  final double stroke;
  const SyncRing({super.key, this.stroke = 1.5});

  @override
  State<SyncRing> createState() => _SyncRingState();
}

class _SyncRingState extends State<SyncRing>
    with SingleTickerProviderStateMixin {
  AnimationController? _turn;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final moving =
        ThemeScope.of(context).animationLevel != AnimationLevel.none;
    if (moving && _turn == null) {
      _turn = AnimationController(
          vsync: this, duration: const Duration(milliseconds: 900))
        ..repeat();
    } else if (!moving) {
      _turn?.dispose();
      _turn = null;
    }
  }

  @override
  void dispose() {
    _turn?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final turn = _turn;
    final ring = CustomPaint(
      painter: _RingPainter(
          t.hairlineStrong, widget.stroke, turn == null ? null : t.accent),
    );
    return turn == null ? ring : RotationTransition(turns: turn, child: ring);
  }
}

class _RingPainter extends CustomPainter {
  final Color track;
  final double stroke;

  /// The colour of the arc that turns, or null for a ring with none.
  final Color? arc;

  const _RingPainter(this.track, this.stroke, this.arc);

  @override
  void paint(Canvas canvas, Size size) {
    final box = (Offset.zero & size).deflate(stroke / 2);
    final pen = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = stroke
      ..strokeCap = StrokeCap.round
      ..color = track;
    canvas.drawOval(box, pen);
    final arc = this.arc;
    if (arc != null) {
      canvas.drawArc(box, -math.pi / 2, math.pi * 0.6, false, pen..color = arc);
    }
  }

  @override
  bool shouldRepaint(_RingPainter old) =>
      old.track != track || old.arc != arc || old.stroke != stroke;
}

/// Ask what a profile is called, in a small window headed [title]. Null when
/// it was dismissed or left empty.
Future<String?> askProfileName(
  BuildContext context, {
  required String title,
  String suggested = '',
  String? hint,
  required String confirm,
}) async {
  final controller = TextEditingController(text: suggested);
  final name = await showLumitModal<String>(
    context: context,
    builder: (close) => FloatSurface(
      width: 320,
      child: Padding(
        padding: const EdgeInsets.all(14),
        child: Builder(builder: (context) {
          final t = ThemeScope.of(context).theme;
          return Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(title, style: t.bodyPrimary),
              const SizedBox(height: 10),
              HouseTextField(
                key: const ValueKey('profile-name-field'),
                controller: controller,
                width: double.infinity,
                autofocus: true,
                hint: l10n.signInName,
                onSubmitted: close,
              ),
              if (hint != null) ...[
                const SizedBox(height: 6),
                Text(hint, style: t.small.copyWith(color: t.textMuted)),
              ],
              const SizedBox(height: 12),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  HouseButton(
                    small: true,
                    frameless: true,
                    onPressed: () => close(null),
                    child: Text(l10n.cancel),
                  ),
                  const SizedBox(width: 6),
                  HouseButton(
                    key: const ValueKey('profile-name-ok'),
                    small: true,
                    primary: true,
                    onPressed: () => close(controller.text),
                    child: Text(confirm),
                  ),
                ],
              ),
            ],
          );
        }),
      ),
    ),
  );
  controller.dispose();
  final trimmed = name?.trim();
  return (trimmed == null || trimmed.isEmpty) ? null : trimmed;
}
