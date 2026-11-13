// The kinds an overlap can be, asked of the engine once, since a panel's build
// reads them and the list never changes.

import 'package:lumit_flutter/l10n/engine_labels.dart';
import 'package:lumit_flutter/src/rust/api/clip_edits.dart';

final List<BridgeTransitionKind> transitionKinds = cutTransitionKinds();

/// What a kind is called: its one look's name, or the effect's.
String kindLabel(BridgeTransitionKind kind) =>
    engineLabel(kind.looks.length == 1 ? kind.looks.first : kind.label);
