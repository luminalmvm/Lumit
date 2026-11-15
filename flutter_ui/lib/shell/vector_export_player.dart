// The Lottie and SVG window's player: the stage with the file playing on it,
// and under it the Viewer's own transport marks, a scrubber, the frame
// counter, and the choice of player, background and zoom.
//
// One clock. The Lottie player is driven by the window's animation
// controller, so the counter and the picture read the same frame. The
// browser player keeps its own time, as a page does: the transport goes to it
// as events and the frame it reports is what the counter shows.

part of 'vector_export_frb.dart';

/// Which player draws the file.
enum _Player { lottie, browser }

/// What is behind the picture.
enum _Ground { board, comp, white, black }

/// The transport rows under the stage.
const double _transportRow = 22;

/// Added before a frame is floored, so a product that lands a hair under a
/// whole frame still counts as that frame.
const double _epsilon = 1e-6;

extension _PlayerColumn on _VectorWindowState {
  int get _frames => _preview?.frames ?? 0;

  /// The frame the clock is on.
  int _frameOf(double value) => _frames == 0
      ? 0
      : (value * _frames + _epsilon).floor().clamp(0, _frames - 1);

  /// Only a running clock moves the frame. A clock set by a seek or a step
  /// is following the frame already chosen.
  void _tick() {
    if (_playerShown != _Player.lottie || !_clock.isAnimating) return;
    final frame = _frameOf(_clock.value);
    if (frame != _frame) _redraw(() => _frame = frame);
  }

  /// A new file: both players take it, on the frame the window is on.
  void _show(BridgeVectorPreview p) {
    final frame = p.frames == 0 ? 0 : _frame.clamp(0, p.frames - 1);
    _frame = frame;
    _clock.duration = Duration(
        microseconds: p.frames == 0 || p.fps <= 0
            ? 1000000
            : (p.frames / p.fps * 1000000).round());
    _clock.value = p.frames == 0 ? 0 : frame / p.frames;
    if (_playing && _playerShown == _Player.lottie) _runClock();
    _web.setGround(_groundCss(ThemeScope.of(context).theme));
    _web.load(
      bytes: p.bytes,
      kind: _settings.format == 'svg' ? 'svg' : 'lottie',
      fps: p.fps,
      frames: p.frames,
      loop: _settings.loopPlayback,
      frame: frame,
    );
    _onPlayhead();
  }

  void _setGround(_Ground ground) {
    _redraw(() => _ground = ground);
    _web.setGround(_groundCss(ThemeScope.of(context).theme));
  }

  /// The ground as the browser player's page paints it, since a web view
  /// draws its own page over whatever is behind it.
  String _groundCss(LumitTheme t) {
    String css(Color c) {
      int byte(double v) => (v * 255).round().clamp(0, 255);
      return 'rgba(${byte(c.r)},${byte(c.g)},${byte(c.b)},${c.a.toStringAsFixed(3)})';
    }

    const tile = _BoardPainter._square * 2;
    return switch (_ground) {
      _Ground.board => 'repeating-conic-gradient(${css(t.surface1)} 0% 25%, '
          '${css(t.surface2)} 0% 50%) 0 0 / ${tile}px ${tile}px',
      _Ground.comp => css(viewerBackgroundColour(_compSettings.background)),
      _Ground.black => css(t.backgroundPresets[0]),
      _Ground.white => css(t.backgroundPresets[1]),
    };
  }

  double get _compFps => _compSettings.fpsDen == 0
      ? 0
      : _compSettings.fpsNum / _compSettings.fpsDen;

  /// The file's frame that shows the comp's frame [frame].
  int _fileFrameOf(int frame) {
    final p = _preview;
    if (p == null || _frames == 0 || _compFps <= 0) return 0;
    return ((frame / _compFps - p.start) * p.fps + _epsilon)
        .floor()
        .clamp(0, _frames - 1);
  }

  /// The comp's frame that the file's frame [frame] shows.
  int _compFrameOf(int frame) {
    final p = _preview;
    if (p == null || p.fps <= 0) return 0;
    return ((p.start + frame / p.fps) * _compFps).round();
  }

  /// Keep the player to the Timeline, so the file plays beside Lumit's own
  /// frame in the Viewer: the Timeline's transport runs both.
  void _setFollow(bool on) {
    _redraw(() => _follow = on);
    if (!on) return;
    if (_playing) {
      _playing = false;
      _clock.stop();
      _web.pause();
    }
    if (widget.ui.selectedComp?.internalid != _comp.internalid) {
      widget.ui.setSelectedComp(_comp);
    }
    _onPlayhead();
  }

  void _onPlayhead() {
    if (!_follow || _driving || _preview == null) return;
    if (widget.ui.selectedComp?.internalid != _comp.internalid) return;
    _showFrame(_fileFrameOf(widget.ui.playheadFrame.value));
  }

  void _onShellPlaying() {
    if (_follow && mounted) _redraw(() {});
  }

  /// Whether the file is playing: the window's own clock, or the Timeline's
  /// while the player keeps to it.
  bool get _shownPlaying => _follow ? widget.ui.playing.value : _playing;

  /// Put the player on [frame], and nothing else.
  void _showFrame(int frame) {
    if (_frames == 0 || frame == _frame) return;
    _redraw(() => _frame = frame);
    if (_playerShown == _Player.browser) {
      _web.seek(frame);
    } else {
      _clock.value = frame / _frames;
    }
  }

  /// Save the frame the player is on as a PNG, drawn by the Lottie player at
  /// the file's own size.
  Future<void> _saveFrame() async {
    final p = _preview;
    if (p == null || p.frames == 0) return;
    final frame = _frame;
    final path = await pickFrameSaveLocation('${_fileName(_compName)} $frame.png');
    if (path == null || !mounted) return;
    try {
      final png = await lottieFramePng(p.bytes,
          zip: _settings.format == 'dotlottie', progress: frame / p.frames);
      if (png == null) throw StateError('no picture');
      await File(path).writeAsBytes(png);
      widget.app.postNotice(l10n.viewerFrameSaved);
    } catch (_) {
      widget.app.postNotice(l10n.viewerFrameNotSaved);
    }
  }

  void _runClock() {
    if (_settings.loopPlayback) {
      _clock.repeat();
    } else {
      if (_clock.value >= 1) _clock.value = 0;
      _clock.forward();
    }
  }

  void _togglePlay() {
    if (_frames == 0) return;
    if (_follow) {
      final ui = widget.ui;
      if (ui.playing.value) {
        ui.stopPlayback(restorePlayhead: false);
      } else {
        if (ui.selectedComp?.internalid != _comp.internalid) {
          ui.setSelectedComp(_comp);
        }
        ui.play();
      }
      return;
    }
    // Played again from its end, a file that doesn't loop starts over.
    if (!_playing && !_settings.loopPlayback && _frame >= _frames - 1) _seek(0);
    _redraw(() => _playing = !_playing);
    if (_playerShown == _Player.browser) {
      _playing ? _web.play() : _web.pause();
    } else if (_playing) {
      _runClock();
    } else {
      _clock.stop();
    }
  }

  void _seek(int frame, {bool stop = false}) {
    if (_frames == 0) return;
    frame = frame.clamp(0, _frames - 1);
    if (_follow) {
      // The playhead goes there too while this comp is in front, so the
      // Viewer shows Lumit's own frame. Another comp's playhead stays put.
      if (widget.ui.selectedComp?.internalid == _comp.internalid) {
        _driving = true;
        widget.ui.scrubTo(_compFrameOf(frame));
        _driving = false;
      }
      _showFrame(frame);
      return;
    }
    if (stop && _playing) {
      _playing = false;
      if (_playerShown == _Player.browser) _web.pause();
    }
    _redraw(() => _frame = frame);
    if (_playerShown == _Player.browser) {
      _web.seek(frame);
    } else {
      _clock.value = frame / _frames;
      if (_playing) _runClock();
    }
  }

  void _step(int by) {
    if (_frames == 0) return;
    // A step wraps, so the arrow keys walk round a loop.
    _seek((_frame + by) % _frames, stop: true);
  }

  /// The player drawing the file: an SVG plays only in the browser and a
  /// dotLottie only in the Lottie player, a Lottie in the one chosen.
  _Player get _playerShown => switch (_settings.format) {
        'svg' => _Player.browser,
        'dotlottie' => _Player.lottie,
        _ => _player,
      };

  void _setPlayer(_Player player) {
    if (player == _player) return;
    final before = _playerShown;
    _redraw(() => _player = player);
    _handOff(before);
  }

  /// The file moved from [before] to another player, which takes up the
  /// frame and the playing.
  void _handOff(_Player before) {
    if (_playerShown == before) return;
    if (_playerShown == _Player.browser) {
      _clock.stop();
      _web.seek(_frame);
      if (_playing) _web.play();
    } else {
      _web.pause();
      if (_frames > 0) _clock.value = _frame / _frames;
      if (_playing) _runClock();
    }
  }

  void _setLoop(bool on) {
    if (_playerShown == _Player.browser) _web.setLoop(on);
    if (_playing && _playerShown == _Player.lottie) {
      _clock.stop();
      _settings = _copy(_settings, loopPlayback: on);
      _runClock();
    }
    // The file carries it too: an SVG's repeat and a dotLottie's manifest.
    _edit(_copy(_settings, loopPlayback: on));
  }

  Widget _playerColumn(BuildContext context, LumitTheme t) => Padding(
        padding: const EdgeInsets.fromLTRB(dialogPadding, 12, 12, 10),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(child: _stage(t)),
            const SizedBox(height: 8),
            _transport(t),
            const SizedBox(height: 4),
            _looks(t),
          ],
        ),
      );

  /// The picture on its ground, fitted or at the zoom asked for, clipped.
  Widget _stage(LumitTheme t) {
    final p = _preview;
    final ground = switch (_ground) {
      _Ground.board => null,
      _Ground.comp => viewerBackgroundColour(_compSettings.background),
      _Ground.black => t.backgroundPresets[0],
      _Ground.white => t.backgroundPresets[1],
    };
    return GestureDetector(
      onSecondaryTapUp: (press) => _stageMenu(press.globalPosition),
      child: ClipRRect(
      borderRadius: BorderRadius.circular(t.tokens.sectionRadius),
      child: Container(
        key: const ValueKey<String>('vector-stage'),
        decoration: BoxDecoration(
          color: ground ?? t.surface1,
          border: Border.all(color: t.hairline),
        ),
        child: CustomPaint(
          painter: ground == null ? _BoardPainter(t) : null,
          child: p == null || p.width == 0 || p.height == 0
              ? const SizedBox.expand()
              : LayoutBuilder(
                  builder: (context, box) {
                    final zoom = _zoom ??
                        (box.maxWidth / p.width)
                            .clamp(0.0, box.maxHeight / p.height);
                    return ClipRect(
                      child: OverflowBox(
                        maxWidth: double.infinity,
                        maxHeight: double.infinity,
                        child: SizedBox(
                          width: p.width * zoom,
                          height: p.height * zoom,
                          child: _playerShown == _Player.browser
                              ? WebPlayer(controller: _web)
                              : LottiePlayer(
                                  key: const ValueKey<String>('vector-lottie'),
                                  bytes: p.bytes,
                                  zip: _settings.format == 'dotlottie',
                                  progress: _clock,
                                ),
                        ),
                      ),
                    );
                  },
                ),
        ),
      ),
      ),
    );
  }

  /// The stage's own menu: the frame on it saved as a picture.
  void _stageMenu(Offset at) {
    // The Lottie player draws the frame; an SVG plays only in the browser.
    final can = _settings.format != 'svg' && _frames > 0 && !kIsWeb;
    reviewRowMenu(context, at, [
      (
        'vector-save-frame',
        l10n.viewerMenuSaveFrame,
        can ? () => unawaited(_saveFrame()) : null,
      ),
    ]);
  }

  /// Play, loop, the Timeline, the scrubber and the frame.
  Widget _transport(LumitTheme t) {
    final frames = _frames;
    final live = frames > 0;
    return SizedBox(
      height: _transportRow,
      child: Row(children: [
        viewerBarMark(
          key: const ValueKey<String>('vector-play'),
          icon: _shownPlaying ? LumitIcon.pause : LumitIcon.play,
          colour: live ? t.textPrimary : t.textDisabled,
          onPressed: live ? _togglePlay : null,
          tip: _shownPlaying ? l10n.tipTransportPause : l10n.tipTransportPlay,
        ),
        viewerBarGapBox(viewerTransportGap),
        viewerBarMark(
          key: const ValueKey<String>('vector-loop'),
          icon: LumitIcon.loop,
          colour: _settings.loopPlayback ? t.textPrimary : t.textMuted,
          onPressed: () => _setLoop(!_settings.loopPlayback),
          tip: l10n.vectorTipLoop,
        ),
        viewerBarGapBox(viewerTransportGap),
        viewerBarMark(
          key: const ValueKey<String>('vector-follow'),
          icon: _follow ? LumitIcon.link : LumitIcon.unlink,
          colour: _follow ? t.textPrimary : t.textMuted,
          onPressed: () => _setFollow(!_follow),
          tip: l10n.vectorTipFollow,
        ),
        const SizedBox(width: viewerTransportGap),
        Expanded(
          child: LayoutBuilder(
            builder: (context, box) => HouseSlider(
              key: const ValueKey<String>('vector-scrub'),
              value: live ? _frame.toDouble() : 0,
              min: 0,
              max: live ? (frames - 1).toDouble() : 1,
              step: 1,
              showValue: false,
              width: box.maxWidth,
              onChanged: live ? (f) => _seek(f.round()) : (_) {},
            ),
          ),
        ),
        const SizedBox(width: viewerTransportGap),
        DragValueField(
          key: const ValueKey<String>('vector-frame'),
          value: _frame,
          bare: true,
          min: 0,
          max: live ? frames - 1 : 0,
          decimals: 0,
          resetTo: 0,
          onChanged: (f) => _seek(f.round(), stop: true),
          onChangeLive: (f) => _seek(f.round(), stop: true),
        ),
        Text(
          ' / $frames',
          key: const ValueKey<String>('vector-counter'),
          style: t.mono
              .copyWith(fontSize: viewerTimecodeSize, color: t.textMuted),
        ),
      ]),
    );
  }

  /// Which player, what is behind the picture, and how big.
  Widget _looks(LumitTheme t) => SizedBox(
        height: _transportRow,
        child: Row(children: [
          LumitTooltip(
            message: l10n.vectorTipPlayer,
            child: SizedBox(
              height: dialogControlHeight,
              child: BareDropdown<_Player>(
                key: const ValueKey<String>('vector-player'),
                value: _playerShown,
                options: _Player.values,
                label: (p) => p == _Player.lottie
                    ? l10n.vectorPlayerLottie
                    : l10n.vectorPlayerBrowser,
                disabledReason: (p) => switch ((p, _settings.format)) {
                  (_Player.browser, 'dotlottie') =>
                    l10n.vectorBrowserNoDotLottie,
                  (_Player.lottie, 'svg') => l10n.vectorLottieNoSvg,
                  _ => null,
                },
                onChanged: _setPlayer,
              ),
            ),
          ),
          viewerBarGapBox(viewerTransportGap),
          _groundMark(t),
          const SizedBox(width: viewerTransportGap),
          LumitTooltip(
            message: l10n.vectorTipZoom,
            child: SizedBox(
              height: dialogControlHeight,
              child: BareDropdown<double?>(
                key: const ValueKey<String>('vector-zoom'),
                value: _zoom,
                options: const [null, 1, 2],
                label: (z) => z == null ? l10n.menuFit : '${(z * 100).round()}%',
                onChanged: (z) => _redraw(() => _zoom = z),
              ),
            ),
          ),
        ]),
      );

  /// The ground behind the picture: the Viewer's board mark, cycling through
  /// the four on a click and offering them by name on a right click.
  Widget _groundMark(LumitTheme t) {
    final rows = [
      (_Ground.board, l10n.fxCheckerboard),
      (_Ground.comp, l10n.compBackground),
      (_Ground.white, l10n.vectorWhite),
      (_Ground.black, l10n.flareBlack),
    ];
    return Builder(
      builder: (context) => GestureDetector(
        onSecondaryTapUp: (press) => showLumitPopup<void>(
          context: context,
          position: press.globalPosition,
          builder: (close) => FloatSurface(
            child: IntrinsicWidth(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final (ground, name) in rows)
                    MenuRow(
                      key: ValueKey<String>('vector-ground-${ground.name}'),
                      onPressed: () {
                        close(null);
                        _setGround(ground);
                      },
                      child: MenuLabel(name, ticked: _ground == ground),
                    ),
                ],
              ),
            ),
          ),
        ),
        child: viewerBarMark(
          key: const ValueKey<String>('vector-ground'),
          icon: LumitIcon.checkerboard,
          colour: _ground == _Ground.board ? t.textMuted : t.textPrimary,
          onPressed: () => _setGround(
              _Ground.values[(_ground.index + 1) % _Ground.values.length]),
          tip: rows.firstWhere((r) => r.$1 == _ground).$2,
          keyed: false,
        ),
      ),
    );
  }
}

/// The transparency board, the Viewer's own squares.
class _BoardPainter extends CustomPainter {
  final LumitTheme theme;
  const _BoardPainter(this.theme);

  static const double _square = 8;

  @override
  void paint(Canvas canvas, Size size) {
    final light = Paint()..color = theme.surface2;
    for (var y = 0.0; y < size.height; y += _square) {
      for (var x = 0.0; x < size.width; x += _square) {
        if (((x + y) / _square).round().isOdd) continue;
        canvas.drawRect(Rect.fromLTWH(x, y, _square, _square), light);
      }
    }
  }

  @override
  bool shouldRepaint(_BoardPainter old) => old.theme != theme;
}
