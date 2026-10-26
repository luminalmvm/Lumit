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

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/account.dart';
import '../state/app_state.dart';
import '../state/phone_line_frb.dart' show AccountDevice;
import '../state/profiles.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'add_phone_window.dart';
import 'pro_window.dart';
import 'profile_chip.dart';
import 'project_history_window.dart' show backupSize, backupTime;
import 'settings_rows.dart';
import 'share_dialog_frb.dart' show askStopKeeping;
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
      l10n.settingsAccountName,
      l10n.settingsAccountUsername,
      l10n.settingsAccountPlan,
      l10n.settingsFriends,
      l10n.settingsStorage,
      l10n.settingsPhones,
      l10n.phonesExport,
      l10n.settingsSyncOn,
      for (final group in ProfileGroup.values) profileGroupLabel(group),
      l10n.backupSettingsOn,
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
        // Free for everyone, so above everything Pro holds.
        if (ui.account.signedIn && matches(l10n.settingsFriends)) {
          sections.add(_Friends(
              key: ValueKey(ui.account.account?.id),
              ui: ui,
              first: sections.isEmpty));
        }
        // Keyed by the account, so another one starts from its own list.
        if (ui.account.pro &&
            (matches(l10n.settingsPhones) || matches(l10n.phonesExport))) {
          sections.add(_Phones(
              key: ValueKey(ui.account.account?.id),
              ui: ui,
              first: sections.isEmpty));
        }
        section(l10n.settingsSync, _sync(context, t));
        section(l10n.backupSettingsGroup, [
          if (matches(l10n.backupSettingsOn)) _BackupRow(ui: ui),
        ]);
        // Under what the account's storage holds, what it keeps online.
        if (matches(l10n.settingsOnlineGroup)) {
          sections.add(_KeptOnline(ui: ui, first: sections.isEmpty));
        }
        return Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: sections,
        );
      },
    );
  }

  /// A row, or nothing when the search hides it. Its [actions] line up at
  /// the page's right, whatever [control] beside them says.
  Widget? _row(LumitTheme t, String title, Widget control,
          {String description = '', List<Widget> actions = const []}) =>
      matches(title)
          ? settingsRow(t, title, description, control, actions: actions)
          : null;

  /// What a row says of itself, beside its buttons.
  Widget _value(LumitTheme t, String words, {bool muted = false}) => Text(
      words,
      style: t.small.copyWith(color: muted ? t.textMuted : t.textSecondary));

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
          profile == profiles.current
              ? _value(t, l10n.profileInUse, muted: true)
              : const SizedBox.shrink(),
          actions: [
            if (profile != profiles.current)
              _button(t, 'profile-remove-${profile.id}', l10n.profileRemove,
                  () async {
                final sure = await askBefore(
                    context,
                    l10n.profileRemoveAsk(profileName(profile)),
                    l10n.profileRemove);
                if (sure) await profiles.remove(profile);
              }),
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
      settingsRow(
        t,
        '',
        '',
        const SizedBox.shrink(),
        actions: [
          _button(t, 'profile-add', l10n.profileAdd, () async {
            final name = await askProfileName(context,
                title: l10n.profileAddTitle,
                confirm: l10n.profileAddConfirm);
            if (name != null) await profiles.switchTo(profiles.add(name));
          }),
        ],
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
          const SizedBox.shrink(),
          actions: [
            _button(t, 'sign-in', l10n.profileSignIn,
                () => unawaited(showSignIn(context))),
          ],
        ),
      ];
    }
    final plan = subscriptionLine(account);
    return [
      _row(
        t,
        account.email,
        const SizedBox.shrink(),
        actions: [
          _button(t, 'sign-out', l10n.profileSignOut,
              () => unawaited(ui.account.signOut())),
          _button(t, 'delete-account', l10n.settingsAccountDelete, () async {
            final sure = await askBefore(context, l10n.settingsAccountDeleteAsk,
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
      // What other people see on previews. Signing in with Apple, Google or
      // Discord fills it with the name they hold, often a real one.
      _row(
        t,
        l10n.settingsAccountName,
        _value(t, account.name),
        actions: [
          _button(t, 'account-rename', l10n.deviceRename,
              () => unawaited(_rename(context, account.name))),
        ],
      ),
      // What friends find the account by. Unique, and chosen once there is
      // a reason to: a friend to add or a preview to send.
      _row(
        t,
        l10n.settingsAccountUsername,
        account.username == null
            ? _value(t, l10n.settingsAccountUsernameNone, muted: true)
            : _value(t, '@${account.username}'),
        actions: [
          _button(
              t,
              'account-username',
              account.username == null
                  ? l10n.accountUsernameChoose
                  : l10n.accountUsernameChange,
              () => unawaited(_setUsername(context, account.username))),
        ],
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
              _value(t, l10n.settingsAccountPlanFree),
            // Bought in the iPhone app, it is changed there, and with Pro
            // already on there is nothing here to buy.
            if (account.onIphone && account.pro) ...[
              const SizedBox(width: 10),
              _value(t, l10n.proManagedOnIphone),
            ],
          ],
        ),
        actions: [
          if (account.subscription != null && !account.onIphone)
            _button(t, 'manage', l10n.proManage,
                () => unawaited(openSubscriptionPage(ui.account)))
          else if (!(account.onIphone && account.pro))
            _button(t, 'pro', l10n.shareCloudSee,
                () => unawaited(showProWindow(context))),
        ],
        description: plan?.$1 ?? '',
      ),
      // The account's, so it is said with the account and not under one of
      // the two things that fill it.
      if (account.pro && matches(l10n.settingsStorage)) _StorageRow(ui: ui),
      // Mail about new comments on review links, for every link the account
      // has. A link has a switch of its own in the Previews window.
      _row(
        t,
        l10n.settingsReviewMail,
        IgnorePointer(
          ignoring: !account.pro,
          child: Opacity(
            opacity: account.pro ? 1 : 0.4,
            child: HouseToggle(
              key: const ValueKey('settings-review-mail'),
              value: account.reviewMail,
              onChanged: (on) => unawaited(_setReviewMail(context, on)),
            ),
          ),
        ),
        description: account.pro ? '' : l10n.reviewNeedsPro,
      ),
    ];
  }

  Future<void> _rename(BuildContext context, String now) async {
    final name = await askProfileName(context,
        title: l10n.accountRenameTitle, suggested: now, confirm: l10n.rename);
    if (name == null || name == now) return;
    try {
      await ui.account.rename(name);
    } on CloudError catch (e) {
      // An empty name never gets this far, so a bad one is a long one.
      if (context.mounted) {
        context.read<LumitState>().postNotice(
            e.code == 'bad_name' ? l10n.deviceNameLong : cloudErrorText(e),
            error: true);
      }
    }
  }

  Future<void> _setUsername(BuildContext context, String? now) async {
    final name = await askProfileName(context,
        title: l10n.accountUsernameTitle,
        suggested: now ?? '',
        confirm: l10n.save,
        hint: l10n.accountUsernameHint);
    if (name == null || name == now) return;
    try {
      await ui.account.setUsername(name);
    } on CloudError catch (e) {
      if (context.mounted) {
        context.read<LumitState>().postNotice(cloudErrorText(e), error: true);
      }
    }
  }

  Future<void> _setReviewMail(BuildContext context, bool on) async {
    try {
      await ui.account.setReviewMail(on);
    } on CloudError catch (e) {
      // The switch stays where the server has it, and says why.
      if (context.mounted) {
        context.read<LumitState>().postNotice(cloudErrorText(e), error: true);
      }
    }
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
        description: pro ? '' : l10n.settingsSyncNeedsPro,
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
          const SizedBox.shrink(),
          actions: [
            _button(
                t,
                'sync-now',
                l10n.settingsSyncNow,
                profiles.status == SyncStatus.syncing
                    ? null
                    : () => unawaited(profiles.syncNow())),
          ],
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

/// How much of the account's storage is used: what backups and the footage
/// of projects kept online both fill. It asks the server as it is shown,
/// and is drawn again when the answer comes.
class _StorageRow extends StatefulWidget {
  final LumitUiState ui;
  const _StorageRow({required this.ui});

  @override
  State<_StorageRow> createState() => _StorageRowState();
}

class _StorageRowState extends State<_StorageRow> {
  @override
  void initState() {
    super.initState();
    unawaited(widget.ui.backup.refresh());
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final backup = widget.ui.backup;
    return ListenableBuilder(
      listenable: backup,
      builder: (context, _) => settingsRow(
        t,
        l10n.settingsStorage,
        '',
        Text(
            backup.limit > 0
                ? l10n.backupStorageIn(backupSize(backup.used),
                    backupSize(backup.limit), backupSize(backup.stored))
                : '',
            key: const ValueKey('settings-storage'),
            style: t.small.copyWith(color: t.textSecondary)),
      ),
    );
  }
}

/// The switch for backing projects up when they are saved.
class _BackupRow extends StatelessWidget {
  final LumitUiState ui;
  const _BackupRow({required this.ui});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final backup = ui.backup;
    return ListenableBuilder(
      listenable: backup,
      builder: (context, _) {
        final pro = ui.account.pro;
        return settingsRow(
          t,
          l10n.backupSettingsOn,
          pro ? '' : l10n.backupNeedsPro,
          IgnorePointer(
            ignoring: !pro,
            child: Opacity(
              opacity: pro ? 1 : 0.4,
              child: HouseToggle(
                key: const ValueKey('settings-backup-on'),
                value: backup.on,
                onChanged: backup.setOn,
              ),
            ),
          ),
        );
      },
    );
  }
}

/// The shared projects the account keeps on Lumit's server, each with the
/// way to stop keeping it. A project whose host closed it without saving,
/// or lost its file, is still there, and this is how it is reached. It asks
/// the server as it is shown, and draws nothing while there are none.
class _KeptOnline extends StatefulWidget {
  final LumitUiState ui;

  /// Whether this is the page's first section, which has no rule above it.
  final bool first;
  const _KeptOnline({required this.ui, required this.first});

  @override
  State<_KeptOnline> createState() => _KeptOnlineState();
}

class _KeptOnlineState extends State<_KeptOnline> {
  List<OnlineKept> _kept = const [];

  /// Whose list is held, and which project is being stopped just now.
  String? _who, _stopping;

  @override
  void initState() {
    super.initState();
    widget.ui.account.addListener(_follow);
    _follow();
  }

  @override
  void dispose() {
    widget.ui.account.removeListener(_follow);
    super.dispose();
  }

  /// Another account, or none: what was listed belonged to the last one.
  void _follow() {
    final who = widget.ui.account.account?.id;
    if (who == _who) return;
    _who = who;
    if (_kept.isNotEmpty) setState(() => _kept = const []);
    if (who != null) unawaited(_read());
  }

  Future<void> _read() async {
    final who = _who;
    try {
      final kept = await widget.ui.keptOnline();
      if (mounted && who == _who) setState(() => _kept = kept);
    } on CloudError {
      // Out of reach. Nothing is listed, and nothing is said of it here.
    }
  }

  /// When it was last opened, the way a list says a day: today and
  /// yesterday by name with the time, and anything earlier by its date.
  String _opened(DateTime at) {
    final now = DateTime.now();
    bool same(DateTime day) =>
        at.year == day.year && at.month == day.month && at.day == day.day;
    if (same(now)) return l10n.settingsOnlineToday(backupTime(at));
    if (same(now.subtract(const Duration(days: 1)))) {
      return l10n.settingsOnlineYesterday(backupTime(at));
    }
    return writtenDay(at);
  }

  /// The one question the Shared project window asks too. A project with
  /// no file known here may have no copy on this computer.
  Future<void> _stop(OnlineKept kept) async {
    final sure = await askStopKeeping(context,
        name: kept.name, here: kept.name != null);
    if (!sure || !mounted) return;
    setState(() => _stopping = kept.id);
    try {
      await widget.ui.stopKeeping(kept.id);
    } on CloudError catch (e) {
      if (mounted) {
        context.read<LumitState>().postNotice(cloudErrorText(e), error: true);
      }
    }
    if (!mounted) return;
    setState(() => _stopping = null);
    await _read();
  }

  @override
  Widget build(BuildContext context) {
    if (_kept.isEmpty) return const SizedBox.shrink();
    final t = ThemeScope.of(context).theme;
    return settingsSection(
      t,
      l10n.settingsOnlineGroup,
      [
        // What the project's own window and the strip say, in a row's
        // form and in words that hold on this page: nothing waits here.
        if (!widget.ui.account.pro)
          settingsRow(
            t,
            l10n.settingsOnlineEnded,
            l10n.settingsOnlineEndedHint,
            const SizedBox.shrink(),
            actions: [
              HouseButton(
                key: const ValueKey('settings-online-pro'),
                small: true,
                onPressed: () => unawaited(showProWindow(context)),
                child: Text(l10n.shareCloudSee, style: t.small),
              ),
            ],
          ),
        for (final kept in _kept)
          _keptRow(
            t,
            kept,
            HouseButton(
              key: ValueKey<String>('settings-online-stop-${kept.id}'),
              small: true,
              onPressed: _stopping == null ? () => _stop(kept) : null,
              child: Text(l10n.settingsOnlineStop, style: t.small),
            ),
          ),
      ],
      first: widget.first,
    );
  }

  /// One project: its file's name, or that it has none in a quieter colour
  /// with the day it was put online under it.
  Widget _keptRow(LumitTheme t, OnlineKept kept, Widget control) {
    final size = backupSize(kept.bytes);
    return switch (kept.name) {
      final name? => settingsRow(t, name,
          l10n.settingsOnlineHolds(size, _opened(kept.opened)),
          const SizedBox.shrink(),
          actions: [control]),
      // settingsRow draws every title in one colour, and this one is not
      // a name, so it is its own and quieter.
      null => Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          mainAxisSize: MainAxisSize.min,
          children: [
            ConstrainedBox(
              constraints:
                  const BoxConstraints(minHeight: settingsRowHeight),
              child: Padding(
                padding: const EdgeInsets.symmetric(
                    horizontal: settingsRowPadding),
                child: Row(
                  children: [
                    SizedBox(
                      width: settingsLabelColumn,
                      child: Text(l10n.settingsOnlineUnnamed,
                          style: t.body.copyWith(color: t.textMuted)),
                    ),
                    const Spacer(),
                    control,
                  ],
                ),
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(
                  settingsRowPadding, 0, settingsRowPadding, 6),
              child: Text(
                  l10n.settingsOnlinePut(size, writtenDay(kept.created)),
                  style: t.small.copyWith(color: t.textMuted)),
            ),
          ],
        ),
    };
  }
}

/// The account's friends: the way to add one by username, whoever asked
/// and whoever was asked, waiting for an answer, then the friends, folded
/// away until opened. Free for everyone. It asks the server as it is shown.
class _Friends extends StatefulWidget {
  final LumitUiState ui;

  /// Whether this is the page's first section, which has no rule above it.
  final bool first;
  const _Friends({super.key, required this.ui, required this.first});

  @override
  State<_Friends> createState() => _FriendsState();
}

class _FriendsState extends State<_Friends> {
  final _name = TextEditingController();

  /// Whether an answer is awaited, so a second press waits for it.
  bool _busy = false;

  /// Whether the friends are unfolded. Shut each time the page is drawn.
  bool _open = false;

  AccountState get _account => widget.ui.account;

  @override
  void initState() {
    super.initState();
    unawaited(_account.loadFriends());
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  /// [call], saying as a notice what went wrong.
  Future<void> _run(Future<void> Function() call) async {
    setState(() => _busy = true);
    try {
      await call();
    } on CloudError catch (e) {
      if (mounted) {
        context.read<LumitState>().postNotice(
            switch (e.code) {
              'not_found' => l10n.friendsNotFound,
              'bad_request' => l10n.friendsNotYourself,
              _ => cloudErrorText(e),
            },
            error: true);
      }
    }
    if (mounted) setState(() => _busy = false);
  }

  void _add() {
    final name = _name.text.trim();
    if (name.isEmpty) return;
    unawaited(_run(() async {
      await _account.addFriend(name);
      _name.clear();
    }));
  }

  Future<void> _remove(Friend friend) async {
    final sure = await askBefore(
        context, l10n.friendsRemoveAsk(friend.name), l10n.friendsRemove);
    if (sure && mounted) {
      await _run(() => _account.removeFriend(friend.username));
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final friends = _account.friends;
    final named = _account.account?.username != null;
    Widget button(String id, String label, VoidCallback onPressed) =>
        HouseButton(
          key: ValueKey<String>('settings-friend-$id'),
          small: true,
          onPressed: _busy ? null : onPressed,
          child: Text(label, style: t.small),
        );
    Widget quiet(String words) =>
        Text(words, style: t.small.copyWith(color: t.textMuted));
    return settingsSection(
      t,
      l10n.settingsFriends,
      [
        settingsRow(
          t,
          l10n.friendsAdd,
          // Friends find each other by username, so one comes first.
          named ? '' : l10n.friendsNeedUsername,
          SizedBox(
            width: 160,
            height: settingsControlHeight,
            child: HouseTextField(
              key: const ValueKey('settings-friend-name'),
              controller: _name,
              width: double.infinity,
              padding: const EdgeInsets.symmetric(horizontal: 6),
              hint: l10n.accountUsernameHint,
              onSubmitted: (_) {
                if (named) _add();
              },
            ),
          ),
          actions: [
            HouseButton(
              key: const ValueKey('settings-friend-add'),
              small: true,
              onPressed: _busy || !named ? null : _add,
              child: Text(l10n.friendsAddButton, style: t.small),
            ),
          ],
        ),
        // Asks wait for an answer, so they stay in sight. So does one this
        // account sent, which is how an Add shows it went.
        for (final asked in friends.asked)
          settingsRow(t, asked.name, '', quiet('@${asked.username}'),
              actions: [
                button(
                    'accept-${asked.username}',
                    l10n.friendsAccept,
                    () => unawaited(
                        _run(() => _account.addFriend(asked.username)))),
                button(
                    'decline-${asked.username}',
                    l10n.friendsDecline,
                    () => unawaited(
                        _run(() => _account.removeFriend(asked.username)))),
              ]),
        for (final asking in friends.asking)
          settingsRow(
              t,
              asking.name,
              '',
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  quiet('@${asking.username}'),
                  const SizedBox(width: 10),
                  quiet(l10n.friendsAsked),
                ],
              ),
              actions: [
                button(
                    'cancel-${asking.username}',
                    l10n.friendsTakeBack,
                    () => unawaited(
                        _run(() => _account.removeFriend(asking.username)))),
              ]),
        // The friends themselves change least and need nothing doing, so
        // they are folded away under their count, two to a line when open.
        if (friends.friends.isNotEmpty) ...[
          _fold(t, friends.friends.length),
          OpenReveal(open: _open, child: _grid(t, friends.friends)),
        ],
      ],
      first: widget.first,
    );
  }

  /// The bar the friends are folded under: how many, and the twirl.
  Widget _fold(LumitTheme t, int count) => Padding(
        // The button's own inset and edge taken back, so its words start
        // where every row's title does.
        padding: const EdgeInsets.symmetric(horizontal: settingsRowPadding - 6),
        child: SizedBox(
          height: settingsRowHeight,
          child: Align(
            alignment: Alignment.centerLeft,
            child: HouseButton(
              key: const ValueKey('settings-friends-fold'),
              frameless: true,
              small: true,
              onPressed: () => setState(() => _open = !_open),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(l10n.friendsCount(count), style: t.body),
                  const SizedBox(width: 4),
                  TwirlTurn(
                    open: _open,
                    child: lumitIcon(
                        _open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                        size: 12,
                        color: t.textMuted),
                  ),
                ],
              ),
            ),
          ),
        ),
      );

  /// [friends] two to a line, each with the way to stop being friends.
  Widget _grid(LumitTheme t, List<Friend> friends) {
    Widget one(Friend friend) => SizedBox(
          key: ValueKey<String>('settings-friend-${friend.username}'),
          height: settingsRowHeight,
          child: Row(
            children: [
              Expanded(
                child: Row(
                  children: [
                    Flexible(
                      child: Text(friend.name,
                          style: t.body,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis),
                    ),
                    const SizedBox(width: 8),
                    Flexible(
                      child: Text('@${friend.username}',
                          style: t.small.copyWith(color: t.textMuted),
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis),
                    ),
                  ],
                ),
              ),
              LumitTooltip(
                message: l10n.friendsRemove,
                child: HouseButton(
                  key: ValueKey<String>('settings-friend-remove-'
                      '${friend.username}'),
                  frameless: true,
                  small: true,
                  padding: const EdgeInsets.all(4),
                  onPressed: _busy ? null : () => unawaited(_remove(friend)),
                  child: glyph.LumitIcon(LumitIcons.close,
                      size: 10,
                      colour: t.textMuted,
                      semanticLabel: l10n.friendsRemove),
                ),
              ),
            ],
          ),
        );
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: settingsRowPadding),
      child: Column(
        children: [
          for (var i = 0; i < friends.length; i += 2)
            Row(
              children: [
                Expanded(child: one(friends[i])),
                const SizedBox(width: 24),
                Expanded(
                    child: i + 1 < friends.length
                        ? one(friends[i + 1])
                        : const SizedBox.shrink()),
              ],
            ),
        ],
      ),
    );
  }
}

/// This computer and the phones paired with the account, each with the way
/// to rename it, the phones with the way to remove one, and the way to add
/// one. It asks the server as it is shown.
class _Phones extends StatefulWidget {
  final LumitUiState ui;

  /// Whether this is the page's first section, which has no rule above it.
  final bool first;
  const _Phones({super.key, required this.ui, required this.first});

  @override
  State<_Phones> createState() => _PhonesState();
}

class _PhonesState extends State<_Phones> {
  List<AccountDevice> _devices = const [];

  @override
  void initState() {
    super.initState();
    unawaited(_read());
  }

  Future<void> _read() async {
    try {
      final devices = await widget.ui.phones.devices();
      if (mounted) setState(() => _devices = devices);
    } on CloudError {
      // Out of reach. Nothing is listed, and nothing is said of it here.
    }
  }

  Future<void> _rename(AccountDevice device) async {
    final name = await askProfileName(context,
        title: l10n.deviceRenameTitle,
        suggested: device.name,
        confirm: l10n.rename);
    if (name == null || !mounted) return;
    await _run(() => widget.ui.phones.rename(device.id, name));
  }

  /// [call], then the list read again. What went wrong is said as a notice.
  Future<void> _run(Future<void> Function() call) async {
    try {
      await call();
    } on CloudError catch (e) {
      if (mounted) {
        // An empty name never gets this far, so a bad one is a long one.
        context.read<LumitState>().postNotice(
            e.code == 'bad_name' ? l10n.deviceNameLong : cloudErrorText(e),
            error: true);
      }
    }
    await _read();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    Widget button(String id, String label, VoidCallback onPressed) =>
        HouseButton(
          key: ValueKey<String>('settings-phone-$id'),
          small: true,
          onPressed: onPressed,
          child: Text(label, style: t.small),
        );
    return settingsSection(
      t,
      l10n.settingsPhones,
      [
        for (final me in _devices.where((d) => d.me))
          settingsRow(t, me.name, l10n.deviceThisComputer,
              const SizedBox.shrink(),
              actions: [
                button('rename-${me.id}', l10n.deviceRename,
                    () => unawaited(_rename(me))),
              ]),
        for (final phone in _devices.where((d) => d.phone))
          settingsRow(
            t,
            phone.name,
            '',
            const SizedBox.shrink(),
            actions: [
              button('remove-${phone.id}', l10n.phoneRemove,
                  () => unawaited(
                      _run(() => widget.ui.phones.forget(phone.id)))),
              button('rename-${phone.id}', l10n.deviceRename,
                  () => unawaited(_rename(phone))),
            ],
          ),
        settingsRow(
          t,
          '',
          '',
          const SizedBox.shrink(),
          actions: [
            button('add', l10n.phoneAdd, () async {
              await showAddPhone(context);
              await _read();
            }),
          ],
        ),
        // This machine's own answer, which the line sends with its state.
        settingsRow(
          t,
          l10n.phonesExport,
          '',
          HouseToggle(
            key: const ValueKey('settings-phones-export'),
            value: widget.ui.workspace.phonesExport,
            onChanged: (on) =>
                setState(() => widget.ui.workspace.setPhonesExport(on)),
          ),
        ),
      ],
      first: widget.first,
    );
  }
}

/// Ask before something that can't be taken back. True for yes. [note] is
/// one quiet line under the question.
Future<bool> askBefore(BuildContext context, String question, String yes,
    {String? note}) async {
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
              if (note != null) ...[
                const SizedBox(height: 6),
                Text(note, style: t.small),
              ],
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
