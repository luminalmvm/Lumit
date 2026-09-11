# Release signing

## What this is for

The updater asks GitHub what the newest release is. GitHub answers with a
download address and, on current API responses, the attachment's SHA-256. The
updater then checks the downloaded file against that digest before running it
(`flutter_ui/lib/state/updates.dart`).

That check is worth having and it is not provenance. The digest arrives in the
same answer as the address it describes, from the same account, over the same
connection. Anything able to put a different installer on the release could put
its digest beside it and the check would pass. What the digest defends against
is the file being damaged or swapped **in transit**; what it cannot defend
against is the release infrastructure itself going wrong — which is the threat
that matters, because the thing at the end of this pipe is an installer, and an
installer is the most dangerous file Lumit ever touches.

So a release carries two more attachments:

| File | What it is |
|---|---|
| `lumit-release-manifest.json` | Every attachment, with its size and SHA-256 |
| `lumit-release-manifest.json.sig` | An Ed25519 signature over those exact bytes, base64 |

and the application carries the matching **public** key, compiled in
(`updateSigningKey` in `flutter_ui/lib/state/release_signature.dart`). A release
is believed when the signature is that key's; an attachment is believed when the
signed manifest says that is its name, its size and its digest.

A compromised release account can publish whatever it likes. Without the private
key it cannot make a Lumit with a pinned key install any of it.

## The switch

`updateSigningKey` is **empty** in a tree where nobody has generated a key, and
while it is empty the updater keeps exactly the behaviour it had: the GitHub
digest, checked as before. The moment a key is pinned, a signed manifest becomes
compulsory and a release without one is refused.

That is deliberate, and it is the only reason the two halves can land
separately. It also means the security property this note describes **is not in
force until somebody throws the switch.** The test
`the shipped build pins no key yet, and says so rather than pretending` fails
the day it is, which is the reminder.

Throwing it is one commit and one secret, and they must go together: a key
pinned without the secret means every release fails to install, and a secret set
without a pinned key means the manifests are published and ignored.

## Generating and pinning the key

Ed25519. Two commands, run **once**, on a machine that is not a CI runner:

```sh
# The private half. This never goes near the repository.
openssl genpkey -algorithm ed25519 -out lumit-release-key.pem

# The public half, raw 32 bytes, base64 — what gets pinned.
openssl pkey -in lumit-release-key.pem -pubout -outform DER | tail -c 32 | base64 -w0
```

Then:

1. Put the **contents of `lumit-release-key.pem`** in the repository secret
   `RELEASE_SIGNING_KEY`. The release workflow's "Sign the release manifest"
   step writes it to the runner's temporary directory under `umask 077`, uses
   it, and deletes it.
2. Put the **public** base64 line in `updateSigningKey`.
3. Delete the guard test named above.

Keep the private key offline and backed up. Losing it means the next release
cannot be installed by anybody running a build with the old key pinned, and the
only way out of that is a manual download — which is why it is worth keeping
somewhere a laptop failure does not reach.

**Rotation** is the same shape and has the same trap: a build pins one key, so a
release signed with a new key cannot be installed by builds that pin the old
one. Rotating means signing with both keys for at least one release cycle, or
accepting that everyone updates by hand once. Neither is pleasant; the private
key not leaking is much the cheaper path.

## How signing works, exactly

The release workflow builds the manifest **from the files themselves**, never
from a list written by hand — a name that drifts is then a signature that does
not match, which is noticed, rather than an entry quietly missing.

`openssl pkeyutl -sign -rawin` is Ed25519's own one-shot signing over the whole
message. It is not a hash-then-sign, and `-rawin` is what selects it; without
that flag OpenSSL would want a digest and produce something the verifier cannot
check.

The verifier is `package:cryptography`'s `Ed25519` in Dart. These two halves are
different languages on different machines with nothing in the type system
connecting them, so `flutter_ui/test/release_pipeline_signature_test.dart`
carries a real manifest and a real signature made by the workflow's exact shell,
and checks them with the updater's exact code. Regenerate that fixture with the
commands above if either half changes.

## What the updater does with it

In `_verify`, on a build with a key pinned:

1. The release must carry both attachments. Missing → refused (`updateNotSigned`).
2. They are fetched with a 64 KiB cap, checked as the bytes arrive rather than
   from a `Content-Length` header, because the header is the server's claim.
3. The signature is checked **before the JSON is parsed**, so a manifest nobody
   signed never reaches the parser.
4. The manifest must name the downloaded attachment, at its exact size and
   digest. Anything else → refused (`updateNotInManifest`).

A failure at any step is a refusal, never a fallback to the weaker check.
Falling back would mean anything able to publish a release could also delete the
manifest and be trusted again, which would make the whole mechanism decorative.

## Test plan

- `test/release_signature_test.dart` — a real generated key pair, signing real
  manifests: the good case, a manifest signed by somebody else, a manifest
  edited after signing, a signature from the previous release replayed at this
  one, signatures and keys of the wrong length or alphabet, and every malformed
  manifest shape (missing version, empty asset list, a digest that is not 64 hex
  characters, a size as text, a zero size, a nameless asset).
- `test/release_pipeline_signature_test.dart` — the cross-language fixture
  above, plus one byte changed in the manifest breaking the signature.
- `test/updates_test.dart` — the state machine around it.

## What this does not do

- It does not protect a user who downloads an installer from the website by
  hand. That is what the platform code signatures are for (Authenticode,
  Developer ID), and they are a separate mechanism with separate keys.
- It does not make the *build* reproducible. A signature says who published the
  artefact, not that the artefact is what the source builds to. Reproducible
  builds are a different and much larger piece of work.
- It does not help if the signing key is on the same machine as the release
  credentials. Keeping them apart is the whole point.
