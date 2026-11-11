// The two windows the Graphics template panel's foot opens.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';

import 'template_data_window.dart';
import 'template_save_dialog.dart';

/// Open the Data window for the graphics template of [comp].
void showTemplateDataWindow(BuildContext context, CompositionReference comp) =>
    unawaited(showTemplateDataWindowFor(context, comp));

/// Open the Save graphics template dialogue for [comp].
void showSaveTemplateDialog(BuildContext context, CompositionReference comp) =>
    unawaited(saveGraphicsTemplate(context, comp));
