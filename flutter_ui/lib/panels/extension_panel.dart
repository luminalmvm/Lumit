// The panel an extension's page is shown in.
//
// One pane per installed extension, told apart by the pane's number. The
// page is a web page from the extension's own folder, drawn by the system's
// web view (extension_page.dart has one for each platform), and everything
// it asks of Lumit goes through an [ExtensionHost].

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/extensions.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../shell/share_dialog_frb.dart';
import '../state/extension_host.dart';
import '../state/file_dialogs.dart';
import '../widgets/controls.dart';
import 'extension_page.dart';
import 'placeholder.dart';

class ExtensionPanel extends StatelessWidget {
  /// The pane's number, which is how the extension it shows is known.
  final int slot;

  const ExtensionPanel({super.key, required this.slot});

  @override
  Widget build(BuildContext context) {
    final service = context.read<LumitUiState>().extensions;
    return ListenableBuilder(
      listenable: service,
      builder: (context, _) {
        final extension = service.inSlot(slot);
        if (extension == null) {
          return PlaceholderPanel(
            icon: LumitIcon.link,
            title: l10n.extensionMissingTitle,
            hint: l10n.extensionMissingHint,
          );
        }
        if (extension.broken) {
          return PlaceholderPanel(
            icon: LumitIcon.link,
            title: l10n.extensionFailedTitle,
            hint: l10n.extensionBrokenHint,
          );
        }
        return ExtensionView(
          // A new page for an extension installed over itself.
          key: ValueKey<String>('${extension.id}:${service.generation}'),
          extension: extension,
        );
      },
    );
  }
}

/// One extension's page, and the [ExtensionHost] that answers it.
class ExtensionView extends StatefulWidget {
  final BridgeExtension extension;

  const ExtensionView({super.key, required this.extension});

  @override
  State<ExtensionView> createState() => _ExtensionViewState();
}

class _ExtensionViewState extends State<ExtensionView>
    implements ExtensionSurface {
  late final ExtensionHost _host;
  late final ExtensionPageLink _link;

  late final String _origin = extensionOrigin(widget.extension.id);

  /// Why it is not, when it could not be put up.
  String? _failure;

  /// The view has gone to a page that is not the extension's own. Nothing
  /// that page asks is answered.
  bool _strayed = false;

  @override
  void initState() {
    super.initState();
    final ui = context.read<LumitUiState>();
    final id = widget.extension.id;
    final folders = ui.extensions.folders(id);
    _host = ExtensionHost(
      extension: widget.extension,
      app: context.read<LumitState>(),
      surface: this,
      approvedFolders: folders,
      onFoldersChanged: () => ui.extensions.setFolders(id, folders),
    );
    _link = ExtensionPageLink(
      onMessage: (message) {
        if (!_strayed) unawaited(_host.handle(message));
      },
      onUrl: (url) {
        final strayed = !url.startsWith('$_origin/');
        if (strayed != _strayed && mounted) setState(() => _strayed = strayed);
      },
      onReady: () {
        if (!mounted) return;
        _host.post = (message) {
          if (!_strayed) _link.post?.call(message);
        };
        _host.start();
      },
      onFailed: (why) {
        if (mounted) setState(() => _failure = why);
      },
    );
  }

  @override
  void dispose() {
    _host.dispose();
    super.dispose();
  }

  @override
  void openShare({String? invite, String? relay}) {
    if (!mounted) return;
    showShareFrb(context, context.read<LumitState>(),
        invite: invite, relay: relay);
  }

  @override
  Future<String?> chooseFolder() => pickFolder();

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    if (_failure case final why?) {
      return PlaceholderPanel(
        icon: LumitIcon.link,
        title: l10n.extensionFailedTitle,
        hint: why,
      );
    }
    return Column(
      children: [
        if (_strayed)
          Container(
            color: t.surface2,
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
            child: Row(
              children: [
                Expanded(
                  child: Text(l10n.extensionStrayed,
                      style: t.small.copyWith(color: t.warning)),
                ),
                HouseButton(
                  key: const ValueKey('extension-back'),
                  small: true,
                  onPressed: () => _link.load
                      ?.call('$_origin/${widget.extension.entry}'),
                  child: Text(l10n.extensionBack, style: t.small),
                ),
              ],
            ),
          ),
        Expanded(
          child: ExtensionPage(extension: widget.extension, link: _link),
        ),
      ],
    );
  }
}
