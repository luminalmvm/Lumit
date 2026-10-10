// Profiles: one person's way of working, kept apart from another's.
//
// A profile is a name and a set of settings: how Lumit looks, its shortcuts,
// its panel layout, its preferences, and the easings, expressions and effect
// presets a person has saved. Several can live on one machine, and switching
// puts one away and takes another out. None of that needs an account.
//
// A profile signed in to an account with Lumit Pro is also kept on Lumit's
// server, so signing in on another machine brings it along, and a change on
// one shows up on the others.
//
// What a profile never holds is anything about the machine: where the cache
// is and how big, which audio device plays, recent projects and where their
// windows were. Those stay put whoever is using it.
//
// Settings are handled a group at a time ([ProfileGroup]). A group is plain
// JSON, read out of the live settings and written back into them, which is
// all switching and syncing either need.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show presetsDirPath;
import 'package:uuid/uuid.dart';

import 'account.dart';
import 'custom_easings.dart';
import 'secret_store.dart';
import 'workspace.dart';

/// The groups a profile's settings fall into. Each can be left out of sync on
/// its own, and the names are what the server files them under.
enum ProfileGroup {
  appearance,
  shortcuts,
  layout,
  preferences,
  library,
  presets;

  /// The keys of the settings file this group owns.
  List<String> get keys => switch (this) {
        appearance => const [
            'color_scheme',
            'theme_shape',
            'accent_override',
            'animation_level',
            'custom_themes',
            'custom_theme',
            'themed_scopes',
            'themed_effect_graphs',
            'themed_viewer_surround',
            'curve_plot_size',
            'smooth_zoomed_viewer',
          ],
        shortcuts => const ['keymap'],
        layout => const ['dock'],
        preferences => const [
            'interface',
            'show_welcome_on_launch',
            'auto_update',
            'autosave_minutes',
            'autosave_keep',
            'precompose_move_attributes',
            'precompose_adjust_duration',
            'precompose_open_new_comp',
            'timeline_columns',
            'default_expression_language',
            'share_name',
            'share_router',
            'share_relay',
            'share_cloud',
            'share_give',
            'share_take',
          ],
        library => const [
            'favourite_effects',
            'saved_expressions',
            'saved_expression_languages',
            'recent_colours',
            'palette_recents',
          ],
        presets => const [],
      };
}

/// The interface scale belongs to the screen it was chosen on, so it is the
/// one preference that stays behind.
const String _scaleKey = 'ui_scale';

/// The most a group may come to before it is left out of a sync. The server
/// takes a mebibyte, and this leaves room for the envelope.
const int _groupLimit = 900 * 1024;

/// One profile on this machine.
class Profile {
  final String id;
  String name;

  /// An index into the theme's people colours, for the disc its initial
  /// sits on.
  int colour;

  /// The account it was last signed in to, to show before the server has
  /// been asked again.
  CloudAccount? account;

  /// Whether it syncs at all, and the groups that are left out when it does.
  bool sync;
  final Set<ProfileGroup> unsynced;

  Profile({
    required this.id,
    required this.name,
    this.colour = 1,
    this.account,
    this.sync = true,
    Set<ProfileGroup>? unsynced,
  }) : unsynced = unsynced ?? {};

  /// The secret store's name for this profile's sign-in.
  String get slot => 'sign-in/$id';

  Map<String, dynamic> toJson() => {
        'id': id,
        'name': name,
        'colour': colour,
        if (account != null) 'account': account!.toJson(),
        'sync': sync,
        'unsynced': [for (final g in unsynced) g.name],
      };

  static Profile? from(Object? j) {
    if (j is! Map || j['id'] is! String || j['name'] is! String) return null;
    final who = j['account'];
    final groups = ProfileGroup.values.asNameMap();
    return Profile(
      id: j['id'] as String,
      name: j['name'] as String,
      colour: j['colour'] is int ? j['colour'] as int : 1,
      account:
          who is Map<String, dynamic> ? CloudAccount.from(who) : null,
      sync: j['sync'] != false,
      unsynced: {
        if (j['unsynced'] case final List<dynamic> off)
          for (final name in off)
            if (groups[name] case final group?) group,
      },
    );
  }
}

/// Where a sync stands, for the strip to say.
enum SyncStatus { off, idle, syncing, failed }

/// Three-way merge of two edits to the same JSON. Where both sides are maps
/// it goes key by key, so a change to one setting here and another there
/// both stand. Where only one side changed a value that side's is kept, and
/// where both did, this machine's is: the person is looking at it.
@visibleForTesting
Object? mergeSettings(Object? base, Object? local, Object? remote) {
  if (local is Map && remote is Map) {
    final was = base is Map ? base : const <dynamic, dynamic>{};
    final merged = <String, dynamic>{};
    for (final key in {...local.keys, ...remote.keys}) {
      final value = mergeSettings(was[key], local[key], remote[key]);
      final gone = !local.containsKey(key) || !remote.containsKey(key);
      // A key one side removed stays removed, unless the other changed it.
      if (value == null && gone) continue;
      merged['$key'] = value;
    }
    return merged;
  }
  return _same(local, base) ? remote : local;
}

bool _same(Object? a, Object? b) => jsonEncode(a) == jsonEncode(b);

String _hash(Object? data) =>
    sha256.convert(utf8.encode(jsonEncode(data))).toString();

/// The profiles on this machine, which one is in use, and its sync.
class ProfilesState extends ChangeNotifier {
  final Workspace workspace;
  final AccountState account;

  /// Called once settings have been written into [workspace] from a profile
  /// or from the server, for whoever has to hand them on: the keymap to the
  /// engine, the autosave interval, the limits on sharing.
  final VoidCallback afterApply;

  ProfilesState(this.workspace, this.account, {required this.afterApply}) {
    _load();
  }

  final List<Profile> all = [];
  late Profile current;

  SyncStatus status = SyncStatus.off;
  DateTime? lastSynced;

  Timer? _settle, _beat;
  bool _started = false, _applying = false, _busy = false, _again = false;
  bool _gone = false;
  String? _wasSignedInTo;

  static const JsonEncoder _pretty = JsonEncoder.withIndent('  ');

  Directory get _dir => Workspace.storeFile().parent;
  File get _index => File('${_dir.path}${Platform.pathSeparator}profiles.json');
  File _file(String id, String what) => File('${_dir.path}'
      '${Platform.pathSeparator}profiles${Platform.pathSeparator}$id.$what.json');

  // --- The list ------------------------------------------------------------

  void _load() {
    Object? inUse;
    try {
      final j = jsonDecode(_index.readAsStringSync());
      if (j is Map) {
        inUse = j['current'];
        for (final entry in j['profiles'] as List<dynamic>? ?? const []) {
          final profile = Profile.from(entry);
          if (profile != null) all.add(profile);
        }
      }
    } catch (_) {
      // No file yet, or one that will not read: start with one profile.
    }
    if (all.isEmpty) {
      // The settings this machine already has become the first profile,
      // named as the person is named to others in a shared project.
      all.add(Profile(
          id: const Uuid().v4(), name: workspace.shareName?.trim() ?? ''));
    }
    current = all.firstWhere((p) => p.id == inUse, orElse: () => all.first);
  }

  void _save() {
    try {
      _index.parent.createSync(recursive: true);
      _index.writeAsStringSync(_pretty.convert({
        'current': current.id,
        'profiles': [for (final p in all) p.toJson()],
      }));
    } catch (_) {
      // A disk that will not take it loses the list, not the settings.
    }
  }

  /// Begin talking to the server: pick up the current profile's sign-in,
  /// and keep it in step from then on. Not called by a test, which is what
  /// keeps a test run off the network.
  Future<void> start() async {
    if (_started) return;
    _started = true;
    account.addListener(_accountChanged);
    workspace.addListener(_settingsChanged);
    // Changes made elsewhere are looked for now and then. Ones made here
    // go as they are made.
    _beat = Timer.periodic(const Duration(minutes: 2), (_) => syncNow());
    unawaited(account.loadConfig());
    await account.use(current.slot, remembered: current.account);
    unawaited(syncNow());
  }

  void _accountChanged() {
    final now = account.account;
    current.account = now;
    // A profile nobody named takes the name its account goes by.
    if (now != null && current.name.trim().isEmpty) current.name = now.name;
    _save();
    if (now?.id != _wasSignedInTo) {
      _wasSignedInTo = now?.id;
      if (now != null) unawaited(syncNow());
    }
    _noteStatus();
    notifyListeners();
  }

  /// A new profile that starts as a copy of the one in use.
  // ponytail: no way to start one from the shipped defaults. Add it if
  // people ask for a clean slate.
  Profile add(String name) {
    final taken = {for (final p in all) p.colour};
    var colour = 1;
    while (taken.contains(colour) && colour < 8) {
      colour++;
    }
    final made = Profile(
        id: const Uuid().v4(), name: name.trim(), colour: colour, sync: true);
    all.add(made);
    _write(_file(made.id, 'settings'), _export());
    _save();
    notifyListeners();
    return made;
  }

  void rename(Profile profile, String name) {
    profile.name = name.trim();
    _save();
    notifyListeners();
  }

  /// Take a profile off this machine. The one in use stays. An account it
  /// was signed in to is not touched, only forgotten here.
  Future<void> remove(Profile profile) async {
    if (profile == current || !all.remove(profile)) return;
    await deleteSecret(profile.slot);
    for (final what in const ['settings', 'synced']) {
      try {
        _file(profile.id, what).deleteSync();
      } catch (_) {
        // Never written.
      }
    }
    _save();
    notifyListeners();
  }

  /// Put the current profile away and take [next] out.
  Future<void> switchTo(Profile next) async {
    if (next == current || !all.contains(next)) return;
    _write(_file(current.id, 'settings'), _export());
    current = next;
    _save();
    final kept = _read(_file(next.id, 'settings'));
    if (kept != null) _apply(kept, ProfileGroup.values);
    status = SyncStatus.off;
    lastSynced = null;
    notifyListeners();
    if (_started) {
      await account.use(next.slot, remembered: next.account);
      unawaited(syncNow());
    }
  }

  // --- Reading settings out and writing them back --------------------------

  /// Every group as it stands now.
  Map<String, dynamic> _export() =>
      {for (final group in ProfileGroup.values) group.name: _read1(group)};

  Map<String, dynamic> _read1(ProfileGroup group) {
    final live = workspace.toJson();
    final data = <String, dynamic>{
      for (final key in group.keys)
        if (live.containsKey(key)) key: live[key],
    };
    switch (group) {
      case ProfileGroup.preferences:
        if (data['interface'] case final Map<String, dynamic> interface) {
          data['interface'] = {...interface}..remove(_scaleKey);
        }
      case ProfileGroup.layout:
        data['workspaces'] = _files(Workspace.userWorkspaceDir());
      case ProfileGroup.library:
        data['easings'] = _text(CustomEasings.storeFile());
      case ProfileGroup.presets:
        data['files'] = _files(_presetsDir);
      default:
    }
    // Through text and back, so what is compared and kept is exactly what a
    // file or the server would hand back.
    return jsonDecode(jsonEncode(data)) as Map<String, dynamic>;
  }

  Directory? get _presetsDir {
    try {
      final path = presetsDirPath();
      return path == null ? null : Directory(path);
    } catch (_) {
      return null;
    }
  }

  String? _text(File file) {
    try {
      return file.existsSync() ? file.readAsStringSync() : null;
    } catch (_) {
      return null;
    }
  }

  /// Every file in [dir] by name, as text.
  Map<String, String> _files(Directory? dir) {
    final found = <String, String>{};
    try {
      if (dir == null || !dir.existsSync()) return found;
      for (final entry in dir.listSync()) {
        if (entry is! File) continue;
        final text = _text(entry);
        if (text != null) found[entry.uri.pathSegments.last] = text;
      }
    } catch (_) {
      // A folder that will not list has nothing to offer.
    }
    return found;
  }

  /// Make [dir] hold exactly [files]: write each, and remove what is not
  /// among them. A name that could climb out of the folder is skipped,
  /// since these names may have come from the server.
  void _place(Directory? dir, Object? files) {
    if (dir == null || files is! Map) return;
    try {
      dir.createSync(recursive: true);
      final wanted = <String>{};
      for (final MapEntry(:key, :value) in files.entries) {
        if (key is! String || value is! String) continue;
        if (key.contains('/') || key.contains(r'\') || key.startsWith('.')) {
          continue;
        }
        wanted.add(key);
        final file = File('${dir.path}${Platform.pathSeparator}$key');
        if (_text(file) != value) file.writeAsStringSync(value);
      }
      for (final entry in dir.listSync()) {
        if (entry is File && !wanted.contains(entry.uri.pathSegments.last)) {
          entry.deleteSync();
        }
      }
    } catch (_) {
      // What could be written was.
    }
  }

  /// Write [groups] of [settings] into the live settings.
  void _apply(Map<String, dynamic> settings, Iterable<ProfileGroup> groups) {
    final live = workspace.toJson();
    final dock = workspace.dock;
    var layout = false;
    for (final group in groups) {
      final data = settings[group.name];
      if (data is! Map<String, dynamic>) continue;
      for (final key in group.keys) {
        if (!data.containsKey(key)) continue;
        if (key == 'interface' &&
            data[key] is Map &&
            live[key] is Map<String, dynamic>) {
          final here = live[key] as Map<String, dynamic>;
          live[key] = {
            ...(data[key] as Map).cast<String, dynamic>(),
            if (here.containsKey(_scaleKey)) _scaleKey: here[_scaleKey],
          };
        } else {
          live[key] = data[key];
        }
      }
      switch (group) {
        case ProfileGroup.layout:
          layout = true;
          _place(Workspace.userWorkspaceDir(), data['workspaces']);
        case ProfileGroup.library:
          final easings = data['easings'];
          if (easings is String && easings != _text(CustomEasings.storeFile())) {
            _writeText(CustomEasings.storeFile(), easings);
          }
          CustomEasings.reload();
        case ProfileGroup.presets:
          _place(_presetsDir, data['files']);
        default:
      }
    }
    _applying = true;
    try {
      // Through text and back for the same reason as reading out: the
      // settings file is read as decoded JSON and nothing else.
      workspace.applyJson(jsonDecode(jsonEncode(live)) as Map<String, dynamic>);
      // Reading settings in fronts the Project panel, as a launch does.
      // Unless the layout itself changed, the panels stay as they were.
      if (!layout) workspace.dock = dock;
      workspace.loadUserWorkspaces();
      workspace.recompose();
      workspace.save();
      afterApply();
    } finally {
      _applying = false;
    }
  }

  Map<String, dynamic>? _read(File file) {
    try {
      final j = jsonDecode(file.readAsStringSync());
      return j is Map<String, dynamic> ? j : null;
    } catch (_) {
      return null;
    }
  }

  void _write(File file, Object? data) => _writeText(file, _pretty.convert(data));

  void _writeText(File file, String text) {
    try {
      file.parent.createSync(recursive: true);
      file.writeAsStringSync(text);
    } catch (_) {
      // Tried.
    }
  }

  // --- Sync ----------------------------------------------------------------

  /// Whether the current profile is being kept on the server.
  bool get syncing => current.sync && account.pro;

  void setSync(bool on) {
    current.sync = on;
    _save();
    _noteStatus();
    notifyListeners();
    if (on) unawaited(syncNow());
  }

  void setGroupSync(ProfileGroup group, bool on) {
    on ? current.unsynced.remove(group) : current.unsynced.add(group);
    _save();
    notifyListeners();
    if (on) unawaited(syncNow());
  }

  void _noteStatus() {
    if (!syncing) {
      status = SyncStatus.off;
    } else if (status == SyncStatus.off) {
      status = SyncStatus.idle;
    }
  }

  /// A setting changed here. Wait for the changes to stop, then send them.
  void _settingsChanged() {
    if (_applying || !syncing) return;
    _settle?.cancel();
    _settle = Timer(const Duration(seconds: 3), syncNow);
  }

  /// Bring this machine and the server level, a group at a time.
  ///
  /// For each group the server's revision is compared with the one last
  /// seen here, and the settings here with what they were then. Nothing new
  /// on either side is nothing to do. New on one side is copied to the
  /// other. New on both is merged setting by setting ([mergeSettings]) and
  /// the result sent back.
  Future<void> syncNow() async {
    if (_gone || !_started || !syncing) return;
    if (_busy) {
      _again = true;
      return;
    }
    _busy = true;
    status = SyncStatus.syncing;
    notifyListeners();
    final profile = current;
    final who = account.account?.id;
    try {
      final index = (await account.call('GET', '/v1/sync'))['groups'];
      final seen = _read(_file(profile.id, 'synced')) ?? {};
      // What was seen belongs to one account. Signing this profile in to
      // another starts again from nothing.
      if (seen['account'] != who) seen.clear();
      seen['account'] = who;
      for (final group in ProfileGroup.values) {
        if (profile.unsynced.contains(group)) continue;
        // The profile was switched, the account changed or Lumit is closing
        // while this was under way. What is left belongs to none of them.
        if (_gone || profile != current || account.account?.id != who) return;
        final theirs = index is Map ? index[group.name] : null;
        final rev = theirs is Map && theirs['rev'] is int
            ? theirs['rev'] as int
            : 0;
        final last = seen[group.name];
        final known = last is Map && last['rev'] is int ? last['rev'] as int : 0;
        final base = last is Map ? last['data'] : null;
        var here = _read1(group);
        final changedHere = base == null || !_same(here, base);
        if (rev == known && !changedHere) continue;

        var send = changedHere;
        var basis = rev;
        if (rev != known) {
          final stored = await account.call('GET', '/v1/sync/${group.name}');
          final remote = stored['data'];
          basis = stored['rev'] is int ? stored['rev'] as int : rev;
          if (remote is Map<String, dynamic>) {
            // The first time this profile meets settings already on the
            // server, the server's are taken whole: that is what signing in
            // on a second machine is for. What was here is kept in a file
            // beside the profile in case that was the wrong way round.
            if (base == null) {
              _write(_file(profile.id, 'before-sync.${group.name}'), here);
            }
            final merged = base == null
                ? remote
                : mergeSettings(base, here, remote) as Map<String, dynamic>;
            if (!_same(merged, here)) {
              _apply({group.name: merged}, [group]);
              here = _read1(group);
            }
            send = !_same(here, remote);
          }
        }
        if (send) {
          if (utf8.encode(jsonEncode(here)).length > _groupLimit) continue;
          try {
            final saved = await account.call('PUT', '/v1/sync/${group.name}',
                body: {'base': basis, 'data': here});
            basis = saved['rev'] is int ? saved['rev'] as int : basis + 1;
          } on CloudError catch (e) {
            // Somebody else got there first. The next pass merges with it.
            if (e.status != 409) rethrow;
            _again = true;
            continue;
          }
        }
        seen[group.name] = {'rev': basis, 'data': here};
        _write(_file(profile.id, 'synced'), seen);
      }
      status = SyncStatus.idle;
      lastSynced = DateTime.now();
    } on CloudError catch (e) {
      // Without Pro there is nothing to keep in step, which is not a
      // failure. Anything else is said, and tried again on the next beat.
      status = e.status == 402 ? SyncStatus.off : SyncStatus.failed;
    } catch (_) {
      status = SyncStatus.failed;
    } finally {
      _busy = false;
      if (!_gone) {
        if (profile == current) notifyListeners();
        if (_again) {
          unawaited(Future<void>.delayed(const Duration(seconds: 1), syncNow));
        }
      }
      _again = false;
    }
  }

  /// A fingerprint of every synced group as it stands, for a test to tell
  /// whether two machines hold the same settings.
  @visibleForTesting
  String get fingerprint => _hash(_export());

  @override
  void dispose() {
    _gone = true;
    _settle?.cancel();
    _beat?.cancel();
    if (_started) {
      account.removeListener(_accountChanged);
      workspace.removeListener(_settingsChanged);
    }
    super.dispose();
  }
}
