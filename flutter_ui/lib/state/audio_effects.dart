// Which effect-stack entries are audio plugins (AP5, K-700/K-707).
//
// The match name's own prefix carries the answer — the engine mints
// `clap:<plugin id>` and `vst3:<class id>` and spells the prefixes once in
// `lumit-core` — so no bridge call is needed to sort a stack into rack and
// picture, which is what lets the Mixer's chain chip and the Effect controls'
// Audio group read the held model in a rebuild.

/// Whether a stack entry's match name names an audio plugin.
bool isAudioEffectName(String name) =>
    name.startsWith('clap:') || name.startsWith('vst3:');
