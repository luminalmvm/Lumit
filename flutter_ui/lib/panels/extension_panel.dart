// The panel an extension's page is shown in.
//
// One pane per installed extension, told apart by the pane's number. The
// page is a web page from the extension's own folder, drawn by the system's
// web view, and everything it asks of Lumit goes through an [ExtensionHost].
// On Windows that view is WebView2. Elsewhere there is none yet, and the
// panel says so.

import 'dart:async';
import 'dart:convert';
import 'dart:io' show Platform;

import 'package:flutter/services.dart' show PlatformException;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/extensions.dart';
import 'package:provider/provider.dart';
import 'package:webview_windows/webview_windows.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../shell/share_dialog_frb.dart';
import '../state/extension_host.dart';
import '../state/file_dialogs.dart';
import '../widgets/controls.dart';
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
        if (!Platform.isWindows) {
          return PlaceholderPanel(
            icon: LumitIcon.link,
            title: l10n.extensionUnsupportedTitle,
            hint: l10n.extensionUnsupportedHint,
          );
        }
        return _ExtensionView(
          // A new page for an extension installed over itself.
          key: ValueKey<String>('${extension.id}:${service.generation}'),
          extension: extension,
        );
      },
    );
  }
}

/// The one web view environment every extension's page shares, made the
/// first time a panel needs it. Each page still keeps its own storage: they
/// are served under different names.
Future<void>? _environment;

Future<void> _ensureEnvironment() => _environment ??= () async {
      try {
        await WebviewController.initializeEnvironment(
            userDataPath: extensionDataDir(id: 'webview'));
      } on PlatformException {
        // Made already, by a panel before a hot restart.
      }
    }();

class _ExtensionView extends StatefulWidget {
  final BridgeExtension extension;

  const _ExtensionView({super.key, required this.extension});

  @override
  State<_ExtensionView> createState() => _ExtensionViewState();
}

class _ExtensionViewState extends State<_ExtensionView>
    implements ExtensionSurface {
  final WebviewController _view = WebviewController();
  late final ExtensionHost _host;
  final List<StreamSubscription<Object?>> _listening = [];

  late final String _origin = extensionOrigin(widget.extension.id);

  /// The page is up and can be drawn.
  bool _ready = false;

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
    unawaited(_open());
  }

  Future<void> _open() async {
    try {
      if (await WebviewController.getWebViewVersion() == null) {
        return _fail(l10n.extensionNoRuntime);
      }
      await _ensureEnvironment();
      await _view.initialize();
      if (!mounted) return;
      // No window of its own, and its files are its own: a page from
      // anywhere else cannot load them.
      await _view.setPopupWindowPolicy(WebviewPopupWindowPolicy.deny);
      await _view.addVirtualHostNameMapping(_origin.substring(8),
          widget.extension.folder, WebviewHostResourceAccessKind.deny);
      await _view.addScriptToExecuteOnDocumentCreated(extensionBootstrapScript);
      _listening.add(_view.url.listen((url) {
        final strayed = !url.startsWith('$_origin/');
        if (strayed != _strayed && mounted) setState(() => _strayed = strayed);
      }));
      _listening.add(_view.webMessage.listen((message) {
        if (!_strayed) unawaited(_host.handle(message));
      }, onError: (_) {}));
      _host.post = (message) {
        if (!_strayed) unawaited(_view.postWebMessage(jsonEncode(message)));
      };
      await _view.loadUrl('$_origin/${widget.extension.entry}');
      if (!mounted) return;
      _host.start();
      setState(() => _ready = true);
    } catch (_) {
      _fail(l10n.extensionFailedHint);
    }
  }

  void _fail(String why) {
    if (mounted) setState(() => _failure = why);
  }

  @override
  void dispose() {
    for (final subscription in _listening) {
      subscription.cancel();
    }
    _host.dispose();
    _view.dispose();
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
    if (!_ready) return ColoredBox(color: t.surface1);
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
                  onPressed: () => unawaited(
                      _view.loadUrl('$_origin/${widget.extension.entry}')),
                  child: Text(l10n.extensionBack, style: t.small),
                ),
              ],
            ),
          ),
        Expanded(child: Webview(_view)),
      ],
    );
  }
}
