// A person's Lumit account, and what Lumit's own servers are asked for.
//
// Lumit works with no account at all. Signing in to one is what Lumit Pro
// hangs off: the relay a shared project can always be joined through, and
// settings that follow a person from one machine to the next.
//
// Everything here speaks to one server, over HTTPS, and nothing here is on a
// rebuild path. A sign-in is two tokens. The short one goes with each request
// and lives in memory for an hour. The long one fetches the next short one,
// is replaced every time it is used, and is kept in the system's own secret
// store ([writeSecret]), never in the settings file.
//
// A password never leaves this machine. What is sent in its place is a key
// stretched from it ([passwordKey]), so the server could not say what the
// password was if it were asked, and a copy of its database is no use for
// guessing one without doing that stretching for every guess.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';
import 'dart:math';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';

import 'external_links.dart';
import 'secret_store.dart';

/// Where Lumit's own servers are. Read from the environment first so a
/// development build can be pointed at a server on this machine.
String get cloudUrl =>
    Platform.environment['LUMIT_CLOUD'] ?? 'https://cloud.lumitlab.com';

/// The page a browser is left on once a sign-in from it has reached Lumit.
const String signedInPage = 'https://lumitlab.com/signed-in';

/// What the server refused with, or `offline` when it was not reached.
class CloudError implements Exception {
  final String code;
  final int status;

  /// What else the answer said. A refused save carries what is stored.
  final Map<String, dynamic> body;
  const CloudError(this.code, [this.status = 0, this.body = const {}]);

  bool get offline => code == 'offline';

  @override
  String toString() => 'CloudError($code, $status)';
}

/// What one plan costs, as the server reads it from the shop.
class CloudPrice {
  final String amount;
  final String currency;
  const CloudPrice(this.amount, this.currency);

  static CloudPrice? from(Object? j) => j is Map &&
          j['amount'] is String &&
          j['currency'] is String
      ? CloudPrice(j['amount'] as String, j['currency'] as String)
      : null;
}

/// What the server has switched on. The sign-in window only offers what is.
class CloudConfig {
  final bool email, google, discord;

  /// Pro is free for everyone who signs in, while it is being tried out.
  final bool preview;

  /// Whether a subscription can be bought yet.
  final bool billing;
  final CloudPrice? monthly, yearly;

  const CloudConfig({
    this.email = false,
    this.google = false,
    this.discord = false,
    this.preview = false,
    this.billing = false,
    this.monthly,
    this.yearly,
  });

  factory CloudConfig.from(Map<String, dynamic> j) {
    final prices = j['prices'];
    return CloudConfig(
      email: j['email'] == true,
      google: j['google'] == true,
      discord: j['discord'] == true,
      preview: j['preview'] == true,
      billing: j['billing'] == true,
      monthly: prices is Map ? CloudPrice.from(prices['monthly']) : null,
      yearly: prices is Map ? CloudPrice.from(prices['yearly']) : null,
    );
  }

  bool get anySignIn => email || google || discord;
}

/// A subscription as the shop last described it.
class CloudSubscription {
  /// `active`, `trialing`, `past_due`, `paused` or `canceled`.
  final String status;

  /// `month`, `year`, or null when the shop did not say.
  final String? interval;

  /// When it next renews, and when it ends if it has been cancelled.
  final DateTime? renews, ends;

  const CloudSubscription(this.status, this.interval, this.renews, this.ends);

  static CloudSubscription? from(Object? j) {
    if (j is! Map || j['status'] is! String) return null;
    DateTime? at(Object? seconds) => seconds is int
        ? DateTime.fromMillisecondsSinceEpoch(seconds * 1000)
        : null;
    return CloudSubscription(j['status'] as String,
        j['interval'] as String?, at(j['renews']), at(j['ends']));
  }
}

/// The account a profile is signed in to.
class CloudAccount {
  final String id, email, name;

  /// A picture's address, from whoever the person signed in with.
  final String? avatar;
  final bool pro;

  /// Pro only because everyone is, for now.
  final bool preview;
  final CloudSubscription? subscription;

  /// How this account can be signed in to: `password`, `google`, `discord`.
  final List<String> providers;

  const CloudAccount({
    required this.id,
    required this.email,
    required this.name,
    this.avatar,
    this.pro = false,
    this.preview = false,
    this.subscription,
    this.providers = const [],
  });

  factory CloudAccount.from(Map<String, dynamic> j) => CloudAccount(
        id: '${j['id']}',
        email: '${j['email']}',
        name: '${j['name']}',
        avatar: j['avatar'] is String ? j['avatar'] as String : null,
        pro: j['plan'] == 'pro',
        preview: j['preview'] == true,
        subscription: CloudSubscription.from(j['subscription']),
        providers: [
          if (j['providers'] case final List<dynamic> all)
            for (final p in all)
              if (p is String) p,
        ],
      );

  Map<String, dynamic> toJson() => {
        'id': id,
        'email': email,
        'name': name,
        'avatar': avatar,
        'plan': pro ? 'pro' : 'free',
        'preview': preview,
        'providers': providers,
      };
}

/// Who a person can sign in with besides a password.
enum SignInWith { google, discord }

/// How many times a password is stretched, and the shortest one taken. The
/// count is the one OWASP gives for PBKDF2 with SHA-256.
const int _stretch = 600000;
const int shortestPassword = 10;

/// The key sent in place of [password]: PBKDF2-HMAC-SHA256 over it, salted
/// with the address it belongs to so the same password on two accounts comes
/// to two keys. About two seconds, off the UI thread.
// ponytail: plain Dart, so a slow machine waits a few seconds to sign in.
// The engine could do it in a tenth of the time if that ever matters.
Future<String> passwordKey(String email, String password) => Isolate.run(() {
      final salt = sha256
          .convert(utf8.encode('lumit-cloud/v1/${email.trim().toLowerCase()}'))
          .bytes;
      final hmac = Hmac(sha256, utf8.encode(password));
      var block = hmac.convert([...salt, 0, 0, 0, 1]).bytes;
      final key = Uint8List.fromList(block);
      for (var round = 1; round < _stretch; round++) {
        block = hmac.convert(block).bytes;
        for (var i = 0; i < key.length; i++) {
          key[i] ^= block[i];
        }
      }
      return _hex(key);
    });

String _hex(List<int> bytes) =>
    [for (final b in bytes) b.toRadixString(16).padLeft(2, '0')].join();

Uint8List _random(int count) {
  final random = Random.secure();
  return Uint8List.fromList(
      [for (var i = 0; i < count; i++) random.nextInt(256)]);
}

String _base64Url(List<int> bytes) =>
    base64Url.encode(bytes).replaceAll('=', '');

/// The account the current profile is signed in to, if it is, and every call
/// made on its behalf.
class AccountState extends ChangeNotifier {
  /// What to say this machine is, on the list of places an account is signed
  /// in. Its name and nothing else.
  static String get deviceLabel {
    final name = Platform.localHostname;
    return name.length > 60 ? name.substring(0, 60) : name;
  }

  final HttpClient _http = HttpClient()
    ..connectionTimeout = const Duration(seconds: 10)
    ..userAgent = 'Lumit';

  CloudConfig? config;
  CloudAccount? account;

  bool get signedIn => account != null;
  bool get pro => account?.pro ?? false;

  /// The secret store's name for the current profile's long token.
  String? _slot;
  String? _access;
  DateTime _accessExpires = DateTime.fromMillisecondsSinceEpoch(0);

  /// One refresh at a time. The long token is replaced each time it is
  /// used, so two requests refreshing at once would have the second present
  /// a token that had just been retired, which the server reads as theft and
  /// answers by ending the sign-in.
  Future<bool>? _refreshing;

  /// Point this at the profile whose sign-in is kept under [slot], and pick
  /// up where that profile left off. [remembered] is the account it was
  /// signed in to when Lumit last ran, shown until the server answers so the
  /// strip does not flash "signed out" on every launch.
  Future<void> use(String slot, {CloudAccount? remembered}) async {
    _slot = slot;
    _access = null;
    account = null;
    if (await readSecret(slot) == null) {
      notifyListeners();
      return;
    }
    account = remembered;
    notifyListeners();
    await refreshAccount();
  }

  /// Ask the server what it offers. Kept, so the sign-in window opens with
  /// the right buttons on it. A server that can't be reached leaves the
  /// last answer standing.
  Future<CloudConfig?> loadConfig() async {
    try {
      final (_, body) = await _send('GET', '/v1/config');
      config = CloudConfig.from(body);
      notifyListeners();
    } on CloudError {
      // Offline, or not there yet.
    }
    return config;
  }

  // --- Requests ------------------------------------------------------------

  /// One request. Throws a [CloudError] for anything but a 2xx answer.
  Future<(int, Map<String, dynamic>)> _send(String method, String path,
      {Object? body, String? bearer}) async {
    final HttpClientResponse response;
    final String text;
    try {
      final request = await _http
          .openUrl(method, Uri.parse('$cloudUrl$path'))
          .timeout(const Duration(seconds: 15));
      request.followRedirects = false;
      if (bearer != null) {
        request.headers.set(HttpHeaders.authorizationHeader, 'Bearer $bearer');
      }
      if (body != null) {
        final bytes = utf8.encode(jsonEncode(body));
        request.headers.contentType = ContentType.json;
        request.contentLength = bytes.length;
        request.add(bytes);
      }
      response = await request.close().timeout(const Duration(seconds: 30));
      text = await utf8.decodeStream(response);
    } on CloudError {
      rethrow;
    } catch (_) {
      throw const CloudError('offline');
    }
    Map<String, dynamic> json = const {};
    try {
      final decoded = text.isEmpty ? null : jsonDecode(text);
      if (decoded is Map<String, dynamic>) json = decoded;
    } catch (_) {
      // Not JSON: an error page from something in between.
    }
    if (response.statusCode < 200 || response.statusCode >= 300) {
      final code = json['error'];
      throw CloudError(
          code is String ? code : 'unknown', response.statusCode, json);
    }
    return (response.statusCode, json);
  }

  /// A request made as the signed-in account. The short token is renewed
  /// first when it is about to run out, and once more if the server says it
  /// already has.
  Future<Map<String, dynamic>> call(String method, String path,
      {Object? body}) async {
    final token = await accessToken();
    if (token == null) throw const CloudError('signed_out', 401);
    try {
      return (await _send(method, path, body: body, bearer: token)).$2;
    } on CloudError catch (e) {
      // 402 and 409 answer with what is stored, which a caller may want.
      if (e.status != 401) rethrow;
    }
    _access = null;
    final again = await accessToken();
    if (again == null) throw const CloudError('signed_out', 401);
    return (await _send(method, path, body: body, bearer: again)).$2;
  }

  /// A short token that is good for at least another minute, or null when
  /// nobody is signed in.
  Future<String?> accessToken() async {
    final good = _accessExpires
        .isAfter(DateTime.now().add(const Duration(minutes: 1)));
    if (_access != null && good) return _access;
    if (_slot == null) return null;
    final refreshed = await (_refreshing ??= _refresh().whenComplete(() {
      _refreshing = null;
    }));
    return refreshed ? _access : null;
  }

  Future<bool> _refresh() async {
    final slot = _slot;
    if (slot == null) return false;
    // Read from the store every time, never from memory: a second Lumit on
    // this machine may have used the token since, and left the next one.
    final refresh = await readSecret(slot);
    if (refresh == null) return _signedOut();
    try {
      final (_, body) =
          await _send('POST', '/v1/auth/refresh', body: {'refresh': refresh});
      await _keep(slot, body);
      return true;
    } on CloudError catch (e) {
      // Only the server saying no ends a sign-in. Being offline does not.
      if (e.status == 401) return _signedOut();
      return false;
    }
  }

  Future<void> _keep(String slot, Map<String, dynamic> tokens) async {
    final refresh = tokens['refresh'];
    final expires = tokens['access_expires'];
    if (refresh is String) await writeSecret(slot, refresh);
    _access = tokens['access'] as String?;
    _accessExpires = DateTime.fromMillisecondsSinceEpoch(
        (expires is int ? expires : 0) * 1000);
  }

  Future<bool> _signedOut() async {
    final slot = _slot;
    if (slot != null) await deleteSecret(slot);
    _access = null;
    if (account != null) {
      account = null;
      notifyListeners();
    }
    return false;
  }

  /// Take a sign-in the server just handed over and make it this profile's.
  Future<void> _adopt(Map<String, dynamic> signIn) async {
    final slot = _slot;
    if (slot == null) throw const CloudError('no_profile');
    await _keep(slot, signIn);
    final who = signIn['account'];
    if (who is Map<String, dynamic>) account = CloudAccount.from(who);
    notifyListeners();
  }

  // --- Signing in ----------------------------------------------------------

  /// Ask for an account under [email]. The server posts a six-digit code to
  /// it either way, and says nothing here about whether the address was
  /// already known, so nobody can use this to find out who has an account.
  Future<void> signUp(String email, String password, String name) async {
    final key = await _keyFor(email, password);
    await _send('POST', '/v1/auth/signup',
        body: {'email': email.trim(), 'key': key, 'name': name.trim()});
  }

  /// The key last made here, and the address it was made for. Held between
  /// asking for an account and typing its code, and no longer.
  (String, String)? _pending;

  Future<String> _keyFor(String email, String password) async {
    final key = await passwordKey(email, password);
    _pending = (email.trim().toLowerCase(), key);
    return key;
  }

  /// Finish [signUp] with the code that was posted.
  ///
  /// The password's key goes with the code. If someone else asked for an
  /// account under this address in between, the code in the inbox is for
  /// their password and not this one, and the server then says no to it.
  Future<void> verify(String email, String code, String password) async {
    final pending = _pending;
    final key = pending != null && pending.$1 == email.trim().toLowerCase()
        ? pending.$2
        : await passwordKey(email, password);
    final signIn = await _send('POST', '/v1/auth/verify', body: {
      'email': email.trim(),
      'code': code.trim(),
      'key': key,
      'device': deviceLabel,
    });
    _pending = null;
    await _adopt(signIn.$2);
  }

  Future<void> signIn(String email, String password) async {
    final key = await _keyFor(email, password);
    await _adopt((await _send('POST', '/v1/auth/signin', body: {
      'email': email.trim(),
      'key': key,
      'device': deviceLabel,
    }))
        .$2);
    _pending = null;
  }

  /// Post a code to [email] for choosing a new password.
  Future<void> resetStart(String email) async =>
      _send('POST', '/v1/auth/reset/start', body: {'email': email.trim()});

  Future<void> resetFinish(String email, String code, String password) async {
    final key = await passwordKey(email, password);
    await _adopt((await _send('POST', '/v1/auth/reset/finish', body: {
      'email': email.trim(),
      'code': code.trim(),
      'key': key,
      'device': deviceLabel,
    }))
        .$2);
  }

  /// The door a sign-in in the browser comes back through, while one is
  /// under way.
  HttpServer? _returning;

  /// Sign in with a Google or Discord account, in the person's own browser.
  ///
  /// Lumit listens on this machine alone, opens the browser at the server,
  /// and the server sends the browser back here once the person has agreed.
  /// What comes back is worth nothing without a secret that never left this
  /// process, so another program listening in on this machine gains nothing.
  /// Throws `cancelled` when [cancelSignIn] is called, and `timed_out` after
  /// five minutes of nobody coming back.
  Future<void> signInWith(SignInWith provider) async {
    cancelSignIn();
    final verifier = _base64Url(_random(32));
    final challenge = _base64Url(sha256.convert(ascii.encode(verifier)).bytes);
    final state = _base64Url(_random(16));
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    _returning = server;
    try {
      final start = Uri.parse(
              '$cloudUrl/v1/auth/oauth/${provider.name}/start')
          .replace(queryParameters: {
        'challenge': challenge,
        'port': '${server.port}',
        'state': state,
      });
      if (!await openExternalLink('$start')) {
        throw const CloudError('no_browser');
      }
      String? grant;
      await for (final request
          in server.timeout(const Duration(minutes: 5), onTimeout: (sink) {
        sink.close();
      })) {
        final query = request.uri.queryParameters;
        final ours = query['state'] == state;
        final failed = query['error'];
        // Anything that is not this sign-in coming back is turned away
        // without being told why.
        request.response.statusCode =
            ours ? HttpStatus.found : HttpStatus.notFound;
        if (ours) {
          request.response.headers.set(
              HttpHeaders.locationHeader,
              failed == null
                  ? signedInPage
                  : '$signedInPage?error=${Uri.encodeQueryComponent(failed)}');
        }
        await request.response.close();
        if (!ours) continue;
        if (failed != null) throw CloudError(failed);
        grant = query['grant'];
        break;
      }
      if (grant == null) {
        throw CloudError(_returning == server ? 'timed_out' : 'cancelled');
      }
      await _adopt((await _send('POST', '/v1/auth/oauth/token', body: {
        'grant': grant,
        'verifier': verifier,
        'device': deviceLabel,
      }))
          .$2);
    } finally {
      if (_returning == server) _returning = null;
      await server.close(force: true);
    }
  }

  /// Stop waiting for the browser to come back.
  void cancelSignIn() {
    final waiting = _returning;
    _returning = null;
    waiting?.close(force: true);
  }

  /// Sign out of this profile's account, here and at the server.
  Future<void> signOut() async {
    final token = _access;
    if (token != null) {
      try {
        await _send('POST', '/v1/auth/signout', bearer: token);
      } on CloudError {
        // The token here is forgotten either way.
      }
    }
    await _signedOut();
  }

  // --- The account ---------------------------------------------------------

  /// Read the account again: its name, and whether it has Pro.
  Future<void> refreshAccount() async {
    try {
      account = CloudAccount.from(await call('GET', '/v1/me'));
      notifyListeners();
    } on CloudError {
      // Offline keeps what is known. Signed out has already been noted.
    }
  }

  Future<void> rename(String name) async {
    account = CloudAccount.from(
        await call('PATCH', '/v1/me', body: {'name': name.trim()}));
    notifyListeners();
  }

  /// Delete the account, its subscription and everything kept for it.
  Future<void> deleteAccount() async {
    await call('DELETE', '/v1/me');
    await _signedOut();
  }

  /// The shop's page for buying Pro by the month or the year.
  Future<String> checkoutUrl({required bool yearly}) async =>
      (await call('POST', '/v1/billing/checkout',
          body: {'plan': yearly ? 'yearly' : 'monthly'}))['url'] as String;

  /// The shop's page for changing or ending a subscription.
  Future<String> portalUrl() async =>
      (await call('POST', '/v1/billing/portal'))['url'] as String;

  /// Look for Pro arriving, after the person was sent to the shop: the shop
  /// tells the server, and the server is asked here every few seconds until
  /// it says so or [patience] runs out. True once the account has Pro.
  Future<bool> awaitPro(
      {Duration patience = const Duration(minutes: 15)}) async {
    final until = DateTime.now().add(patience);
    final wasFor = account?.id;
    while (DateTime.now().isBefore(until)) {
      await Future<void>.delayed(const Duration(seconds: 4));
      if (account?.id != wasFor) return false;
      await refreshAccount();
      if (account?.subscription != null && pro) return true;
    }
    return false;
  }

  @override
  void dispose() {
    cancelSignIn();
    _http.close(force: true);
    super.dispose();
  }
}
