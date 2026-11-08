// The kinds an overlap can be, asked of the engine once, since a panel's build
// reads them and the list never changes.

import 'package:lumit_flutter/src/rust/api/clip_edits.dart';

final List<BridgeTransitionKind> transitionKinds = cutTransitionKinds();
