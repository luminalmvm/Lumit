// What each setting in the Settings window is on a new install, by the key of
// the control that shows it. Read off a workspace that was never loaded, so a
// default changed where the setting lives is changed here too. The few the
// engine keeps are written as the engine ships them.

import 'settings.dart';
import 'workspace.dart';

/// The shipped value of each setting, by its control's key. A row whose key
/// isn't here has nothing to go back to: a button, a readout, or a value the
/// engine picks for the machine.
final Map<String, Object?> shippedSettings = () {
  final w = Workspace.shipped();
  final i = w.interface;
  final p = w.performance;
  return <String, Object?>{
    // General
    'settings-language': i.language,
    'settings-welcome-on-launch': w.showWelcomeOnLaunch,
    'settings-keep-running': w.keepRunning,
    'settings-open-at-sign-in': w.openAtSignIn,
    'settings-project-template': w.projectTemplate,
    'settings-auto-update': w.autoUpdate,
    'settings-nightly-icon': w.nightlyIcon,
    'settings-share-up-limit': w.shareUpLimit,
    'settings-share-down-limit': w.shareDownLimit,
    'settings-discord-status': w.discordStatus,
    'settings-live-endpoint': w.liveEndpoint,
    'settings-tangent-panels': w.tangentPanels,
    'settings-blender': w.blenderPath,
    'settings-scripts-folder': w.scriptsFolder,
    for (final event in ScriptEvent.values)
      'settings-event-script-${event.name}': w.eventScripts[event],
    // Appearance
    'settings-scheme': w.themeChoice,
    'settings-accent': w.accentOverride,
    'settings-themed-splash': w.themedSplash,
    'settings-shape': w.themeShape.name,
    'settings-reduce-transparency': i.reduceTransparency,
    'settings-label-case': i.labelCase,
    'settings-lantern-room': i.room,
    'settings-ui-scale': i.uiScale * 100,
    'settings-tooltips': i.showTooltips,
    'settings-animation': w.animationLevel,
    'settings-compact': i.compact,
    'settings-chrome-labels': i.chromeLabels,
    'settings-themed-scopes': w.themedScopes,
    'settings-themed-effect-graphs': w.themedEffectGraphs,
    'settings-align-first': i.alignToFirstPicked,
    'settings-themed-surround': w.themedViewerSurround,
    'settings-viewer-bars': i.viewerBars,
    'settings-tool-bar-position': i.toolBarPosition,
    'settings-range-sliders': i.rangeSliders,
    'settings-command-box': i.commandBox,
    'settings-icon-set': i.iconSet,
    'settings-project-thumbnails': i.projectThumbnails,
    'settings-layer-names-on-bars': i.layerNamesOnBars,
    'settings-marker-lines': i.markerLines,
    'settings-wire-style': i.wireStyle,
    'settings-multiwave': i.multiwaveWaveforms,
    'settings-waveform-from-bottom': i.waveformsFromBottom,
    'settings-waveform-scale': i.waveformScale,
    // Timeline
    'settings-retime-speed-lens': i.retimeOpensToSpeed,
    'settings-retime-in-seconds': i.retimeInSeconds,
    'settings-video-as-sequence': i.videoAsSequenceLayer,
    'settings-streams-arrival': i.streamsArrival,
    'settings-cut-sound-with-picture': i.cutSoundWithPicture,
    'settings-cut-tracks-any-order': i.cutTracksAnyOrder,
    'settings-paste-at-original-time': i.pasteLayersAtOriginalTime,
    'settings-playhead-stays': i.playheadStaysOnStop,
    'settings-new-expressions-language': w.defaultExpressionLanguage,
    'settings-transform-in-fx': i.transformInEffectControls,
    'settings-fx-key-strip': w.fxKeyStrip,
    'settings-easing-in-popup': i.easingInPopup,
    'settings-quantise-sound': i.quantiseSound,
    'settings-freeze-keeps-sound': i.cutFreezeKeepsSound,
    'settings-default-transition': i.cutDefaultTransition,
    'settings-right-click-node-search': i.rightClickOpensNodeSearch,
    'settings-tab-node-search': i.tabOpensNodeSearch,
    'settings-shift-a-node-search': i.shiftAOpensNodeSearch,
    // Viewer
    'settings-smooth-zoomed-viewer': w.smoothZoomedViewer,
    'settings-show-tone-map': i.showToneMap,
    'settings-invert-orbit-across': i.invertOrbitAcross,
    'settings-invert-orbit-up-down': i.invertOrbitUpDown,
    // Audio
    'settings-audio-device': w.audioDevice,
    'settings-audio-offset': w.audioOffsetMs,
    'settings-touch-return': w.touchReturnMs,
    'settings-scrub-on-drag': w.scrubOnDrag,
    'settings-normalise-target': w.normaliseTargetDb,
    // Autosave
    'settings-autosave-minutes': w.autosaveMinutes,
    'settings-autosave-keep': w.autosaveKeep,
    // Export: an unwritten store is blank and asks where to write.
    'settings-export-preset': '',
    'settings-export-template': '',
    'settings-export-destination': 'ask',
    'settings-notify-exports': w.notifyExports,
    // Preview and cache
    'settings-playback-mode': p.playback,
    'settings-full-res-drags': p.fullResDragPreviews,
    'settings-hardware-decode': p.hardwareDecode,
    'settings-plugins-on-card': true,
  };
}();
