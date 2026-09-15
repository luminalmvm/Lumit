// The one place the addons page speaks to the engine.
//
// `addons.dart` holds the catalogue, the download and the state the Settings
// page draws, all in plain Dart so it can be tested with no engine at all. This
// file is the other half: it turns the bridge's own types into that file's, and
// it is the only file that names them. Everything the engine can be asked about
// an addon goes through here.

import 'package:lumit_flutter/src/rust/api/addons.dart';

import 'addons.dart';

/// The service as the application runs it, with every collaborator wired to the
/// engine. A test builds an [AddonService] directly instead.
AddonService createAddonService() => AddonService(
      list: _list,
      install: _install,
      remove: _remove,
      runtime: _runtime,
      loadRuntime: _load,
      folder: addonsDir,
    );

List<Addon> _list() => [for (final addon in addonList()) _addon(addon)];

Future<void> _install(String manifest, List<String> files) =>
    addonInstall(manifest: manifest, files: files);

void _remove(String id) => addonRemove(id: id);

RuntimeStatus _runtime() => _status(addonRuntime());

Future<RuntimeStatus> _load() async => _status(await addonRuntimeLoad());

Addon _addon(BridgeAddon addon) => Addon(
      id: addon.id,
      kind: addon.kind == BridgeAddonKind.runtime
          ? AddonKind.runtime
          : AddonKind.model,
      name: addon.name,
      version: addon.version,
      summary: addon.summary,
      licence: addon.licence,
      licenceUrl: addon.licenceUrl,
      sizeBytes: addon.sizeBytes.toInt(),
      task: addon.task,
      broken: addon.broken,
    );

RuntimeStatus _status(BridgeRuntimeStatus status) => RuntimeStatus(
      state: switch (status.state) {
        BridgeRuntimeState.missing => RuntimeState.missing,
        BridgeRuntimeState.present => RuntimeState.present,
        BridgeRuntimeState.loaded => RuntimeState.loaded,
        BridgeRuntimeState.failed => RuntimeState.failed,
      },
      provider: status.provider,
      version: status.version,
      detail: status.detail,
    );
