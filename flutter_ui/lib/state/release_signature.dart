// Whether a downloaded release really is one of ours.
//
// # In plain terms
//
// The updater asks GitHub what the newest release is, and GitHub answers with a
// download address and — on newer responses — the attachment's SHA-256. The
// updater then checks the file against that digest before running it.
//
// That check is worth having, and it is not provenance. The digest arrives in
// the same answer as the address it describes, from the same account, over the
// same connection: anything that could put a different installer on the release
// could put its digest beside it, and the check would pass. What it defends
// against is the file being damaged or swapped *in transit*. What it cannot
// defend against is the release infrastructure itself going wrong, which is the
// threat that matters, because the file at the end of it is an installer and an
// installer is the most dangerous thing Lumit ever touches.
//
// So a release carries one more thing: a small **manifest** naming every
// attachment and its size and digest, and a **detached signature** over that
// manifest made with a key that lives nowhere near GitHub. The application
// carries the matching public key, compiled in ([updateSigningKey]). A release
// is believed when the signature is this key's, and an attachment is believed
// when the signed manifest says that is its size and its digest. A compromised
// release account can publish whatever it likes; without the private key it
// cannot make this application run any of it.
//
// **Pinning a key is a deliberate act.** [updateSigningKey] is empty in a tree
// where nobody has generated one yet, and while it is empty this file refuses
// nothing — the updater falls back to the digest check, which is where it was
// before. The moment a key is pinned, a signed manifest becomes compulsory and
// an update that has none is refused. That is the whole of the switch, and the
// steps to throw it are in `docs/impl/release-signing.md`.
//
// **Nothing here reads the network or the disk.** It is given bytes and answers
// yes or no, which is what lets the updater's tests drive the whole sequence.

import 'dart:convert';
import 'dart:typed_data';

import 'package:cryptography/cryptography.dart';

/// The public half of the release signing key, base64, raw 32 bytes.
///
/// Empty until somebody generates the pair and pins it here. See
/// `docs/impl/release-signing.md` §"Signing a release" for how, in two commands.
///
/// This is a **public** key: it is meant to be read, copied and published. The
/// private half never goes near this repository or the release runner's
/// ordinary secrets — it is what makes a release a release.
const String updateSigningKey = '';

/// Whether this build insists on a signed release manifest.
///
/// The single switch. False in a tree with no key pinned, where the updater
/// keeps the behaviour it had; true the moment one is, from which point an
/// unsigned update is refused rather than installed.
bool get releaseSigningIsEnforced => updateSigningKey.isNotEmpty;

/// What one attachment is, as the signed manifest says it.
class SignedAsset {
  const SignedAsset({
    required this.name,
    required this.size,
    required this.sha256,
  });

  /// The attachment's file name, exactly as it is on the release.
  final String name;

  /// How many bytes it is.
  final int size;

  /// Its SHA-256, lower-case hex, no prefix.
  final String sha256;
}

/// The name the release attaches its manifest under, and its signature.
///
/// Fixed rather than derived from the version, so the updater can find them in
/// a release's attachment list without first having to agree about how a
/// version is spelled.
const String releaseManifestName = 'lumit-release-manifest.json';
const String releaseManifestSignatureName = 'lumit-release-manifest.json.sig';

/// The most a manifest may weigh. It is a list of a dozen file names and
/// digests; sixty-four kilobytes is a hundred times that and still nothing.
const int releaseManifestMaxBytes = 64 * 1024;

/// Why a release could not be believed. Each is a distinct thing to tell the
/// user, and each is a refusal rather than a warning.
enum ReleaseTrust {
  /// The signature is this application's key's, and the manifest holds the
  /// attachment that was downloaded.
  trusted,

  /// The release has no signed manifest at all. On a build with a key pinned
  /// this is a refusal: every release made since the key existed has one, so a
  /// release without one is a release made by somebody else.
  unsigned,

  /// A manifest is there and the signature over it is not this key's.
  badSignature,

  /// The signature checks out and the manifest does not name the attachment
  /// that was downloaded, or names it with a different size or digest.
  notInManifest,

  /// The manifest is not a manifest: too big, not JSON, or the wrong shape.
  malformed,
}

/// The signed manifest, once it has been believed.
class VerifiedManifest {
  const VerifiedManifest({required this.version, required this.assets});

  /// The version the manifest is for, as the release spells it.
  final String version;

  /// Every attachment it names.
  final List<SignedAsset> assets;

  /// What it says about [name], or null if it says nothing.
  SignedAsset? assetNamed(String name) {
    for (final asset in assets) {
      if (asset.name == name) return asset;
    }
    return null;
  }
}

/// Check a detached signature over `manifestBytes` against [updateSigningKey],
/// and read the manifest if it holds up.
///
/// `signatureBase64` is the signature file's contents: base64 of the raw 64
/// signature bytes, with any surrounding whitespace ignored.
///
/// Returns the manifest and [ReleaseTrust.trusted], or null and the reason.
/// **The signature is checked before the JSON is parsed**, so a manifest nobody
/// signed never reaches the parser at all.
Future<(VerifiedManifest?, ReleaseTrust)> verifyManifest(
  List<int> manifestBytes,
  String signatureBase64,
) =>
    verifyManifestWithKey(manifestBytes, signatureBase64, updateSigningKey);

/// [verifyManifest], against a key the caller names.
///
/// The same code, one step earlier: [verifyManifest] is this with the pinned
/// key filled in. Split out because the pinned key is a `const` and a test
/// cannot assign one — so the tests generate a real Ed25519 pair, sign real
/// manifests with it, and drive exactly the path the shipped updater takes.
/// A verification nobody has watched succeed *and* fail is not one to rely on.
Future<(VerifiedManifest?, ReleaseTrust)> verifyManifestWithKey(
  List<int> manifestBytes,
  String signatureBase64,
  String publicKeyBase64,
) async {
  if (publicKeyBase64.isEmpty) {
    // Nothing to check against. The caller decides what that means; this
    // function will not invent a verdict it cannot support.
    return (null, ReleaseTrust.unsigned);
  }
  if (manifestBytes.length > releaseManifestMaxBytes) {
    return (null, ReleaseTrust.malformed);
  }

  final Uint8List key;
  final Uint8List signature;
  try {
    key = base64.decode(publicKeyBase64.trim());
    signature = base64.decode(signatureBase64.trim());
  } on FormatException {
    return (null, ReleaseTrust.badSignature);
  }
  // Ed25519: a 32-byte public key and a 64-byte signature, always. Checking the
  // lengths here means a truncated or padded file is a refusal rather than
  // something the library has to have an opinion about.
  if (key.length != 32 || signature.length != 64) {
    return (null, ReleaseTrust.badSignature);
  }

  final ok = await Ed25519().verify(
    manifestBytes,
    signature: Signature(
      signature,
      publicKey: SimplePublicKey(key, type: KeyPairType.ed25519),
    ),
  );
  if (!ok) return (null, ReleaseTrust.badSignature);

  final manifest = parseManifest(manifestBytes);
  if (manifest == null) return (null, ReleaseTrust.malformed);
  return (manifest, ReleaseTrust.trusted);
}

/// Read a manifest's JSON. Public for the tests; callers go through
/// [verifyManifest], which will not parse an unsigned one.
///
/// Every field is checked rather than cast: this is a file off the network, and
/// a shape that is nearly right is exactly the shape that gets believed by
/// accident. Null for anything that is not a manifest.
VerifiedManifest? parseManifest(List<int> bytes) {
  if (bytes.length > releaseManifestMaxBytes) return null;
  final Object? json;
  try {
    json = jsonDecode(utf8.decode(bytes));
  } on FormatException {
    return null;
  }
  if (json is! Map<String, dynamic>) return null;

  final version = json['version'];
  if (version is! String || version.isEmpty) return null;

  final rawAssets = json['assets'];
  if (rawAssets is! List || rawAssets.isEmpty) return null;

  final assets = <SignedAsset>[];
  for (final raw in rawAssets) {
    if (raw is! Map) return null;
    final asset = raw.cast<String, dynamic>();
    final name = asset['name'];
    final size = asset['size'];
    final digest = asset['sha256'];
    if (name is! String || name.isEmpty) return null;
    if (size is! int || size <= 0) return null;
    if (digest is! String) return null;
    // Exactly 64 hex characters. A digest that is nearly a digest is not one,
    // and a comparison against it would be a comparison that always fails or,
    // worse, one that is never made.
    final normalised = digest.toLowerCase();
    if (normalised.length != 64 ||
        !normalised.split('').every((c) => '0123456789abcdef'.contains(c))) {
      return null;
    }
    assets.add(SignedAsset(name: name, size: size, sha256: normalised));
  }
  return VerifiedManifest(version: version, assets: assets);
}
