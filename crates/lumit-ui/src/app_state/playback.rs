//! `AppState` timeline playback: markers and beat detection, the playback
//! clock and per-frame tick, and the project title.

use super::*;

impl AppState {
    /// Trim the selected retimed footage layer so it ends exactly where the
    /// retime runs out of source (K-022): no auto-ripple — an explicit command
    /// the overrun indicator invites. No-op with a note if it doesn't overrun.
    #[cfg(feature = "media")]
    pub fn trim_selected_to_source_end(&mut self) {
        use lumit_core::model::LayerKind;
        use lumit_core::time::CompTime;
        use lumit_core::Rational;
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let Some(layer_id) = self.selected_layer else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Some(layer) = comp.layers.iter().find(|l| l.id == layer_id) else {
            return;
        };
        let LayerKind::Footage {
            item,
            retime: Some(rt),
        } = &layer.kind
        else {
            self.error = Some("select a retimed footage layer".into());
            return;
        };
        let src_dur = match self.media.map.get(item) {
            Some(media::MediaStatus::Ready { probe, frames, .. }) => match probe.video.as_ref() {
                Some(v) => *frames as f64 / v.fps().max(1.0),
                None => return,
            },
            _ => return,
        };
        let src_r = Rational::from_f64_on_grid(src_dur, 1000).unwrap_or(Rational::ONE);
        let Some(ot) = rt.overrun_local_time(src_r) else {
            self.error = Some("this clip doesn't run out of source".into());
            return;
        };
        let in_point = layer.in_point;
        let start_offset = layer.start_offset;
        let new_out = start_offset.0.to_f64() + ot;
        let out_point =
            CompTime(Rational::from_f64_on_grid(new_out, 1000).unwrap_or(layer.out_point.0));
        self.commit(Op::SetLayerSpan {
            comp: comp_id,
            layer: layer_id,
            in_point,
            out_point,
            start_offset,
        });
        self.refresh_preview();
    }

    /// Drop a user marker at the playhead on the current composition.
    pub fn add_marker_at_playhead(&mut self) {
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let Ok(t) = comp.frame_rate.time_of_frame(self.preview_frame as i64) else {
            return;
        };
        let mut markers = comp.markers.clone();
        markers.push(lumit_core::markers::Marker::user(uuid::Uuid::now_v7(), t.0));
        markers.sort_by_key(|m| m.time.0);
        self.commit(lumit_core::Op::SetCompMarkers {
            comp: comp_id,
            markers,
        });
    }

    /// Remove all detected Beat markers from the current composition, keeping
    /// user and chapter markers.
    pub fn clear_beat_markers(&mut self) {
        let Some(comp_id) = self.preview_comp.or(self.selected_comp) else {
            return;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        if !comp.markers.iter().any(|m| m.is_beat()) {
            return;
        }
        let markers: Vec<_> = comp
            .markers
            .iter()
            .filter(|m| !m.is_beat())
            .cloned()
            .collect();
        self.commit(lumit_core::Op::SetCompMarkers {
            comp: comp_id,
            markers,
        });
    }

    /// Detect beat markers for `comp_id` off the UI thread: mix the comp's
    /// audio (same path as playback), run onset + tempo detection, and hand the
    /// beat times back ([`Self::poll_beats`] turns them into markers). No-op —
    /// with an error note — for a silent comp.
    #[cfg(feature = "media")]
    pub fn detect_beats(&mut self, comp_id: Uuid, sensitivity: f32) {
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let rate = self
            .ensure_audio_engine()
            .map(|e| e.device_rate())
            .unwrap_or(48_000);
        let jobs = self.comp_audio_jobs(&doc, comp);
        if jobs.is_empty() {
            self.error = Some("no audio in this composition to detect beats from".into());
            return;
        }
        let duration_s = comp.duration.0.to_f64();
        let tx = self.beats_tx.clone();
        std::thread::spawn(move || {
            let samples = crate::export::mixdown(&jobs, rate, duration_s);
            let analysis = lumit_audio::beat::analyse_stereo(&samples, rate, sensitivity);
            // Grid-assist: nudge near-grid onsets onto the tempo grid (≤45ms),
            // which removes the small analysis latency without moving outliers.
            let times: Vec<f64> = analysis.onsets.iter().map(|o| o.time).collect();
            let snapped = lumit_audio::beat::snap_to_grid(&times, analysis.bpm, 0.045);
            let beats: Vec<(f64, f32)> = snapped
                .iter()
                .zip(&analysis.onsets)
                .map(|(t, o)| (*t, o.confidence))
                .collect();
            let _ = tx.send((comp_id, analysis.bpm, beats));
        });
    }

    /// Drain finished beat analysis into Beat markers (replacing only the prior
    /// Beat markers) as one undo step.
    #[cfg(feature = "media")]
    pub fn poll_beats(&mut self) {
        let mut newest = None;
        while let Ok(msg) = self.beats_rx.try_recv() {
            newest = Some(msg);
        }
        let Some((comp_id, bpm, beats)) = newest else {
            return;
        };
        self.detected_bpm = Some((comp_id, bpm));
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            return;
        };
        let new_beats: Vec<lumit_core::markers::Marker> = beats
            .iter()
            .filter_map(|(t, c)| {
                let time = lumit_core::Rational::from_f64_on_grid(t.max(0.0), 1000).ok()?;
                Some(lumit_core::markers::Marker::beat(
                    uuid::Uuid::now_v7(),
                    time,
                    *c,
                ))
            })
            .collect();
        let markers = lumit_core::markers::with_regenerated_beats(&comp.markers, new_beats);
        self.commit(lumit_core::Op::SetCompMarkers {
            comp: comp_id,
            markers,
        });
    }

    #[cfg(feature = "media")]
    pub fn is_playing(&self) -> bool {
        self.comp_playback.is_some() || self.audio_engine.as_ref().is_some_and(|e| e.is_playing())
    }

    /// Advance comp playback; returns true while playing (UI keeps repainting).
    ///
    /// Two disciplines (K-171, docs/06 §6.5):
    /// - **Cached** (the default): render every frame, never skip. The playhead
    ///   advances to the next frame only once it is cached, paced to at most
    ///   realtime — so a rendered span plays at true speed and an
    ///   as-yet-unrendered span advances exactly as fast as frames complete
    ///   (slower than realtime), dropping none. Audio plays only during smooth
    ///   realtime replay and pauses whenever a frame is being awaited, so sound
    ///   never runs ahead of a stalled picture.
    /// - **Realtime**: chase the clock, never freeze; the adaptive controller
    ///   drops resolution to keep up ([`Self::comp_realtime_tick`]).
    #[cfg(feature = "media")]
    pub fn comp_playback_tick(&mut self) -> bool {
        if self.comp_playback.is_none() {
            return false;
        }
        let Some(comp_id) = self.preview_comp else {
            self.comp_playback = None;
            return false;
        };
        let doc = self.store.snapshot();
        let Some(comp) = doc.comp(comp_id) else {
            self.comp_playback = None;
            return false;
        };
        let (wa_start, wa_end) = self.work_area_frames(comp);
        let fps = comp.frame_rate.fps().max(1.0);
        if self.preview_realtime {
            self.comp_realtime_tick(comp_id, wa_start, wa_end, fps)
        } else {
            self.comp_cached_tick(comp_id, wa_start, wa_end, fps)
        }
    }

    /// Cached mode: render-gated stepping (K-171). See [`Self::comp_playback_tick`].
    #[cfg(feature = "media")]
    fn comp_cached_tick(
        &mut self,
        comp_id: Uuid,
        wa_start: usize,
        wa_end: usize,
        fps: f64,
    ) -> bool {
        use lumit_eval::schedule::{cached_audio_lookahead, cached_step};
        let Some(pb) = self.comp_playback else {
            return false;
        };
        // Clamp a playhead left outside the work area (e.g. by a scrub) back in.
        if self.preview_frame < wa_start || self.preview_frame >= wa_end {
            self.preview_frame = wa_start;
            self.comp_playback = Some(CompPlayback::start(wa_start));
            self.refresh_preview();
            return true;
        }
        let next = if self.preview_frame + 1 >= wa_end {
            wa_start // loop the work area
        } else {
            self.preview_frame + 1
        };
        // Is `next` ready to show? A keyable frame is ready once it is in the
        // RAM cache; an unkeyable frame (unprobed footage) can't be gated on the
        // cache, so it is treated as ready (best effort — it renders live).
        let next_ready = match self.frame_key_for(comp_id, next) {
            Some(key) => self.comp_frame_cache.contains_key(&key),
            None => true,
        };
        // Audio gate (owner): sound runs exactly when the coming quarter
        // second is already renderable, so a cached run has audio from its
        // very first frame — no warm-up streak — and a still-rendering
        // stretch stays silent instead of flapping on and off.
        let run_ready = next_ready
            && self.cached_run_ready(comp_id, cached_audio_lookahead(fps), wa_start, wa_end);
        let elapsed = pb.started.elapsed().as_secs_f64();
        let frame_dur = 1.0 / fps;
        let step = cached_step(next_ready, elapsed, frame_dur, run_ready);
        let mut audio_playing = step.audio_playing;

        if step.advance {
            // Fixed-timestep pace (tester report): carry the overshoot into
            // the next frame's window instead of restarting the timer at
            // "now", which lost up to a UI tick per frame — replay ran slower
            // than realtime, the audio clock pulled ahead, and the >2-frame
            // resync yanked it back for ever. A hitch re-anchors (and pauses
            // audio this tick) rather than fast-forwarding.
            let (carry, continuous) = lumit_eval::schedule::cached_pace_carry(elapsed, frame_dur);
            let started = Instant::now()
                .checked_sub(std::time::Duration::from_secs_f64(carry))
                .unwrap_or_else(Instant::now);
            if !continuous {
                audio_playing = false;
            }
            self.preview_frame = next;
            self.comp_playback = Some(CompPlayback {
                started,
                start_frame: next,
            });
            self.refresh_preview();
        } else if step.request_next {
            // Render-gate: make sure the frame we are waiting on is on its
            // way (idempotent — a matching request in flight is not
            // re-sent). A cached frame under the playhead also keeps its
            // display up to date.
            self.request_frame_render(comp_id, next);
        }

        // Audio follows the picture: play while the stretch ahead is ready,
        // pause while a frame is awaited, and keep it seeked to the shown
        // frame so a resumed stretch starts in sync.
        self.sync_cached_audio(comp_id, audio_playing, fps);

        // Warm ahead only once we are replaying smoothly (audio playing), so a
        // prefetch never competes with the render we are gated on.
        if audio_playing && self.fill_in_flight.is_none() {
            if let Some(prefetch) = self.next_playback_prefetch(comp_id, self.preview_frame, wa_end)
            {
                self.request_fill_frame(comp_id, prefetch);
            }
        }
        true
    }

    /// Whether the `lookahead` frames after the playhead (wrapping the work
    /// area exactly as playback does) are all ready to show — the Cached-mode
    /// audio gate (`run_ready`). Rides the memoised cache bar when the comp is
    /// small enough to have one, so the steady path adds no hashing; longer
    /// comps key just the window, a bounded per-tick cost. An unkeyable comp
    /// (unprobed or unprobeable footage) counts as ready, matching the
    /// stepper's own best-effort rule, so its audio still plays.
    #[cfg(feature = "media")]
    fn cached_run_ready(
        &mut self,
        comp_id: Uuid,
        lookahead: usize,
        wa_start: usize,
        wa_end: usize,
    ) -> bool {
        let span = (wa_end - wa_start).max(1);
        let frames: Vec<usize> = (1..=lookahead)
            .map(|k| {
                let f = self.preview_frame + k;
                if f >= wa_end {
                    wa_start + (f - wa_end) % span
                } else {
                    f
                }
            })
            .collect();
        if frames
            .first()
            .is_some_and(|&f| self.frame_key_for(comp_id, f).is_none())
        {
            return true; // unkeyable: renders live, best effort — like next_ready
        }
        let doc = self.store.snapshot();
        if let Some(bars) = doc.comp(comp_id).and_then(|comp| self.cache_bar(comp)) {
            return frames
                .iter()
                .all(|&f| matches!(bars.get(f), Some(CacheTier::Ram)));
        }
        frames
            .iter()
            .all(|&f| match self.frame_key_for(comp_id, f) {
                Some(key) => self.comp_frame_cache.contains_key(&key),
                None => true,
            })
    }

    /// Ensure `frame` of `comp_id` is being rendered/displayed now. Unlike
    /// [`Self::refresh_preview`] (which always targets `preview_frame`), this
    /// renders an arbitrary frame the cached stepper is gated on.
    #[cfg(feature = "media")]
    fn request_frame_render(&mut self, comp_id: Uuid, frame: usize) {
        // A frame the disk tier already holds is promoted, not re-rendered.
        if let Some(key) = self.frame_key_for(comp_id, frame) {
            if self.comp_frame_cache.contains_key(&key) {
                return; // already ready
            }
            if self.disk_has(key) {
                self.disk_request_load(key);
                return;
            }
        }
        if self.fill_in_flight != Some((comp_id, frame)) {
            self.request_fill_frame(comp_id, frame);
        }
    }

    /// Cached-mode audio: play/pause per the stepper and keep the audio clock
    /// pinned to the shown frame, so sound tracks the render-gated picture
    /// (K-171) instead of running on ahead.
    #[cfg(feature = "media")]
    fn sync_cached_audio(&mut self, comp_id: Uuid, want_playing: bool, fps: f64) {
        if self.audio_loaded_comp != Some(comp_id) {
            return; // silent comp, or its mix not loaded yet
        }
        let target_s = self.preview_frame as f64 / fps;
        let Some(engine) = &self.audio_engine else {
            return;
        };
        if want_playing {
            // Resync only when it has drifted more than ~2 frames, so smooth
            // replay is not re-seeked every frame (which would stutter).
            if (engine.clock_seconds() - target_s).abs() > 2.0 / fps {
                engine.seek_seconds(target_s);
            }
            if !engine.is_playing() {
                engine.play();
            }
        } else {
            if engine.is_playing() {
                engine.pause();
            }
            engine.seek_seconds(target_s); // park it on the shown frame
        }
    }

    /// Realtime mode: chase the wall/audio clock, dropping resolution to keep
    /// up rather than freezing (K-030/K-171).
    ///
    /// Audio never waits: it plays on its own clock and the picture chases it,
    /// showing the newest frame it manages to render. The rule that keeps this
    /// from freezing is **render-pull**: exactly one live render is in flight at
    /// a time and it is *never* superseded by the clock moving on. The old tick
    /// re-requested every clock boundary, so under load each render was
    /// abandoned before it finished — nothing ever completed, the adaptive
    /// controller was never fed a measurement, and the picture froze. Now a slow
    /// frame is allowed to finish; its measured cost drops the resolution tier
    /// ([`Self::target_width_for`] reads [`Self::realtime_ctrl`]); and only then
    /// do we pull the next frame — at the clock's *current* position, skipping
    /// whatever the render took. Cached frames present for free and don't gate.
    #[cfg(feature = "media")]
    fn comp_realtime_tick(
        &mut self,
        comp_id: Uuid,
        wa_start: usize,
        wa_end: usize,
        fps: f64,
    ) -> bool {
        let Some(pb) = self.comp_playback else {
            return false;
        };
        let clock_driven = self.audio_loaded_comp == Some(comp_id)
            && self.audio_engine.as_ref().is_some_and(|e| e.is_playing());
        let frame = if clock_driven {
            let t = self
                .audio_engine
                .as_ref()
                .map(|e| e.clock_seconds())
                .unwrap_or(0.0);
            (t * fps).round().max(0.0) as usize
        } else {
            pb.start_frame + (pb.started.elapsed().as_secs_f64() * fps) as usize
        };
        if frame >= wa_end {
            if self.audio_loaded_comp == Some(comp_id) {
                if let Some(engine) = &self.audio_engine {
                    engine.seek_seconds(wa_start as f64 / fps);
                    engine.play();
                }
            } else {
                self.comp_playback = Some(CompPlayback::start(wa_start));
            }
            self.preview_frame = wa_start;
            self.realtime_inflight = None; // abandon any live render across the loop
            self.refresh_preview();
            return true;
        }

        // Fast path: the clock frame is already cached — present it for free and
        // warm ahead. A live render becomes moot the moment a newer cached frame
        // carries the picture, so clear the gate.
        if let Some(key) = self.frame_key_for(comp_id, frame) {
            if self.comp_frame_cache.contains_key(&key) {
                if frame != self.preview_frame {
                    self.preview_frame = frame;
                    self.cached_present = Some(key);
                }
                self.realtime_inflight = None;
                if self.fill_in_flight.is_none() {
                    if let Some(prefetch) = self.next_playback_prefetch(comp_id, frame, wa_end) {
                        self.request_fill_frame(comp_id, prefetch);
                    }
                }
                return true;
            }
        }

        // Uncached: render-pull. Hold `preview_frame` on the frame we asked for
        // (so its result presents as the shown frame, is timed, and feeds the
        // controller — a mismatch would bank it as a silent fill instead) until
        // it lands. The completion handler clears `realtime_inflight`; the
        // timeout only rescues a render lost to an unrelated supersede.
        let awaiting_render = matches!(
            self.realtime_inflight,
            Some((_, at)) if at.elapsed() < super::REALTIME_RENDER_TIMEOUT
        );
        if !awaiting_render {
            // Pipeline idle (or a lost render timed out): pull the clock's
            // current frame and hold it until it lands. While a render is in
            // flight we do nothing — never supersede it (that was the freeze).
            self.preview_frame = frame;
            self.realtime_inflight = Some((frame, Instant::now()));
            self.refresh_comp_preview();
        }
        true
    }

    #[cfg(feature = "media")]
    pub fn playback_clock(&self) -> Option<f64> {
        self.audio_engine.as_ref().map(|e| e.clock_seconds())
    }

    /// Space: play/pause. Plays the open composition (audio-synced, mixing its
    /// audio layers) or the previewed footage, from the current frame.
    #[cfg(feature = "media")]
    /// Stop playback completely: pause the audio engine AND clear the comp
    /// playback clock. Scrubbing (a seek during playback) pauses, so the audio
    /// and the transport state must stop with the frame advance — clearing only
    /// `comp_playback` used to leave the audio engine running, so `is_playing`
    /// stayed true (the play button stuck) and the audio played on.
    pub fn pause_playback(&mut self) {
        if let Some(engine) = &self.audio_engine {
            engine.pause();
        }
        self.comp_playback = None;
        #[cfg(feature = "media")]
        {
            self.realtime_inflight = None;
        }
    }

    pub fn toggle_play(&mut self) {
        // Any second press pauses.
        if self.is_playing() {
            self.pause_playback();
            return;
        }

        // Composition playback.
        if let Some(comp_id) = self.preview_comp {
            let doc = self.store.snapshot();
            let Some(comp) = doc.comp(comp_id) else {
                return;
            };
            let fps = comp.frame_rate.fps().max(1.0);
            // Start on the wall clock immediately; audio joins when mixed.
            self.comp_playback = Some(CompPlayback::start(self.preview_frame));
            // Only replay the loaded mix when it still matches the comp; an edit
            // since it was baked (mute, move, trim, delete) re-bakes it instead,
            // so playback follows the current comp (GEN-4 fixes).
            let jobs = self.comp_audio_jobs(&doc, comp);
            let sig_matches = self.audio_loaded_comp == Some(comp_id)
                && !jobs.is_empty()
                && self.audio_loaded_sig
                    == Some(super::audio_jobs_signature(&jobs, comp.duration.0.to_f64()));
            if sig_matches {
                if let Some(engine) = &self.audio_engine {
                    engine.seek_seconds(self.preview_frame as f64 / fps);
                    engine.play();
                }
            } else if jobs.is_empty() {
                // No audible audio: drop any stale mix, play on the wall clock.
                if self.audio_loaded_comp == Some(comp_id) {
                    if let Some(engine) = &self.audio_engine {
                        engine.unload();
                    }
                    self.audio_loaded_comp = None;
                    self.audio_loaded_sig = None;
                }
            } else {
                self.prepare_comp_audio(comp_id);
            }
            return;
        }

        // Footage playback.
        let Some(id) = self.preview_item else { return };
        let Some(fps) = self.preview_fps() else {
            return;
        };
        let Some(buffer) = self
            .audio_cache
            .get(&id)
            .map(|c| std::sync::Arc::clone(&c.0))
        else {
            self.request_preview_audio(); // will be ready on a later press
            return;
        };
        let start = self.preview_frame as f64 / fps;
        if self.ensure_audio_engine().is_none() {
            return;
        }
        let needs_load = self.audio_loaded != Some(id);
        if needs_load {
            self.audio_loaded = Some(id);
            self.audio_loaded_comp = None; // a footage buffer is loaded now
        }
        if let Some(engine) = &self.audio_engine {
            if needs_load {
                engine.load(buffer);
            }
            engine.seek_seconds(start);
            engine.play();
        }
    }

    pub fn project_title(&self) -> String {
        let name = self
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        if self.dirty {
            format!("{name} • Lumit")
        } else {
            format!("{name} — Lumit")
        }
    }
}
