// House controls, owned rather than Material (docs/archive/flutter-port/04): every
// colour and metric reads the theme, idle widgets are borderless, hover and
// press bring an edge back (the K-084 owner amendment).
//
// The kit outgrew one file, so it is a folder now — one part per widget family,
// each under the readability rule's thousand lines (K-007). This file is the
// barrel: everything the kit ever exported is still reached by importing
// `widgets/controls.dart`, so no call site anywhere had to change.

export 'controls/base.dart';
export 'controls/buttons.dart';
export 'controls/dropdowns.dart';
export 'controls/indicators.dart';
export 'controls/menus.dart';
export 'controls/modal_window.dart';
export 'controls/popups.dart';
export 'controls/slider.dart';
export 'controls/text_field.dart';
export 'controls/value_arithmetic.dart';
export 'controls/value_field.dart';
