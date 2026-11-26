
import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';

import 'template_data_window.dart';
import 'template_save_dialog.dart';

void showTemplateDataWindow(BuildContext context, CompositionReference comp) =>
    unawaited(showTemplateDataWindowFor(context, comp));

void showSaveTemplateDialog(BuildContext context, CompositionReference comp) =>
    unawaited(saveGraphicsTemplate(context, comp));
