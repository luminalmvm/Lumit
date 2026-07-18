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
    /// Audio-clock-driven when this comp's mixed audio is loaded and running
    /// (the audio card's sample count is the one clock — docs/impl §4), else a
    /// wall-clock fallback so silent comps and the pre-mix moment still play.
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

        let clock_driven = self.audio_loaded_comp == Some(comp_id)
            && self.audio_engine.as_ref().is_some_and(|e| e.is_playing());
        // The audio clock IS comp time (mix sample 0 = comp time 0).
        let frame = if clock_driven {
            let t = self
                .audio_engine
                .as_ref()
                .map(|e| e.clock_seconds())
                .unwrap_or(0.0);
            (t * fps).round().max(0.0) as usize
        } else if let Some((started, start_frame)) = self.comp_playback {
            start_frame + (started.elapsed().as_secs_f64() * fps) as usize
        } else {
            return false;
        };

        if frame >= wa_end {
            // Loop the work area (07-UI-SPEC transport: loop work area default).
            // Re-seek AND re-play the mix so a loop that reached the buffer's
            // end (which pauses the stream) restarts cleanly.
            if self.audio_loaded_comp == Some(comp_id) {
                if let Some(engine) = &self.audio_engine {
                    engine.seek_seconds(wa_start as f64 / fps);
                    engine.play();
                }
            } else {
                self.comp_playback = Some((Instant::now(), wa_start));
            }
            self.preview_frame = wa_start;
            self.refresh_preview();
            return true;
        }
        if frame != self.preview_frame {
            self.preview_frame = frame;
            self.refresh_preview();
        }
        // Warm a little ahead of the clock so the work-area loop stays smooth
        // once frames are cached (docs/impl/playback-scheduler.md §5). Only when
        // the frame under the playhead is already cached — so this never
        // pre-empts the present request above — and one prefetch at a time.
        if self.fill_in_flight.is_none() {
            let present_cached = self
                .frame_key_for(comp_id, frame)
                .is_some_and(|k| self.comp_frame_cache.contains_key(&k));
            if present_cached {
                if let Some(prefetch) = self.next_playback_prefetch(comp_id, frame, wa_end) {
                    self.request_fill_frame(comp_id, prefetch);
                }
            }
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
    pub fn toggle_play(&mut self) {
        // Any second press pauses.
        if self.is_playing() {
            if let Some(engine) = &self.audio_engine {
                engine.pause();
            }
            self.comp_playback = None;
            return;
        }

        // Composition playback.
        if let Some(comp_id) = self.preview_comp {
            let doc = self.store.snapshot();
            let Some(fps) = doc.comp(comp_id).map(|c| c.frame_rate.fps().max(1.0)) else {
                return;
            };
            // Start on the wall clock immediately; audio joins when mixed.
            self.comp_playback = Some((Instant::now(), self.preview_frame));
            if self.audio_loaded_comp == Some(comp_id) {
                if let Some(engine) = &self.audio_engine {
                    engine.seek_seconds(self.preview_frame as f64 / fps);
                    engine.play();
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
        let Some(buffer) = self.audio_cache.get(&id).cloned() else {
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
