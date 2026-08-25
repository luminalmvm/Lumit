// The Dart-side read model (K-184): the fronted comp as the panels draw it.
//
// In plain terms: every question a panel used to ask the engine while drawing
// — layer names, switches, bar positions, effect values — is answered from
// this one held copy instead. The copy is refreshed by ONE bridge call
// (`getModel`), and only when the engine says the document changed. So a
// rebuild costs no bridge calls at all, and pure-interface changes (selecting
// a layer, moving the playhead) never touch the engine to redraw.
//
// The model is plain data. Edits still go through the reference handles —
// this holds what to *show*, never what to *do*.

import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:uuid/uuid.dart';

class CompModel extends ChangeNotifier {
  CompositionReference? _comp;
  BridgeCompModel? _model;

  /// The engine revision [_model] was read at, or null before the first read.
  /// Comparing it per read is what makes every rebuild honest: a widget that
  /// rebuilds for any reason sees the current document, exactly as when every
  /// widget re-read the engine itself — for one call instead of dozens.
  BigInt? _revision;

  /// The frame [_freshen] last asked the engine for its revision in.
  ///
  /// Asking is itself a bridge call, and a rebuild reads several getters —
  /// layers, duration, rate — each of which used to ask for itself. Twirling
  /// one layer open cost a dozen. The document cannot move part-way through a
  /// frame we are drawing, so one question per frame answers all of them; an
  /// edit made between frames calls [refresh] anyway, which forces a re-read.
  Duration? _checkedIn;

  /// The layers of the fronted comp, top of the stack first. Empty when no
  /// comp is fronted (panels then show their placeholder anyway).
  List<BridgeLayerEntry> get layers {
    _freshen();
    return _model?.layers ?? const [];
  }

  /// The comp's length in frames, matching `durationFrames`.
  int get durationFrames {
    _freshen();
    return _model?.durationFrames.toInt() ?? 0;
  }

  /// The comp's rate as a plain number — what maps seconds onto the time
  /// axis (the waveform lane) without a bridge call per paint. 60 before any
  /// model has loaded, so nothing divides by zero.
  double get fps {
    _freshen();
    final fps = _model?.fps ?? 60.0;
    return fps > 0 ? fps : 60.0;
  }

  /// The comp's exact rate, for the timecode readout: 29.97 must count 30
  /// frames a second, which the plain [fps] double cannot say (docs/14 §2).
  (int, int) get fpsExact {
    _freshen();
    final m = _model;
    return m == null ? (60, 1) : (m.fpsNum, m.fpsDen);
  }

  /// Whether the comp's master motion-blur shutter is on (K-120) — what the
  /// Timeline's master button draws. Writes go through
  /// `CompositionReference.setMotionBlurEnabled`.
  bool get motionBlurEnabled {
    _freshen();
    return _model?.motionBlurEnabled ?? false;
  }

  /// The engine revision the held model was read at, or null before the first
  /// read. What a panel caching something *derived* from the model compares
  /// against: the cache is good for as long as this number is, and an edit
  /// moves it. Reading it freshens the model first, so it never reports a
  /// revision older than what the caller is about to draw.
  BigInt? get revision {
    _freshen();
    return _revision;
  }

  /// The comp's background colour off the held copy, scene-linear RGBA, or
  /// null before the first read. For the Viewer bar's swatch, which rebuilds
  /// on every arriving frame: the colour rides in the model rather than being
  /// asked for per rebuild (K-184). Writes go through
  /// `CompositionReference.setBackground`; the change refreshes this model.
  F32Array4? get heldBackground => _model?.background;

  /// The copy in hand, and the revision it was read at, **without asking the
  /// engine anything** (K-230).
  ///
  /// For the paint path, and only for it. Every getter above checks with the
  /// engine that the document has not moved — once per frame while a frame is
  /// being built, and *every time* outside one, which is where pointer handlers
  /// run. That check is a bridge call, so a tool that redraws as the mouse
  /// moves was asking whether the document had changed at the rate the mouse
  /// reports, and the answer was always no: moving a mouse changes no document.
  ///
  /// Drawing never needs the check. A change refreshes this model and notifies,
  /// and everything that draws from it is listening — so a paint that used the
  /// held copy is repainted from the new one the moment there is one. Code that
  /// has just *committed* an edit and wants to read it back keeps the checking
  /// getters above.
  List<BridgeLayerEntry> get heldLayers => _model?.layers ?? const [];
  BigInt? get heldRevision => _revision;

  /// The comp's rate from the copy in hand, on the same terms — what turns a
  /// playhead frame into seconds while a paint is running, which is where a
  /// keyed shape modifier has to be read (K-553).
  double get heldFps {
    final fps = _model?.fps ?? 60.0;
    return fps > 0 ? fps : 60.0;
  }

  /// The comp this model is bound to has gone — deleted, or undone out of
  /// existence — rather than merely being empty. A comp that is *there* always
  /// reads as a model, even with no layers in it, so the pair below says
  /// exactly one thing: something is fronted, and the engine has never heard
  /// of it. Whoever fronted it is the one that has to move on ([_freshen]
  /// already refuses to throw), and this is how they find out for free.
  bool get compGone => _comp != null && _model == null;

  /// Point the model at [comp] (or null) and read it.
  void bind(CompositionReference? comp) {
    _comp = comp;
    refresh();
  }

  /// Re-read the whole model — one bridge call — and repaint whoever listens.
  ///
  /// Called when the engine reports a change, and by panels right after they
  /// commit an op, so their own edit is on screen without waiting for the
  /// change stream's round trip.
  void refresh() {
    _revision = null;
    _checkedIn = null;
    _freshen();
    notifyListeners();
  }

  /// Re-read only if the document has moved since the last read.
  void _freshen() {
    final comp = _comp;
    if (comp == null) {
      _model = null;
      return;
    }
    // Between frames there is no timestamp to group by — and a read there is
    // usually code checking its own edit landed — so those always ask.
    final binding = SchedulerBinding.instance;
    final frame = binding.schedulerPhase == SchedulerPhase.idle
        ? null
        : binding.currentFrameTimeStamp;
    if (frame != null && frame == _checkedIn && _model != null) return;
    _checkedIn = frame;
    try {
      final revision = comp.documentRevision();
      if (revision == _revision && _model != null) return;
      _model = comp.getModel();
      _revision = revision;
    } catch (_) {
      // The comp has gone (deleted, or the project closed): an empty model,
      // not a crash — the panels show their placeholders.
      _model = null;
    }
  }

  /// The entry for [id], or null when the layer is gone.
  BridgeLayerEntry? byId(UuidValue id) {
    for (final entry in layers) {
      if (entry.layer.internallayerId == id) return entry;
    }
    return null;
  }
}
