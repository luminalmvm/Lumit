# Research: the gaming-edit / montage editing scene (for Kiriko design doc)

Researched 2026-07-12 via web search + fetches. Confidence flags: [verified] = multiple sources; [likely] = single source or strong community consensus; [unverified] = could not confirm directly (YouTube blocks anonymous fetches via consent redirect; Socialblade 403s).

---

## 1. T3C and the scene's big names

### T3C — "The Editing Community"
- **T3C = The Editing Community**, self-described as "the largest editing community in the world with one mutual goal: to help improve your creative skills". [verified — Discord invite page, X/Twitter @t3c_editing, discordbotlist]
- It is simultaneously a **Discord server** (discord.gg/t3c, ~35,000 members per discordbotlist snapshot), a **YouTube channel** (channel UCXxNxWXAd0m0B6oIGTtxlNQ; also surfaced under handle @the3n19ma), an X account, and even a Steam group. The Discord "focuses on helping others and gathering everyone with an interest in video-game editing or editing in general, plus 3D people (Cinema 4D/Blender)". [verified for Discord/X; subscriber count of the YT channel unverified — YouTube consent-walled the fetch, Socialblade 403'd. The 100k+ figure given in the brief is plausible and consistent with its "largest community" positioning but I could not independently confirm it.]
- Channel content: editing tutorials, community edit showcases and playlists (e.g. a "T3C fortnite edits" playlist), style breakdowns. Centre of gravity is **Fortnite/FPS montages edited in After Effects**, with crossover into anime edits (AMV) and TikTok velocity edits — the same technique stack.
- Structurally, T3C is the model of how this scene organises: a Discord where clips, **project files ("PFs")**, CC packs and presets are shared, critiques exchanged, and editors recruited by cash-paying YouTubers/orgs.

### Other big names / reference points
- **FaZe Flea** — probably the single most-imitated Fortnite montage editor; multiple "How to Edit Like Flea / FaZe Flea — THE ULTIMATE GUIDE" tutorials with large view counts. [verified]
- **Bloodx** (Norwegian Fortnite creator, montage-heavy channel) — frequently referenced in the montage-editing orbit. [likely]
- The **Fiverr tier**: hundreds of $5–$50 gig editors advertise "clean smooth Fortnite montage with flow, impacts, buildups, beatshakes, transitions" — that vocabulary is the scene's shared style language. [verified]
- Adjacent scenes with the identical toolchain: **Valorant montages** (Twixtor + Magic Bullet "CC" tutorials), **AMV/anime edits**, and **TikTok velocity edits** (bbunsie, kqrivfx etc. — TikTok tutorial creators with big followings).

### The aesthetic
- **Velocity edits**: constant speed-ramping — footage whips fast→ultra-slow→fast, timed to the music. The defining move of the scene. [verified]
- **Smooth/flow edits**: everything eased; smooth zooms/punch-ins, position slides, "flow" between clips so the whole montage feels like one continuous camera move.
- **Beat-sync everything**: cuts, shakes ("beatshakes"), zooms, flashes land exactly on kicks/snares; buildups get accelerating effects, the drop gets the hero clip.
- Signature garnish: **camera shake on impacts, RGB split/chromatic aberration, glow blooms, flash/strobe frames, motion blur on all movement, letterboxing, stylised colour grade ("CC")**.

---

## 2. The typical pipeline

1. **Record gameplay** — NVIDIA ShadowPlay (now "NVIDIA App") or OBS. High fps is prized for slow-mo headroom: NVIDIA added **240 fps ShadowPlay recording up to 4K** (RTX 40/50 dual-NVENC; single-encoder cards do 1440p240). Typical bitrates 25–40 Mbps at 1080p60, 45–70 Mbps at 1440p60, higher for 240 fps. Hardware NVENC = near-zero performance cost while playing. [verified]
   - Serious montage players re-record in private matches/replay mode for clean clips; Fortnite's Replay system lets them re-shoot kills with free cameras.
2. **Clip triage / rough cut** — some cut in Premiere Pro, Vegas Pro (the old-school choice), or DaVinci Resolve, then finish in AE; a large fraction of the scene does the *entire* edit inside After Effects despite it not being an NLE, because the effects only exist there. [verified — tutorials exist for Vegas, Resolve, Premiere and AE variants of the same montage workflow]
3. **Pick the song first, sync to beats** — markers on kicks/snares; every cut/effect keyed to markers. (CapCut has auto beat-sync; AE has nothing native — editors tap markers by hand or use scripts.)
4. **Time-ramping** — Twixtor (or AE's Timewarp/frame-blend as the poor-man's version) for velocity ramps; time-remap keyframes shaped in the **graph editor** ("smooth velocity" tutorials are essentially graph-editor tutorials).
5. **Motion pass** — smooth zooms/punch-ins (scale+position keyframes with eased curves, or Sapphire S_WarpTransform / Premiere Transform-effect presets), camera shake (S_Shake or wiggle expressions), 3D camera moves on flat gameplay.
6. **Style pass** — RSMB motion blur over everything, Deep Glow blooms, flash frames/strobes on beats, RGB split, optics compensation warps.
7. **"CC" (colour correction)** — Magic Bullet Looks preset or a hand-built stack; editors share "CC packs" (.ffx presets / Looks files) constantly.
8. **Render** — H.264 via Media Encoder, 1080p60 VBR ~16–25+ Mbps for YouTube. [verified]

### Community sharing culture
- **"PF" = project file.** Editors give away or sell .aep project files of a finished edit ("Free PF in desc."); learners open them to reverse-engineer. [verified — e.g. "Valorant Velocity/Twixtor Tutorial | Free PF in desc."]
- **CC packs** (colour presets, often requiring Magic Bullet Looks), **editing packs** (transitions, overlays, shakes as .ffx/.aep) distributed via Velosofy, Payhip, Discord servers, TikTok links, AEJuice free packs. [verified]
- Implication for Kiriko: **preset/pack import-export and shareable project files are a first-class growth mechanic**, not an afterthought. The scene onboards via shared files.

---

## 3. Staple third-party plugins and what stock AE lacks

| Plugin | Vendor / price ballpark | What it does | Why stock AE can't |
|---|---|---|---|
| **Twixtor (Pro)** | RE:Vision FX, ~$139–330 | Per-pixel optical-flow retiming; synthesises new frames for extreme slow-mo; the engine behind velocity edits | AE's Timewarp uses the same-generation older Kronos tech but is widely considered worse/slower; Pixel Motion frame blending artefacts more |
| **RSMB (ReelSmart Motion Blur)** | RE:Vision FX, ~$120 | Adds optical-flow motion blur to footage that has none (game footage has zero natural motion blur) | AE's Pixel Motion Blur exists but is slow and lower quality; CC Force Motion Blur only works on layer transforms |
| **Deep Glow** | Plugin Everything, ~$30 | Physically-inspired, energy-conserving glow with proper falloff, GPU-accelerated | AE's stock Glow is notoriously ugly/clip-prone; Deep Glow is near-universal in packs |
| **Sapphire (S_Shake, S_WarpTransform, S_Glow, S_FilmEffect…)** | Boris FX, ~$495/yr — overwhelmingly pirated in this scene | 270+ effects; S_Shake = one-slider parameterised camera shake (Normal/Twitchy/Jumpy) used constantly for impacts/beatshakes | Wiggle expressions do crude shake but no per-frequency control, no motion-blurred shake, no easy presets |
| **Magic Bullet Looks** | Maxon/Red Giant, ~$25/mo suite | Point-and-click grading playground, 300+ presets — the "CC" engine of the scene | Lumetri exists but the preset-first, tool-chain UI of Looks is what packs are built on |
| **Red Giant Universe** | Maxon | Grab-bag of stylised transitions, glows, VHS, flicker/strobe effects | Convenience/preset breadth |
| **BCC / Continuum** (occasional), **Optical Flares** (Video Copilot, lens flares), **Saber** (free, energy beams), **FX Console** (free Video Copilot workflow script) | — | Garnish + workflow speedups | — |

Key insight: **every staple plugin is compensating for a missing GPU-native primitive: optical-flow retime, optical-flow motion blur, quality glow, parameterised shake, preset-driven grading.** A native editor that ships these five in-box removes ~$800+ of plugin spend (or the piracy that substitutes for it — the plugin cost issue is openly acknowledged: they "can be quite expensive to purchase", and the scene skews teenage).

---

## 4. AE pain points for this audience

- **Preview lag is the #1 complaint.** AE renders on CPU into a RAM cache; velocity edits with Twixtor+RSMB+Deep Glow stacked mean editors work at quarter-res and still can't scrub. Adobe forum threads on "Twixtor stuttering/lagging" note the issue is widespread among TikTok-style editors. RAM guidance alone tells the story: 16 GB bare minimum, 32 GB recommended, and AE still previews only what fits in cache. [verified]
- **Crashes and lost work** — chronic community complaint, especially with third-party plugin stacks and the cracked installs common in the scene.
- **Slow exports** — H.264 out of AE is second-class (Adobe's own guidance: round-trip through Media Encoder); a 3-minute montage can take hours on a mid-range machine.
- **Cost**: AE ~$23/mo alone / ~$55/mo All Apps, plus the plugin stack above. Commentary is blunt: for students and social-media creators "that subscription is hard to defend". Result: rampant cracked AE + cracked Twixtor/Sapphire among younger editors. [verified for pricing sentiment; piracy prevalence is scene common-knowledge, consistent with GitHub "After Effects Pack" repos and mod-APK ecosystem around Alight Motion]
- **Multi-app juggling**: Premiere/Vegas/Resolve for cutting + AE for effects + Media Encoder for export; or doing everything in AE, which has no real timeline-editing ergonomics (no ripple trim, painful audio handling, no beat-marker tooling).
- **Twixtor artefacts**: warping/"gloopiness"/splotches on fast motion, occlusions and HUD elements; workarounds are manual (animating Motion Vector Quality, masking, pre-composing HUD out). [verified — RE:Vision's own docs/forums]
- **Frame-rate mismatch bugs**: mixing 240 fps clips into 60 fps comps with Twixtor is a stutter minefield (Adobe forum thread confirms).
- **No native beat sync**: syncing to music is entirely manual in AE; CapCut's auto-velocity/beat-sync is a big reason beginners start (and stay) there.

---

## 5. What the scene says it wants (ideal-editor signals)

Direct wish-thread evidence is thin (Reddit is poorly indexed), but the revealed preferences are consistent across tutorials, tool marketing and migration patterns:

- **Real-time playback of the stacked look** (retime + blur + glow + grade) — the single thing AE cannot give them; CapCut/Alight Motion won users on instant preview despite far weaker output.
- **Beat/music tooling**: waveform prominence, auto beat markers, snap-to-beat keyframes (CapCut's headline feature; absent in AE).
- **Velocity ramping as a first-class primitive** with a good curve editor and built-in high-quality optical flow — not a $300 plugin bolted onto time-remap.
- **One app for cut + effects + export** — the Premiere+AE+AME shuffle is resented.
- **Cheap or free**, or at least ownable — the audience is teenagers; the free tier is how CapCut, Resolve and Alight Motion each took a slice of this scene. Migration to DaVinci Resolve for montages is already visible ("How to Edit a Fortnite Montage for BEGINNERS — DaVinci Resolve", multiple 2023–24 tutorials) purely on price, despite Resolve lacking the AE effect stack.
- **Preset/pack ecosystem compatibility** — a new editor is dead on arrival for this scene if there's no way to share PFs/CC packs; conversely, shipping with a pack marketplace/import is instant adoption fuel.
- Alternatives currently discussed (Natron, Fusion, Pikimov, Blender, Cavalry/Maxon's new free tools) all miss this audience: node-based, mograph-focused, or missing optical-flow retiming — i.e. **nobody has built "AE for montage kids" natively on the GPU**. That is Kiriko's gap.

---

## 6. Hardware profile

- **Consumer gaming PCs, overwhelmingly NVIDIA.** They already own RTX cards because they play the games; ShadowPlay/NVENC workflow assumes GeForce. RTX 40/50 series adds dual NVENC (240 fps capture, fast H.264/HEVC/AV1 encode). [verified]
- RAM: gaming builds ship 16–32 GB; AE's own guidance (16 min / 32 recommended) means many are at the floor. A GPU-resident pipeline sidesteps the RAM-cache model entirely.
- Storage: NVMe standard on gaming builds; source footage is high-bitrate H.264/HEVC game capture (long-GOP — decode performance matters; NVDEC hardware decode is available on every card they own).
- Practical takeaway: **CUDA/NVENC/NVDEC-first is the right priority**; AMD/Intel GPUs are a minority in this audience; many are on gaming *laptops* (thermals → efficiency matters).

## 7. Deliverables / formats

- **YouTube montages: 1080p60 or 1440p60 H.264**, VBR ~16–25+ Mbps (1440p+ gets YouTube's VP9/AV1 re-encode = visibly better quality, a known trick: upscale 1080p projects to 1440p/4K for upload).
- **TikTok/Shorts/Reels: 1080×1920 vertical**, short (10–60 s) velocity edits; often the same clips re-framed. Auto/easy vertical reframe is a real feature need.
- 60 fps is table stakes for gameplay (24/30 fps montages read as amateur in this scene); source material 60–240 fps.
- Audio: AAC stereo 192 kbps+; music-driven, minimal dialogue.

---

## Sources (main)
- Discord/T3C: discord.com/invite/t3c; discordbotlist.com/servers/t3c; x.com/t3c_editing; steamcommunity.com/groups/T3C_Group
- Twixtor: revisionfx.com/products/twixtor; creativecow.net Twixtor warping threads; community.adobe.com "Twixtor is stuttering/lagging" (12043592)
- Plugins: effectscollective.com "Most Used After Effects Plugins"; rohitvfx.com top-19 plugins; borisfx.com S_Shake docs/blog; maxon.net Magic Bullet Looks
- Pipeline/tutorials: youtube.com watch?v=7EZvAX8F87c (Valorant velocity + free PF), 9mmYE-jLg2w (full velocity tutorial), r8peQlw1GUs (Edit like FaZe Flea), aRoGioOvE5E (Pro Fortnite montages in AE 2024), aejuice.com velocity-edits blog
- Capture: tweaktown.com NVIDIA 240fps ShadowPlay; fragclips.com ShadowPlay settings; techguides.yt bitrates
- Pain/cost: dev.to "After Effects Is Too Expensive"; quora.com AE pricing; schoolofmotion.com RAM guide; pugetsystems.com AE hardware recommendations
- Alternatives landscape: xda-developers.com AE-alternative series; justcreative.com 2025 alternatives; capcut.com velocity-edit pages; themotionalight.com Alight Motion vs AE
- Packs/PF culture: velosofy.com fortnite editing packs; payhip.com free CC packs; junoschool.org AE export settings
