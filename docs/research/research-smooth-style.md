# Research: the smooth / MVM editing style (Luminal, stooh, starker)

Researched 2026-07-12 for the Kiriko design docs. Sources: the three channels' full video
descriptions (via YouTube RSS feeds), their Payhip stores, CODMVM, Effects Collective,
HLAE/advancedfx docs, community tutorials. Direct quotes are from video descriptions.

---

## 1. What "MVM" means, and what this style is

- **MVM = "movie making" / "music video maker"** — the term comes from the Call of Duty
  movie-making mod lineage (**IW3MVM** for CoD4, T6MVM for BO2: demo-theatre mods with
  free dollycam, greenscreen passes, depth display, fog control, FOV/roll control, and
  multi-pass AVI export). The community hub is **CODMVM (codmvm.com)** — "an index for a
  large part of the Call of Duty, Counter-Strike and other editing communities",
  archiving **237,635 videos**, with an active Discord (~3.1k members) distributing mods,
  demos and support. Editors adopt the suffix as identity — see e.g. the reference
  channel `@Luminalmvm`.
- Urban Dictionary (community-written) draws the key line: *"Movie Making aka Film making
  in terms of editing video game footage. Note: mvm is not the same as an 'edit'… Mvm
  are longer, in general require more work and usually has a story behind it."* In
  practice the scene's works are called **"edits"** (1–3 min, one song, one game) and the
  people are "editors"; both stand apart from **montages/fragmovies** (kill-counting
  highlight reels).
- **How it differs from a classic kill montage.** A montage is clip-driven: the frags are
  the content, cuts land on kills, hype pacing, energetic text callouts. An MVM-style
  edit is **music- and mood-driven**: gameplay is raw material for a short music video.
  Kills matter less than **camera work, compositing, atmosphere and story**. Luminal's
  Cronus is explicit about this: started Dec 2020, finished after 2 years, with "the
  'story' I'd had in mind since atlas". Edits are "accepted" by curated editing
  communities/teams the way AMVs enter studios — Luminal notes "Evamedia: Accepted";
  stooh tags `#VenexaArts #ForgetAboutFreeman` (Venexa Arts is a Steam-group editing team
  with application standards; edits over 15s preferred).
- The adjacent tutorial scene calls the same look the **"flow style"** — e.g. jayd3nfx's
  "How to ACTUALLY EDIT in The FLOW Style" series (112k views): "how to transition two
  clips that don't flow, but we force it to flow (simple CINEMATIC transition)". So the
  verified characterisation: **smoother, cinematic, AMV-adjacent music-video editing of
  game footage — confirmed.** Aesthetic references are anime and AMV culture throughout
  (Luminal: "worn out weeb", RADWIMPS/Your Name songs, Danganronpa, JoJo "Great Days",
  K-pop; stooh: vocaloid/Hatsune Miku work).

## 2. The three editors

### Luminal (@Luminalmvm) — reference channel one. 2.91k subs, 18 videos
- CS:GO / BO2 / Apex / Overwatch edits. Flagships: **Atlas** (CS:GO, 58.9k views, 2 months'
  work), **Cronus** (CS:GO, 25.8k views, 2 years elapsed), **What is love?** (CS:GO, 51.6k),
  **What About You** (BO2, 43.8k), **Dream Lantern** (BO2, 4.5k, "my best edit to date").
- Publishes **effect breakdown videos** (Cronus breakdown: 14 chaptered scene deconstructions
  — whiteboard scene, POVs, fire scenes, HE frag scene, portal scenes) and **two AE
  tutorials**: *Advanced Motion Blur* (Force Motion Blur vs RSMB vs Sapphire
  `s_OCIOTransform` linear-workspace trick; render time vs realism trade-offs) and
  *Advanced Depth and Particular* (Trapcode Particular emitted onto depth passes "for
  better compositing", plus normalising 3D camera tracks).
- Sells **project files on Payhip** ($4–10; Cronus includes "all effects and settings…
  including the color correction") and shares **full clip packages: "World, Greenscreen,
  Depth and Camera Paths"** plus the raw demos — i.e. the complete HLAE multi-pass
  pipeline (see §4).
- Credits reveal the ecosystem: "Fad, for creating a plugin that saved my life" (community
  plugin devs), "Tira, chatting about superior motion blur methods", collaborative "ops"
  (opinions/feedback) culture, named inspirations **Wh1teend, Nobunaga, Janezi, Cherry**
  (canonical smooth-style editors).

### stooh (@stooh) — 70k-view peak, motion-graphics-leaning
- Channel bio "forget about freeman" (Half-Life; also an editing-community tag). Flagships:
  **Ängel** (70.4k views), **senses** (28.9k), **kaleido rush** (15.1k), **kazefuku** ft.
  letnes (7.3k), plus Reflex Arena, VR found-footage experiments, and (2025–26) pure
  **motion graphics / UI animation / vocaloid character rigging** (`#aftereffects
  #motiongraphics #ui`, "illustration + rigging + animation practice").
- Sells project files + clips per edit and an **"Editing Pack 2024"** (€10, 587MB):
  11 AE project files, **37 AE presets, 23 colour-grading presets, 19 textures,
  13 CS:GO configs, 6 COD4 configs, 1 Source config**, 8 logos. Requirements name the
  plugin stack outright: **Sapphire, FilmConvert, RSMB, Uni(verse), BCC, MBL, Deep Glow,
  Signal** (AE 2020+).
- Trajectory matters for Kiriko: the ceiling of this style is indistinguishable from
  motion design — game edits and mograph are one continuum for these users.

### starker (@starkerr) — 8.54k subs, 75 videos; the "went pro" path
- **Aziz, 24, motion designer from Saudi Arabia** (linktr.ee/starker; Behance
  behance.net/azizvisuals). Classic-era edits: **Melody** (67k), **Helmir** (64k),
  **lovely** (34k), **Drift – Fortnite Edit** (29k), plus CoD4/BO2 work — his store sells
  each **project file** ($3–8, 2018/2019 bundles $20) *and the recording configs*:
  "BO2 - CFG", "BO2/COD4 - CFGs", "'Visions' THEME + COD4 Inspector settings".
- Graduated to **FaZe Clan client work**: Valorant/Halo/Rocket League/CS:GO team
  announcements, FaZe1 campaign, McDonald's x FaZe promo, ATL FaZe Champs promo
  ("Directing, Editing, and Compositing"), with credited sub-teams (3D artists, "frag
  editing", SFX, VO) and his own **BREAKDOWN: FaZe1** video. Esports org promo is the
  commercial endpoint of this exact skillset.

**Shared commercial pattern:** all three monetise via Payhip **project files, preset/CC
packs and recording configs** — the scene learns by opening each other's timelines. A
tool that can't open/share/remix full projects (with effects, expressions and colour
intact) breaks the scene's core learning loop.

## 3. Defining techniques, ranked by importance

1. **Velocity / time ramping ("smooth twixtor", flow ramps).** The signature. Footage
   recorded or interpolated to high fps, then speed-ramped with eased keyframes so action
   pours in and out of slow motion in time with the music. Tools: **Twixtor** (optical
   flow; the community also uses **Flowframes (RIFE AI)** — per Effects Collective,
   editors "run their footage through it before editing to result in a smoother look",
   i.e. RIFE as a pre-pass, Twixtor/time-remap for the in-AE ramp; **Twixtor Assistor**
   scripts build the precomp structure automatically). CS:GO/CoD demos sidestep some of
   this: demo playback speed is controlled at record time (host_timescale/mirv), so slowmo
   can be rendered natively at any fps.
2. **Synthetic 3D camera on gameplay.** Two forms: (a) **in-engine camera paths** —
   HLAE/IW3MVM dollycams flown through frozen or slowed demos, exportable as **camera
   path data imported into AE's 3D camera** so composited layers stick to the world;
   (b) **faux-3D in AE** — 2.5D parallax from depth passes, camera-tracked POVs
   ("Normalise your 3D Camera Tracks" is a Luminal-referenced tutorial). This is the
   biggest visual separator from montage editing.
3. **Multi-pass compositing (greenscreen + depth + world).** Clips ship as separate
   passes: world, character matte ("greenscreen"), **depth**. Uses: text/particles
   *behind* the player, depth-of-field, fog, depth-driven particle fields (Luminal's
   Advanced Depth + Particular tutorial), selective grades. Where no engine pass exists
   (Fortnite, Apex, Overwatch), editors **mask/rotoscope by hand** — heavy manual roto is
   normal in this scene.
4. **Seamless "flow" transitions.** Whip/zoom/warp transitions, masked object wipes
   (character or foreground geometry wipes the frame), portal/morph gags (Cronus's "Hand
   Portal Scene", "Mirage Tunnel Scene"), match-cut momentum carried across scenes. Built
   from precomps + motion tile/replicate edge stretching + directional blur + hand-drawn
   masks. Sold transition packs (Handy Seamless Transitions) exist but top editors build
   their own.
5. **Motion blur as craft.** Not a checkbox: Luminal made a whole tutorial comparing
   **Force Motion Blur, RSMB (vector), pixel motion blur and Sapphire in linear/OCIO
   workspace**, explicitly trading render time vs realism (shutter angle tuning, DoF
   interaction). "Superior motion blur methods" is literally a topic of friendly debate.
6. **Camera shake with character.** Smooth handheld drift and micro-jitter layered over
   moves (wiggle/Twitch/shake presets), plus hard directional **impact shakes** on
   accents. Shake is an *animation-curve* product, not a canned overlay — it must
   composite with the 3D camera.
7. **Colour grading + film emulation.** A whole-video "CC" (colour correct) look is part
   of an editor's identity: teal/bleach or warm anime-ish grades, **FilmConvert**, Magic
   Bullet/Looks, LUT-ish adjustment-layer stacks. stooh ships **23 colour presets**;
   Luminal's project file sells partly *because* "including the color correction".
8. **Glow, light and atmosphere.** **Deep Glow** (the scene's default pretty-glow),
   Sapphire glows/light rays, Optical Flares, fog (mvm_fog), **Trapcode Particular** dust
   and ambient particle fields on depth layers.
9. **Overlays, textures, grain.** Dust/dirt textures, film grain, borders ("cinematic
   borders" via NESLayers), light leaks, RGB-split/chromatic aberration; stooh's pack
   ships **19 textures**; the flow-tutorial ecosystem trades overlay packs (haz,
   akira-jpeg). stooh 2025: "I'm bringing rgb back."
10. **Typography and motion graphics.** Restrained, kinetic type (song lyric moments,
    scene labels) and — at the ceiling — full **UI/HUD motion graphics** (stooh's `#ui`
    pieces, starker's FaZe announcement mograph, decal design). Text-behind-subject via
    the matte passes.
11. **Sync philosophy: flow over hard hits.** Sync to the *phrase and energy* of the
    music — ramps swell with builds, camera lands with downbeats — rather than one-cut-
    per-beat. Impacts still hit (HE frag on the drop), but between hits everything glides;
    "force it to flow" is the tutorial-scene mantra. Song choice is emotional/melodic
    (M83, RADWIMPS, Johnny Goth, K-pop, vocaloid) rather than trap/dubstep hype.
12. **SFX design layer.** Whooshes, risers, impacts, ambience under the song — Luminal
    repeatedly self-criticises his SFX and released "SFX only" versions of edits; scene
    treats sound design as a distinct discipline worth crediting (starker credits
    dedicated SFX people on FaZe projects).

**Recording practice:** demo/replay-based games (CS, CoD4/BO2 via HLAE/IW3MVM/T6MVM)
are re-recorded from demos at controlled timescale, high fps, multi-stream (world/matte/
depth), with tuned **game configs** (fov, filmtweaks, fog, viewmodel) — hence configs are
sellable products. Non-demo games (Fortnite/Apex/OW) rely on high-fps capture + replay
modes + AI interpolation (Flowframes/RIFE) + hand roto. Luminal's "Memories" even applies
the style to iPhone holiday footage — the grammar transfers to IRL video.

## 4. What this style demands from a tool (vs classic montage editing)

Classic montage: cut accuracy, beat markers, speed ramps, shake/impact presets, text pops
— mostly *editing*. This style is *compositing + animation*:

1. **Graph-editor ergonomics as a first-class feature.** Everything above is eased
   keyframes: ramps, cameras, shakes, transitions. Speed/value graph quality, easy
   copy/paste of eases (community script **EasyCopy** exists purely to copy eases without
   overwriting values) — smooth interpolation quality IS the style's name.
2. **Real 3D camera + 2.5D compositing**, including **importing game camera paths**
   (HLAE export) and depth-pass-aware effects (DoF, fog, particles-on-depth, depth mattes).
3. **Heavy masking/rotoscoping tooling** — per-frame character roto for games without
   matte passes; matte-pass workflows (track matte ergonomics) for games with them.
   Text-behind-subject and object-wipe transitions live or die here.
4. **High-quality retiming built in**: optical-flow + AI (RIFE-class) interpolation
   native, with artifact handling — replacing the Twixtor + Flowframes + Assistor-script
   chain that currently spans three tools.
5. **Programmable/craft motion blur** (vector-based, shutter control, applies to
   retimed footage and synthetic camera moves) — not just a layer switch.
6. **Transition building via nesting/precomps** — reusable, parameterised seamless
   transitions; anything that makes "precomp + edge-stretch + directional blur + mask"
   a composable unit wins.
7. **Particles, glow, grain, film-emulation colour** approaching Particular/Deep Glow/
   FilmConvert quality, because the third-party plugin stack (Sapphire, BCC, RSMB, Deep
   Glow, Universe, FilmConvert, Signal, MBL) is where most of this style's money and
   crashes live.
8. **Preset/pack + project-file interchange.** The scene's economy and pedagogy are
   Payhip packs (presets, CCs, textures, project files). Shareable, openable, remixable
   project files are a growth loop, not a nice-to-have.
9. **Audio-aware timeline** — waveform-driven keyframing (beat/energy markers, audio-to-
   keyframe) supporting *flow* sync, not just beat-snap cutting.

## 5. AE pain points for this style (evidence-backed)

- **Project scale collapse.** Luminal on Atlas: "towards the end of making this the
  project file got so large **after effects was almost unusable**, so towards the end it
  feels pretty rushed". Two-minute edits take months partly because AE drags.
- **Render-time vs quality blackmail on motion blur/retiming.** An entire tutorial
  exists on choosing blur methods by render cost; Force Motion Blur/Twixtor stacks are
  notoriously slow; linear-workspace (OCIO) tricks are needed for correct blur/glow.
- **Plugin dependency sprawl.** The baseline kit is ~6–8 paid third-party plugins
  (Twixtor, RSMB, Sapphire, BCC, Deep Glow, Particular, FilmConvert…) — cost, licensing,
  and "project file won't open without X" friction for the very project-file-sharing
  culture the scene runs on.
- **Precomp labyrinth.** Transition/ramp structures need deep nesting; scripts
  (Twixtor Assistor, NESLayers, FXconsole) exist solely to automate AE boilerplate —
  evidence the primitives are wrong-shaped for this workflow.
- **No native editing comfort.** AE is a compositor; assembling a 2-minute music video
  in it means fighting the timeline (community meme: "you can now edit in After Effects"
  tutorials, Railcut plugin). Editors juggle Vegas/Premiere + AE round-trips.
- **Manual roto burden** in AE for non-demo games; Rotobrush is slow/imprecise on fast
  game footage.
- **Ease copying/graph friction** — see EasyCopy above; managing hundreds of eased
  keyframes across layers is hand-cramping in AE's graph editor.
- **External tool chain**: HLAE/IW3MVM recording, Flowframes pre-pass, AMVtool/Handbrake
  re-encoding, Payhip for distribution — a native tool that ingests demo passes and
  camera paths directly would delete real steps.

## 6. Source list (key)

- Channel feeds (full descriptions): youtube.com/feeds/videos.xml?channel_id=
  UC1CBXzcx7J2ZMTuC9du8W8A (Luminal), UCZ-_vgo18EDJwXyj-C9ohiw (stooh),
  UCT1LGKPe5Vo3awxbq-5eGVw (starker)
- Stores: payhip.com/Luminal, payhip.com/starker, stooh pack payhip.com/b/6dciH
- Key videos: Cronus edit k1SZpKtfgvs + breakdown 0me_MwjvGp0; Atlas ftr_unBLfJo;
  Advanced Motion Blur YdbJijoHJmw; Advanced Depth & Particular Ed0p5uA3MjM;
  stooh Ängel epHdJO5_bNg; starker BREAKDOWN: FaZe1 UKS_mpyVB7w
- Community: codmvm.com (+ /archive, /mod/iw3mvm), effectscollective.com (plugins +
  free-tools articles), doc.hlae.site + advancedfx wiki (mirv_streams matte/depth),
  steamcommunity.com/groups/VenexaArts, linktr.ee/starker, behance.net/azizvisuals,
  jayd3nfx flow-style tutorial yr1E5WffuUA, urbandictionary.com/define.php?term=MVM
