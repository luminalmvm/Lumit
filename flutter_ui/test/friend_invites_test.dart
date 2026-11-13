import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/friend_invites.dart';

void main() {
  test('an invite sealed to a computer opens with its key and no other', () async {
    final mine = await inviteKeys(Uint8List.fromList(List.filled(32, 7)));
    final other = await inviteKeys(Uint8List.fromList(List.filled(32, 9)));
    const text = (link: 'lumit://join/abc', password: 'open sesame', project: 'Stack', look: false);
    final sealed = await sealInvite(text, (await mine.extractPublicKey()).bytes);

    expect(sealed, matches(RegExp(r'^[\w-]+$')));
    expect(await openInvite(sealed, mine), text);
    expect(await openInvite(sealed, other), isNull);
    // One changed character and it opens for nobody.
    final spoilt = sealed.replaceRange(60, 61, sealed[60] == 'A' ? 'B' : 'A');
    expect(await openInvite(spoilt, mine), isNull);
    expect(await openInvite('not an invite', mine), isNull);
  });

  test('a job sealed for a computer opens there, and only as a job', () async {
    final mine = await inviteKeys(Uint8List.fromList(List.filled(32, 7)));
    final other = await inviteKeys(Uint8List.fromList(List.filled(32, 9)));
    final job = utf8.encode('{"job":"5b0c","spec":{"codec":"h264"}}');
    final sealed = await sealBox(job, (await mine.extractPublicKey()).bytes,
        info: 'lumit computer v1');

    expect(await openBox(sealed, mine, info: 'lumit computer v1'), job);
    expect(await openBox(sealed, other, info: 'lumit computer v1'), isNull);
    expect(await openInvite(sealed, mine), isNull);
  });
}
