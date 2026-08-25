import 'dart:async';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:provider/provider.dart';

class ProjectItemBuilder extends StatefulWidget {
  const ProjectItemBuilder(
      {required this.item, required this.builder, super.key});

  final ItemReference item;
  final Widget Function(BuildContext context) builder;

  @override
  State<ProjectItemBuilder> createState() => _ProjectItemBuilderState();
}

class _ProjectItemBuilderState extends State<ProjectItemBuilder> {
  StreamSubscription? sub;

  @override
  void initState() {
    final store = Provider.of<LumitState>(context, listen: false);
    sub = store.onChange.listen(onChange);
    super.initState();
  }

  @override
  void dispose() {
    sub?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return RepaintBoundary(child: widget.builder(context));
  }

  void onChange(ScopedChange event) {
    // The scope of this change is below this item, dont rebuild
    if (event.layer != null) return;

    if (event.item?.equals(item: widget.item) == true) {
      setState(() {});
    }
  }
}
