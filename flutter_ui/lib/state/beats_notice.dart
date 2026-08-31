// The sentence a finished beat detection leaves on the status line — one
// function, because three surfaces run detection (the Audio panel's Generate,
// the Timeline's more menu, Composition ▸ Detect beats) and they must say the
// same thing.

import 'package:lumit_flutter/src/rust/api/beats.dart';

import '../l10n/strings.dart';

/// What a detection that placed markers says (the AudioWorkspace board's own
/// status caption): the confirmed tempo and the count, or the count alone when
/// no grid stood. The refusals and the empty run keep their own sentences at
/// each call site — they differ by what was asked, and this answer does not.
String beatsFoundNotice(BridgeBeatsResult found) => found.bpm > 0
    ? l10n.beatsGridConfirmed(found.bpm.toStringAsFixed(0), '${found.placed}')
    : l10n.beatsPlaced('${found.placed}');
