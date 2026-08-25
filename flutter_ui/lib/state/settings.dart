// Application-wide settings the Flutter frontend actually reads. What a setting
// *means* is the engine's business and is reached through the bridge; this file
// holds the working preferences of the interface itself, plus the handful of
// machine-local numbers the engine has nowhere to keep — the cache budgets are
// live engine state with no store behind them, so without a copy here they
// reset to the default on every launch. The keymap blob in `Workspace` is the
// same arrangement: the settings file ferries it, Rust decides what it does.

import 'package:lumit_flutter/src/rust/api/cache.dart';

/// Which of the two playback behaviours the Viewer uses (docs/13 §B5).
///
/// Mirrors the engine's `BridgePlaybackMode`. Held separately rather than using
/// that type directly so the settings file does not depend on generated code,
/// and so a value written by an older build still loads.
enum PlaybackMode {
  /// Keep time, lower the resolution.
  adaptive,

  /// Every frame at full resolution, cached, sound silenced.
  everyFrame,
}

/// The Viewer's working preferences (Settings → Performance). The shared
/// texture is the only frame transport (K-183), so there is no toggle for it.
class PerformanceSettings {
  /// Which playback behaviour the Viewer uses. Kept here rather than only in
  /// the Viewer's own state so the choice survives a restart — it is a working
  /// preference, not a per-session toggle.
  PlaybackMode playback;

  /// The rendered-frame cache budget in bytes, as the user last set it.
  ///
  /// Null means "whatever the engine defaults to" — the shipped default is the
  /// engine's business, and writing a copy of it out at first launch would
  /// freeze today's default into every settings file. Only a deliberate change
  /// puts a number here. The frontend never interprets it; it hands it back to
  /// the engine on the next launch exactly as it received it.
  int? cacheBudgetBytes;

  /// The graphics-card preview cache budget in bytes. Null as for
  /// [cacheBudgetBytes].
  int? vramBudgetBytes;

  /// The disk frame cache's budget in bytes. Null as for [cacheBudgetBytes].
  int? diskBudgetBytes;

  /// Where parked frames live, as the engine's own enum name
  /// (`appData` / `besideProject` / `custom`). Null means the engine's default.
  /// The frontend never interprets it beyond showing the choice — it hands the
  /// name back on the next launch.
  String? diskCacheLocation;

  /// The folder chosen for the `custom` location. Null when none has been
  /// picked, in which case the engine keeps its default rather than pointing
  /// the tier at nothing.
  String? diskCacheFolder;

  PerformanceSettings({
    this.playback = PlaybackMode.adaptive,
    this.cacheBudgetBytes,
    this.vramBudgetBytes,
    this.diskBudgetBytes,
    this.diskCacheLocation,
    this.diskCacheFolder,
  });

  Map<String, dynamic> toJson() => {
        'playback': playback.name,
        if (cacheBudgetBytes != null) 'cache_budget_bytes': cacheBudgetBytes,
        if (vramBudgetBytes != null) 'vram_budget_bytes': vramBudgetBytes,
        if (diskBudgetBytes != null) 'disk_budget_bytes': diskBudgetBytes,
        if (diskCacheLocation != null) 'disk_cache_location': diskCacheLocation,
        if (diskCacheFolder != null) 'disk_cache_folder': diskCacheFolder,
      };

  factory PerformanceSettings.fromJson(Map<String, dynamic> j) =>
      PerformanceSettings(
        // An unknown name (an older or newer build) falls back to adaptive,
        // which is the mode that always plays.
        playback: PlaybackMode.values.firstWhere(
          (m) => m.name == j['playback'],
          orElse: () => PlaybackMode.adaptive,
        ),
        // A value written by a build that stored something else entirely, or a
        // hand-edited file, must not stop the settings loading: anything that
        // is not a positive whole number is treated as absent.
        cacheBudgetBytes: _positiveInt(j['cache_budget_bytes']),
        vramBudgetBytes: _positiveInt(j['vram_budget_bytes']),
        diskBudgetBytes: _positiveInt(j['disk_budget_bytes']),
        diskCacheLocation: _nonEmpty(j['disk_cache_location']),
        diskCacheFolder: _nonEmpty(j['disk_cache_folder']),
      );
}

int? _positiveInt(Object? v) => v is int && v > 0 ? v : null;

String? _nonEmpty(Object? v) => v is String && v.isNotEmpty ? v : null;

/// The engine's cache-location enum from the name stored in the settings file.
///
/// By name, not by index: the settings file outlives any particular build, and
/// a reordered enum would otherwise silently move a user's cache to a different
/// folder. An unknown name — an older or newer build — falls back to the
/// application's own folder, which always works.
BridgeCacheLocation cacheLocationFromName(String name) =>
    BridgeCacheLocation.values.firstWhere(
      (l) => l.name == name,
      orElse: () => BridgeCacheLocation.appData,
    );

/// How the Viewer's chrome is laid out round the picture (K-448, K-466).
///
/// The approved drawing splits it: the magnification, the preview quality and
/// the colour pipeline in a header strip above the picture, everything else in
/// the bar below it. The other two gather the lot into one strip, for anyone
/// who would rather spend 22 pixels once than twice.
enum ViewerBars {
  /// The drawing's own: pickers above, everything else below.
  split,

  /// One strip, above the picture.
  top,

  /// One strip, below the picture.
  bottom,
}

/// What the chrome says: a word, or the glyph that stands for it (K-440).
///
/// In plain terms: every button, tab and toggle in Lumit has a word and a
/// drawing that mean the same thing. This chooses which of the two is shown.
/// A tooltip always carries the word, in every mode, so nothing is ever
/// unnameable — and content the user typed is never turned into a picture.
///
/// **Icons is the shipped default**, by the owner's ruling after desktop
/// testing; K-440 wrote Words. The first surface to read this setting is the
/// Timeline's Switches / Modes / Parent column toggles, and the owner's answer
/// on seeing them was that a row of three short words is a sentence to read
/// where three marks are a thing to aim at.
enum ChromeLabels {
  /// The word, everywhere.
  words,

  /// Buttons, tabs and toggles become glyphs; panel titles stay text.
  icons,

  /// Panel titles too.
  iconsEverywhere,
}

/// The presentation baseline the whole interface is drawn on top of (K-560).
///
/// In plain terms: the owner tested Lumit at 110% and ruled that size right, so
/// **what 110% showed is what 100% now draws**. The drawings stay authoritative
/// at their logical sizes (K-450) and no metric, manifest or mockup moves — the
/// interface is simply presented a tenth larger than it lays out, underneath
/// whatever factor the user sets for themselves.
const double uiScaleBaseline = 1.1;

/// What the interface is actually scaled by: the user's own factor over the
/// baseline. The Settings slider reads [InterfaceSettings.uiScale], the user's
/// half, so its percentages are against the new 100% rather than the old one.
double effectiveUiScale(double userScale) => userScale * uiScaleBaseline;

/// Interface (Settings → Interface): UI scale and tooltips (K-117), plus the
/// two editing preferences that make Lumit behave the Vegas way (K-246).
class InterfaceSettings {
  /// The user's own scale factor, 1.0 being the shipped size — *not* what the
  /// interface is scaled by, which is this over [uiScaleBaseline] (K-560).
  double uiScale;
  bool showTooltips;

  /// Whether the Effect controls panel repeats the layer's Transform rows
  /// above its effect stack.
  ///
  /// Off by default: the Timeline's fold-out already shows Transform, and
  /// the panel is for the *effects* on a layer — the repeat pushed the stack
  /// down a screen on a 3D layer. Kept as a choice because it is a habit
  /// After Effects users bring with them.
  bool transformInEffectControls;

  /// Whether a Retime channel opens in the graph editor showing playback speed
  /// rather than source position (K-246, realising K-075's preference).
  ///
  /// On, the channel opens to its Velocity lens and that lens is the **speed
  /// envelope** of K-247 — one point per key, whose height is the speed. Off,
  /// it opens to Time and the speed view keeps the ordinary two-sided
  /// derivative shape every other property has. Ordinary properties are
  /// unaffected either way; this is a Retime-only preference.
  bool retimeOpensToSpeed;

  /// Whether the Retime row shows its source position in **seconds** rather
  /// than as a timecode (K-287).
  ///
  /// Off by default: a Retime says which moment of the source is showing, and
  /// every other time in the editor says that as `HH:MM:SS:FF` — a lone
  /// decimal number of seconds meant doing arithmetic to line a retime up with
  /// anything else (K-075 asked for the timecode). On is for the people who
  /// think in seconds, and for the sub-frame precision a whole-frame clock
  /// face cannot show.
  bool retimeInSeconds;

  /// Whether video footage and image sequences added to a comp arrive as a
  /// one-clip Sequence layer rather than a Footage layer (K-246).
  ///
  /// Still images never do: there is nothing to cut in a single frame.
  bool videoAsSequenceLayer;

  /// Whether stopping playback leaves the playhead on the frame that was on
  /// screen, rather than putting it back where play started (K-254).
  ///
  /// Off by default: playback is a preview of the moment you are working on,
  /// and coming back to a different frame than you left means finding your
  /// place again after every space bar. On is the After Effects behaviour, and
  /// what Lumit did before this existed — hence the phrasing as a deviation
  /// from the default rather than a choice between two equals.
  bool playheadStaysOnStop;

  /// Whether a pasted layer keeps the time it was copied at, rather than
  /// starting at the playhead (K-275).
  ///
  /// Off by default: pasting is nearly always "put one here", and the playhead
  /// is where *here* is. On is for the other job — rebuilding the same moment
  /// in a second composition, where a layer that landed anywhere but its own
  /// timecode would have to be dragged back by hand every time. Effects ignore
  /// it either way: a copied animation is placed by its first keyframe.
  bool pasteLayersAtOriginalTime;

  /// Whether a waveform draws as the three-band **multiwave** stack rather
  /// than one plain wave (K-280).
  ///
  /// On by default: a single wave says how loud a moment is and nothing about
  /// what is in it, and a mastered track is one solid block whichever
  /// instrument is playing. The stack splits it into bass, middle and treble,
  /// so a kick and a hi-hat are told apart at a glance — which is what an edit
  /// is aimed at. Off gives the plain wave back, unchanged.
  bool multiwaveWaveforms;

  /// Whether a waveform stands on the floor of its row rather than being
  /// centred about silence (K-285).
  ///
  /// Off by default: centred is what the eye expects of a *wave*, and it is
  /// what Lumit has always drawn. On, each column is folded onto the baseline
  /// and reaches up by how far the signal swung either way — half of a
  /// centred wave is a mirror of the other half, so folding it spends the
  /// whole row's height on the half that carries the information. Applies to
  /// the single wave and the stack alike.
  bool waveformsFromBottom;

  /// Whether the Viewer bar carries its tone map button (K-314).
  ///
  /// Off by default: tone mapping is a preview-only way of reading a picture
  /// brighter than the screen can show, which most work never needs, and a
  /// button that changes what the Viewer shows is not one to leave lying about
  /// for people who will never want it. Off also *disengages* it — the
  /// effective look's tone map is false whatever a comp stored — so hiding the
  /// button can never strand an engaged look with nothing to turn it off.
  bool showToneMap;

  /// Whether the graph editor's **Easing…** button opens the shape editor as a
  /// popup over the footer, rather than docking the Easing panel (K-349).
  ///
  /// Off by default: a popup closes on any click outside it, and choosing
  /// different keyframes *is* a click outside — so one drawn shape could only
  /// ever be tried on the selection that was live when it opened, which is the
  /// opposite of what a reusable ease is for. On is for a small screen, or for
  /// anyone who would rather not spend a column on it: the same editor, opened
  /// and dismissed where the button is.
  bool easingInPopup;

  /// Whether a layer's name is written along its bar in the Timeline's lane
  /// area (K-514).
  ///
  /// **Off by default**, by the owner's explicit ruling after desktop testing.
  /// The approved mockups draw the name on every bar and the editor did the
  /// same; on a real comp the row of names turned the lane area into a second
  /// copy of the outline, which is already spelling out exactly those names a
  /// few pixels to the left. Turning it on gives the labels back, unchanged.
  bool layerNamesOnBars;

  /// Whether rows are drawn a pixel or two tighter than the approved mockups
  /// render them (K-454, `DensityTokens` in `theme/theme.dart`).
  ///
  /// Off by default, and that default is the point of the setting: the
  /// mockups' own room is what the editor should look like, and this is the
  /// escape hatch for someone working on a short screen who would rather have
  /// four more layers in view than the air around them. It changes nothing but
  /// heights — no colour, no size of type, nothing about what anything means.
  bool compact;

  /// How the Viewer's two strips are arranged round the picture (K-448,
  /// K-466).
  ///
  /// [ViewerBars.split] by default, because it is what the approved drawing
  /// draws: the three pickers in a header above the picture, the ways of
  /// looking and the transport in the bar below it. The other two gather
  /// everything into one strip at whichever end is asked for — the same
  /// controls in the same order, on one row instead of two.
  ViewerBars viewerBars;

  /// What the chrome says: words, or the icon set's glyphs (K-440).
  ///
  /// [ChromeLabels.icons] by default — see the enum for why that is not
  /// K-440's own Words. A settings file written before this field existed
  /// adopts it, deliberately: the ruling is about what the editor should look
  /// like, not about who asked first, and Words is one click away.
  ChromeLabels chromeLabels;

  /// The interface language, as a BCP-47 tag (`en`, `de`, `zh`), or null to
  /// follow whatever the machine is set to (K-303).
  ///
  /// Null by default and stored only once chosen, so a user who never opens the
  /// picker follows their operating system for ever — including after they
  /// change it — rather than being frozen into whatever language they happened
  /// to launch Lumit in the first time. A tag Lumit has no strings for resolves
  /// to English at load rather than refusing to open (see `l10n/strings.dart`).
  String? language;

  InterfaceSettings({
    this.language,
    this.chromeLabels = ChromeLabels.icons,
    this.uiScale = 1.0,
    this.showTooltips = true,
    this.transformInEffectControls = false,
    this.retimeOpensToSpeed = false,
    this.retimeInSeconds = false,
    this.videoAsSequenceLayer = false,
    this.playheadStaysOnStop = false,
    this.pasteLayersAtOriginalTime = false,
    this.multiwaveWaveforms = true,
    this.waveformsFromBottom = false,
    this.showToneMap = false,
    this.easingInPopup = false,
    this.layerNamesOnBars = false,
    this.compact = false,
    this.viewerBars = ViewerBars.split,
  });

  Map<String, dynamic> toJson() => {
        if (language != null) 'language': language,
        'chrome_labels': chromeLabels.name,
        // The user's factor under its own key: the old `ui_scale` held the
        // whole scale, and reading one as the other would resize every
        // interface written before K-560 by a tenth. See `fromJson`.
        'ui_scale_user': uiScale,
        'show_tooltips': showTooltips,
        'transform_in_effect_controls': transformInEffectControls,
        'retime_opens_to_speed': retimeOpensToSpeed,
        'retime_in_seconds': retimeInSeconds,
        'video_as_sequence_layer': videoAsSequenceLayer,
        'playhead_stays_on_stop': playheadStaysOnStop,
        'paste_layers_at_original_time': pasteLayersAtOriginalTime,
        'multiwave_waveforms': multiwaveWaveforms,
        'waveforms_from_bottom': waveformsFromBottom,
        'show_tone_map': showToneMap,
        'easing_in_popup': easingInPopup,
        'layer_names_on_bars': layerNamesOnBars,
        'compact': compact,
        'viewer_bars': viewerBars.name,
      };
  factory InterfaceSettings.fromJson(Map<String, dynamic> j) =>
      InterfaceSettings(
        // Absent means "follow the machine", which is what every settings file
        // written before this field existed was doing.
        language: j['language'] as String?,
        // By name, not by index: a settings file outlives any build, and a
        // reordered enum would otherwise silently change what the chrome
        // says. An unknown name — an older file that has none, or a newer
        // build's — is the shipped default.
        chromeLabels: ChromeLabels.values.firstWhere(
          (c) => c.name == j['chrome_labels'],
          orElse: () => ChromeLabels.icons,
        ),
        // K-560's one migration, and it is a key rename rather than a file
        // version because it is one number: `ui_scale` was the whole scale,
        // `ui_scale_user` is the user's half of it. A file written before the
        // rebase divides by the baseline once — so an interface that was at
        // 100% stays exactly the size it was, at a user factor of about 91% —
        // and because the old key is never written again it can never divide
        // twice. Neither key means a new user: 1.0, the shipped size.
        uiScale: (j['ui_scale_user'] as num?)?.toDouble() ??
            ((j['ui_scale'] as num?)?.toDouble() ?? uiScaleBaseline) /
                uiScaleBaseline,
        showTooltips: j['show_tooltips'] as bool? ?? true,
        transformInEffectControls:
            j['transform_in_effect_controls'] as bool? ?? false,
        // Absent means off, which is the After Effects behaviour Lumit had
        // before these existed — a settings file written by an older build
        // must not silently change how the editor works.
        retimeOpensToSpeed: j['retime_opens_to_speed'] as bool? ?? false,
        // Absent means off: the timecode readout is the new default, so a
        // settings file written before this field existed adopts it.
        retimeInSeconds: j['retime_in_seconds'] as bool? ?? false,
        videoAsSequenceLayer: j['video_as_sequence_layer'] as bool? ?? false,
        // Absent means off here too, but for the opposite reason: the returning
        // playhead is the *new* default (K-254), so a settings file written
        // before this field existed adopts it rather than being pinned to the
        // old behaviour by its own silence.
        playheadStaysOnStop: j['playhead_stays_on_stop'] as bool? ?? false,
        // Absent means off: a paste lands at the playhead, which is what a
        // settings file written before this field existed already did.
        pasteLayersAtOriginalTime:
            j['paste_layers_at_original_time'] as bool? ?? false,
        // Absent means on: the multiwave stack is the new default (K-280),
        // and a settings file written before this field existed should get
        // the better picture rather than be pinned to the old one.
        multiwaveWaveforms: j['multiwave_waveforms'] as bool? ?? true,
        // Absent means off: centred is what a settings file written before
        // this field existed was already drawing.
        waveformsFromBottom: j['waveforms_from_bottom'] as bool? ?? false,
        // Absent means off: hidden is the new default, and a settings file
        // written before this field existed adopts it — a comp that stored an
        // engaged tone map is disengaged with it rather than left stranded.
        showToneMap: j['show_tone_map'] as bool? ?? false,
        // Absent means off: the panel is the default (K-349), and the popup
        // this replaced never shipped in a release, so no settings file can be
        // asking for it by silence.
        easingInPopup: j['easing_in_popup'] as bool? ?? false,
        // Absent means off (K-514). Every settings file written before this
        // field existed was written by a build that always drew the names, so
        // those users lose them — deliberately, because off is the owner's
        // ruling on what the editor should look like, and the labels are one
        // click away for anyone who wants them back.
        layerNamesOnBars: j['layer_names_on_bars'] as bool? ?? false,
        // Absent means off, which is the roomy default — and every settings
        // file written before this field existed was written by a build that
        // drew the tight rows. Those users get the extra pixel or two back,
        // deliberately: the mockups' room is the decision (K-454), and the
        // tight set is now something to ask for rather than something to
        // inherit by silence.
        compact: j['compact'] as bool? ?? false,
        // By name, not by index: a settings file outlives any build, and a
        // reordered enum would otherwise silently rearrange the Viewer. An
        // unknown name — an older file that has none, or a newer build's — is
        // the split the drawing draws.
        viewerBars: ViewerBars.values.firstWhere(
          (b) => b.name == j['viewer_bars'],
          orElse: () => ViewerBars.split,
        ),
      );
}
