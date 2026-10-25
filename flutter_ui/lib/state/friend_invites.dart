// Invites to multi-edit, sent to a friend and kept from Lumit's server.
//
// An invite holds the secret that opens a shared project, which the server
// is never to read. So each computer makes a key of its own and hands the
// server only its public half. An invite to a friend is sealed once for each
// of their computers' keys, and the server only passes the sealed copies
// on: X25519 between a key made for the one sending and the computer's,
// HKDF-SHA256 over that, then ChaCha20-Poly1305. The computer opens its copy
// with the private half, which never leaves its keychain.

import 'dart:async';
import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:flutter/foundation.dart';

import 'account.dart';
import 'secret_store.dart';

/// What an invite says once it is opened. [look] is the invite that can
/// only look.
typedef InviteText = ({String link, String? password, String project, bool look});

/// An invite a friend sent, opened.
typedef FriendInvite = ({String id, Friend from, InviteText text});

const _info = 'lumit friend invite v1';
final _x25519 = X25519();
final _aead = Chacha20.poly1305Aead();

Future<SecretKey> _key(SecretKey shared, List<int> sender, List<int> to) =>
    Hkdf(hmac: Hmac.sha256(), outputLength: 32).deriveKey(
        secretKey: shared, nonce: [...sender, ...to], info: utf8.encode(_info));

/// [text] sealed so only the holder of [to]'s private half can open it: the
/// sender's public key, the nonce, then the sealed text and its tag, as
/// base64url without padding.
Future<String> sealInvite(InviteText text, List<int> to) async {
  final mine = await _x25519.newKeyPair();
  final sender = (await mine.extractPublicKey()).bytes;
  final shared = await _x25519.sharedSecretKey(
      keyPair: mine,
      remotePublicKey: SimplePublicKey(to, type: KeyPairType.x25519));
  final box = await _aead.encrypt(
      utf8.encode(jsonEncode({
        'link': text.link,
        'password': text.password,
        'project': text.project,
        'look': text.look,
      })),
      secretKey: await _key(shared, sender, to));
  return base64Url
      .encode([...sender, ...box.nonce, ...box.cipherText, ...box.mac.bytes])
      .replaceAll('=', '');
}

/// What [sealed] says, opened with [keys]. Null for anything that is not an
/// invite sealed to them.
Future<InviteText?> openInvite(String sealed, SimpleKeyPair keys) async {
  try {
    final bytes = base64Url.decode(base64Url.normalize(sealed));
    if (bytes.length < 32 + 12 + 16) return null;
    final sender = bytes.sublist(0, 32);
    final to = (await keys.extractPublicKey()).bytes;
    final shared = await _x25519.sharedSecretKey(
        keyPair: keys,
        remotePublicKey: SimplePublicKey(sender, type: KeyPairType.x25519));
    final opened = await _aead.decrypt(
        SecretBox(bytes.sublist(44, bytes.length - 16),
            nonce: bytes.sublist(32, 44),
            mac: Mac(bytes.sublist(bytes.length - 16))),
        secretKey: await _key(shared, sender, to));
    final j = jsonDecode(utf8.decode(opened));
    if (j is! Map || j['link'] is! String) return null;
    return (
      link: j['link'] as String,
      password: j['password'] is String ? j['password'] as String : null,
      project: j['project'] is String ? j['project'] as String : '',
      look: j['look'] == true,
    );
  } catch (_) {
    return null;
  }
}

String _hex(List<int> bytes) =>
    bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();

List<int> _unhex(String hex) => [
      for (var i = 0; i + 1 < hex.length; i += 2)
        int.parse(hex.substring(i, i + 2), radix: 16)
    ];

/// The invites waiting for this computer, and sending them.
class FriendInvites extends ChangeNotifier {
  final AccountState account;

  FriendInvites(this.account);

  /// Newest first.
  List<FriendInvite> waiting = const [];

  /// Called once for each invite that comes while Lumit is open, to say so.
  void Function(FriendInvite)? onNew;

  SimpleKeyPair? _keys;
  String? _for;
  Timer? _poll;
  final Set<String> _seen = {};

  /// Begin following the account. Not called by a test.
  void start() {
    account.addListener(_accountChanged);
    _accountChanged();
  }

  void _accountChanged() {
    final who = account.signedIn ? account.account?.id : null;
    if (who == _for) return;
    _for = who;
    _keys = null;
    _seen.clear();
    _poll?.cancel();
    waiting = const [];
    notifyListeners();
    if (who == null) return;
    unawaited(_ready(who));
    // ponytail: asked once a minute while signed in. A computer on Pro's line
    // to the account could be told instead, if the minute ever matters.
    _poll = Timer.periodic(const Duration(minutes: 1), (_) => unawaited(check()));
  }

  /// This computer's key for the account, made the first time and kept in
  /// the keychain, and its public half given to the server for this sign-in.
  Future<void> _ready(String who) async {
    final slot = 'friend-key-$who';
    final kept = await readSecret(slot);
    var keys = kept == null ? null : await _x25519.newKeyPairFromSeed(_unhex(kept));
    if (keys == null) {
      keys = await _x25519.newKeyPair();
      await writeSecret(slot, _hex(await keys.extractPrivateKeyBytes()));
    }
    if (_for != who) return;
    _keys = keys;
    try {
      await account.call('PUT', '/v1/keys',
          body: {'key': _hex((await keys.extractPublicKey()).bytes)});
    } on CloudError {
      // Tried again at the next sign-in.
    }
    await check();
  }

  /// Read the invites waiting for this computer again.
  Future<void> check() async {
    final keys = _keys, who = _for;
    if (keys == null || who == null) return;
    final Map<String, dynamic> j;
    try {
      j = await account.call('GET', '/v1/invites');
    } on CloudError {
      return;
    }
    final list = j['invites'];
    final opened = <FriendInvite>[];
    for (final i in list is List ? list : const []) {
      if (i is! Map || i['id'] is! String || i['sealed'] is! String) continue;
      final text = await openInvite(i['sealed'] as String, keys);
      if (text == null) continue;
      final from = i['from'] is Map ? i['from'] as Map : const {};
      opened.add((
        id: i['id'] as String,
        from: (
          username: '${from['username'] ?? ''}',
          name: '${from['name'] ?? ''}'
        ),
        text: text,
      ));
    }
    if (_for != who) return;
    waiting = opened;
    notifyListeners();
    for (final invite in opened) {
      if (_seen.add(invite.id)) onNew?.call(invite);
    }
  }

  /// Send [text] to [username], sealed for each of their computers. False
  /// when they have no computer to send it to yet.
  Future<bool> send(String username, InviteText text) async {
    final name = Uri.encodeComponent(username);
    final j = await account.call('GET', '/v1/friends/$name/keys');
    final keys = [
      for (final k in j['keys'] is List ? j['keys'] as List : const [])
        if (k is String) k
    ];
    if (keys.isEmpty) return false;
    await account.call('POST', '/v1/invites', body: {
      'to': username,
      'sealed': [
        for (final k in keys) {'key': k, 'box': await sealInvite(text, _unhex(k))}
      ],
    });
    return true;
  }

  /// Joined or turned down: gone from every computer of the account.
  Future<void> dismiss(String id) async {
    waiting = [for (final i in waiting) if (i.id != id) i];
    notifyListeners();
    try {
      await account.call('DELETE', '/v1/invites/$id');
    } on CloudError {
      // It runs out by itself in a week.
    }
  }

  @override
  void dispose() {
    _poll?.cancel();
    account.removeListener(_accountChanged);
    super.dispose();
  }
}

/// For a test: a key pair from its private half.
@visibleForTesting
Future<SimpleKeyPair> inviteKeys(Uint8List seed) =>
    _x25519.newKeyPairFromSeed(seed);
