// The friends list as the server sends it, read into the three lists the
// Account page draws. A person the answer cannot name is left out rather
// than drawn as a blank row. And a link's owner, who is left out of the
// friends it can be sent to.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/account.dart';
import 'package:lumit_flutter/state/reviews.dart';

void main() {
  test('the friends answer reads into friends, asks and asked', () {
    final read = Friends.from({
      'friends': [
        {'username': 'ana', 'name': 'Ana'},
        {'name': 'Nobody'},
      ],
      'asked': [
        {'username': 'ben', 'name': 'Ben'},
      ],
      'asking': [
        {'username': 'cy'},
      ],
    });
    expect(read.friends, [(username: 'ana', name: 'Ana')]);
    expect(read.asked, [(username: 'ben', name: 'Ben')]);
    // No name: the username stands in for one.
    expect(read.asking, [(username: 'cy', name: 'cy')]);

    final empty = Friends.from({'error': 'offline'});
    expect([empty.friends, empty.asked, empty.asking], everyElement(isEmpty));
  });

  test('the account keeps its username', () {
    final account = CloudAccount.from(
        {'id': 1, 'email': 'a@b.c', 'name': 'Ana', 'username': 'ana'});
    expect(account.username, 'ana');
    expect(CloudAccount.from(account.toJson()).username, 'ana');
    expect(CloudAccount.from({'id': 1, 'email': '', 'name': ''}).username,
        isNull);
  });

  test("a link keeps its owner's username", () {
    expect(
        Review.from({
          'owner': {'username': 'ben'}
        }).ownerUsername,
        'ben');
    expect(Review.from({'owner': {'username': null}}).ownerUsername, isNull);
    expect(Review.from({}).ownerUsername, isNull);
  });
}
