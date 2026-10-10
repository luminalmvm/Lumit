// Settings ▸ Account: the profiles on this machine, the Lumit account the
// one in use is signed in to, and what of it is kept in sync.
//
// A page of its own file because it watches things the other pages do not.
// They are drawn from the settings and redrawn when a setting changes. This
// one is drawn from the account and the profiles, which change when the
// server answers, so it listens to those itself.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/account.dart';
import '../state/app_state.dart';
import '../state/profiles.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'pro_window.dart';
import 'profile_chip.dart';
import 'settings_rows.dart';
import 'sign_in_window.dart';

/// What a kind of setting is called on the page.
String profileGroupLabel(ProfileGroup group) => switch (group) {
      ProfileGroup.appearance => l10n.syncGroupAppearance,
      ProfileGroup.shortcuts => l10n.syncGroupShortcuts,
      ProfileGroup.layout => l10n.syncGroupLayout,
      ProfileGroup.preferences => l10n.syncGroupPreferences,
      ProfileGroup.library => l10n.syncGroupLibrary,
      ProfileGroup.presets => l10n.syncGroupPresets,
    };

/// The name of every row the page can show, for the command palette, which
/// asks before there is a page to read them off.
List<String> accountSettingsRows() => [
      l10n.settingsAccountProfiles,
      l10n.settingsAccountPlan,
      l10n.settingsSyncOn,
      for (final group in ProfileGroup.values) profileGroupLabel(group),
    ];

class AccountSettings extends StatelessWidget {
  final LumitUiState ui;

  /// Whether the search in force lets a row called this be shown.
  final bool Function(String title) matches;

  const AccountSettings({super.key, required this.ui, required this.matches});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ListenableBuilder(
      listenable: Listenable.merge([ui.account, ui.profiles]),
      builder: (context, _) {
        final sections = <Widget>[];
        void section(String title, List<Widget?> rows) {
          final kept = rows.whereType<Widget>().toList();
          if (kept.isEmpty) return;
          sections
              .add(settingsSection(t, title, kept, first: sections.isEmpty));
        }

        section(l10n.settingsAccountProfiles, _profiles(context, t));
        section(l10n.settingsAccountGroup, _account(context, t));
        section(l10n.settingsSync, _sync(context, t));
        return Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: sections,
        );
      },
    );
  }

  Widget? _row(LumitTheme t, String title, Widget control,
          {String description = ''}) =>
      matches(title) ? settingsRow(t, title, description, control) : null;

  Widget _button(LumitTheme t, String id, String label, VoidCallback? onPressed) =>
      HouseButton(
        key: ValueKey<String>('settings-$id'),
        small: true,
        onPressed: onPressed,
        child: Text(label, style: t.small),
      );

  List<Widget?> _profiles(BuildContext context, LumitTheme t) {
    final profiles = ui.profiles;
    if (!matches(l10n.settingsAccountProfiles)) return const [];
    return [
      for (final profile in profiles.all)
        settingsRow(
          t,
          profileName(profile),
          '',
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (profile == profiles.current)
                Text(l10n.profileInUse,
                    style: t.small.copyWith(color: t.textMuted))
              else
                _button(t, 'profile-remove-${profile.id}', l10n.profileRemove,
                    () async {
                  final sure = await _confirm(
                      context,
                      l10n.profileRemoveAsk(profileName(profile)),
                      l10n.profileRemove);
                  if (sure) await profiles.remove(profile);
                }),
              const SizedBox(width: 8),
              _button(t, 'profile-rename-${profile.id}', l10n.profileRename,
                  () async {
                final name = await askProfileName(context,
                    title: l10n.profileRenameTitle,
                    suggested: profile.name,
                    confirm: l10n.rename);
                if (name != null) profiles.rename(profile, name);
              }),
            ],
          ),
        ),
      settingsRow(
        t,
        '',
        l10n.settingsAccountProfilesHint,
        _button(t, 'profile-add', l10n.profileAdd, () async {
          final name = await askProfileName(context,
              title: l10n.profileAddTitle,
              hint: l10n.profileAddHint,
              confirm: l10n.profileAddConfirm);
          if (name != null) await profiles.switchTo(profiles.add(name));
        }),
      ),
    ];
  }

  List<Widget?> _account(BuildContext context, LumitTheme t) {
    final account = ui.account.account;
    if (account == null) {
      return [
        _row(
          t,
          l10n.profileLocal,
          _button(t, 'sign-in', l10n.profileSignIn,
              () => unawaited(showSignIn(context))),
          description: l10n.settingsAccountSignedOut,
        ),
      ];
    }
    final plan = subscriptionLine(account);
    return [
      _row(
        t,
        account.email,
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            _button(t, 'sign-out', l10n.profileSignOut,
                () => unawaited(ui.account.signOut())),
            const SizedBox(width: 8),
            _button(t, 'delete-account', l10n.settingsAccountDelete,
                () async {
              final sure = await _confirm(
                  context,
                  l10n.settingsAccountDeleteAsk,
                  l10n.settingsAccountDeleteConfirm);
              if (!sure) return;
              try {
                await ui.account.deleteAccount();
              } on CloudError catch (e) {
                // Still signed in, and told why: most likely the shop could
                // not be reached to end the subscription first.
                if (context.mounted) {
                  context
                      .read<LumitState>()
                      .postNotice(cloudErrorText(e), error: true);
                }
              }
            }),
          ],
        ),
      ),
      _row(
        t,
        l10n.settingsAccountPlan,
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (account.pro)
              ProBadge(preview: account.preview)
            else
              Text(l10n.settingsAccountPlanFree,
                  style: t.small.copyWith(color: t.textSecondary)),
            const SizedBox(width: 10),
            account.subscription != null
                ? _button(t, 'manage', l10n.proManage,
                    () => unawaited(openSubscriptionPage(ui.account)))
                : _button(t, 'pro', l10n.shareCloudSee,
                    () => unawaited(showProWindow(context))),
          ],
        ),
        description: plan?.$1 ?? '',
      ),
    ];
  }

  List<Widget?> _sync(BuildContext context, LumitTheme t) {
    final profiles = ui.profiles;
    final pro = ui.account.pro;
    final current = profiles.current;
    Widget toggle(String id, bool value, ValueChanged<bool> set) => IgnorePointer(
          ignoring: !pro,
          child: Opacity(
            opacity: pro ? 1 : 0.4,
            child: HouseToggle(
              key: ValueKey<String>('settings-sync-$id'),
              value: value,
              onChanged: set,
            ),
          ),
        );
    return [
      _row(
        t,
        l10n.settingsSyncOn,
        toggle('on', current.sync, profiles.setSync),
        description: pro ? l10n.settingsSyncHint : l10n.settingsSyncNeedsPro,
      ),
      if (current.sync)
        for (final group in ProfileGroup.values)
          _row(
            t,
            profileGroupLabel(group),
            toggle(group.name, !current.unsynced.contains(group),
                (on) => profiles.setGroupSync(group, on)),
          ),
      if (profiles.syncing)
        _row(
          t,
          l10n.settingsSyncNow,
          _button(
              t,
              'sync-now',
              l10n.settingsSyncNow,
              profiles.status == SyncStatus.syncing
                  ? null
                  : () => unawaited(profiles.syncNow())),
          description: switch (profiles.status) {
            SyncStatus.syncing => l10n.syncStatusSyncing,
            SyncStatus.failed => l10n.syncStatusFailed,
            SyncStatus.idle => l10n.syncStatusSynced,
            SyncStatus.off => '',
          },
        ),
    ];
  }
}

/// Ask before something that can't be taken back. True for yes.
Future<bool> _confirm(BuildContext context, String question, String yes) async {
  final sure = await showLumitModal<bool>(
    context: context,
    builder: (close) => FloatSurface(
      width: 360,
      child: Padding(
        padding: const EdgeInsets.all(14),
        child: Builder(builder: (context) {
          final t = ThemeScope.of(context).theme;
          return Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(question, style: t.bodyPrimary.copyWith(height: 1.4)),
              const SizedBox(height: 14),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  HouseButton(
                    key: const ValueKey('confirm-no'),
                    small: true,
                    primary: true,
                    autofocus: true,
                    onPressed: () => close(false),
                    child: Text(l10n.cancel),
                  ),
                  const SizedBox(width: 6),
                  HouseButton(
                    key: const ValueKey('confirm-yes'),
                    small: true,
                    onPressed: () => close(true),
                    child: Text(yes),
                  ),
                ],
              ),
            ],
          );
        }),
      ),
    ),
  );
  return sure ?? false;
}
