// The shell's bottom status line: what the engine is doing right now.
//
// One quiet strip under the dock (docs/07-UI-SPEC.md §1), left to right:
// whether the document is saved, the cache meter (how full the rendered-frame
// store is, with the exact megabytes), the latest notice with its close
// button, and the background jobs — a MAKE-PROXY transcode and an export, each
// with its progress and a Cancel that works from anywhere, not only with the
// dialogue open. Both engine polls latch their state between calls, so this and
// the export dialogue can both ask without stealing each other's answer.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show pluginMessages;
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'package:lumit_flutter/panels/timeline_timings.dart';

import 'cache_confirm_frb.dart';

/// Bumped by whatever starts an export — the export dialogue, the console's
/// snapshot — so the strip knows to start polling. Without a start signal the
/// only way to notice an export is to ask the engine on a timer all session,
/// which cost the idle strip ~12 bridge calls a second for the answer "no".
final ValueNotifier<int> statusLineExportStarted = ValueNotifier<int>(0);

/// The same signal for **MAKE-PROXY** (K-501), which is background work of
/// exactly the same shape — start, poll, cancel — and so is surfaced in
/// exactly the same place rather than in a progress bar of the Project
/// panel's own.
///
/// Bumped twice per job, and by two different callers: by the Project panel
/// when it starts one, so the strip knows to begin polling, and by the strip
/// itself when the job stops running, so the panel re-reads the item that just
/// gained a proxy. One notifier rather than two, because "the proxy job moved"
/// is the only thing either side is asking about.
final ValueNotifier<int> proxyJobChanged = ValueNotifier<int>(0);

class StatusLineFrb extends StatefulWidget {
  /// The poll seam, injected by tests so no engine has to run an export.
  final BridgeExportState Function()? poll;

  /// The same seam for the proxy job.
  final BridgeProxyState Function()? proxyPollFn;

  /// The same seam for what plugins have asked Lumit to say, so a test can
  /// raise a plugin message without a plugin.
  final List<String> Function()? pluginMessagesFn;

  const StatusLineFrb(
      {super.key, this.poll, this.proxyPollFn, this.pluginMessagesFn});

  @override
  State<StatusLineFrb> createState() => _StatusLineFrbState();
}

class _StatusLineFrbState extends State<StatusLineFrb> {
  BridgeExportState _export = const BridgeExportState.idle();
  BridgeProxyState _proxy = const BridgeProxyState.idle();
  Timer? _timer;

  /// One pending repaint for a cache bump that arrived while the tick was
  /// off, so a burst of banked frames coalesces into a single redraw.
  Timer? _bump;

  late final LumitUiState _ui;

  /// The shell state the notices go on. Read here rather than through
  /// [LumitUiState] because a notice is the application's, not the interface's.
  late final LumitState _app;

  @override
  void initState() {
    super.initState();
    _ui = context.read<LumitUiState>();
    _app = context.read<LumitState>();
    // One poll up front: a strip mounted over a running export (a hot reload
    // mid-export, say) has to pick it up without waiting for a start signal.
    _export = (widget.poll ?? exportPoll)();
    _proxy = (widget.proxyPollFn ?? proxyPoll)();
    statusLineExportStarted.addListener(_tick);
    proxyJobChanged.addListener(_tick);
    // Playback fills the caches, so the meter ticks while it runs.
    _ui.playing.addListener(_tick);
    // A frame banked or delivered outside playback — the idle fill, a scrub —
    // moves the meter too, coalesced to one repaint per half second.
    _ui.cacheChanged.addListener(_bumped);
    _ui.frameArrived.addListener(_bumped);
    _syncTimer();
  }

  /// Half a second is fast enough to feel live on a bar this small. Each tick
  /// redraws the whole strip: the export poll, the dirty flag and the cache
  /// numbers are all sync reads of a few held values, and the strip is 20
  /// pixels of mostly text.
  void _tick() {
    _export = (widget.poll ?? exportPoll)();
    final was = _proxy is BridgeProxyState_Running;
    _proxy = (widget.proxyPollFn ?? proxyPoll)();
    _sayWhatThePluginsSaid();
    _syncTimer();
    if (mounted) setState(() {});
    // The job stopped running on this tick, so the item it belonged to has
    // just gained its proxy (the poll is what attaches it). Told after the
    // repaint, and only on the edge, so a panel listening does one re-read per
    // job rather than one per tick.
    if (was && _proxy is! BridgeProxyState_Running) proxyJobChanged.value++;
  }

  /// Whatever plugins asked Lumit to say since the last tick, as calm notices
  /// (docs/12 §2.2).
  ///
  /// **Never modal, and never a dialogue.** An OFX plugin may raise an error, a
  /// warning or a question at any moment, including in the middle of a render;
  /// a modal box in the middle of playback would be the worst possible answer,
  /// and a question has already been told "you decide" at the suite. Until the
  /// owner says what the Message suite should look like, this is the whole of
  /// it: the last message wins the notice line, exactly as every other quiet
  /// message does.
  ///
  /// Drained here because this is the tick that runs while a comp is playing,
  /// which is when a plugin is speaking. A session with no plugins takes one
  /// sync read that answers with an empty list.
  void _sayWhatThePluginsSaid() {
    final said = (widget.pluginMessagesFn ?? pluginMessages)();
    if (said.isEmpty) return;
    for (final message in said) {
      _app.postNotice(message);
    }
  }

  /// The tick runs only while something on the strip is actually moving: an
  /// export in flight, or playback filling the caches. Everything else that
  /// changes the strip announces itself — document edits notify the shell
  /// state, notices are a ValueListenable, banked frames bump
  /// [LumitUiState.cacheChanged] — so an idle strip costs no bridge calls.
  void _syncTimer() {
    if (_export is BridgeExportState_Running ||
        _proxy is BridgeProxyState_Running ||
        _ui.playing.value) {
      _timer ??=
          Timer.periodic(const Duration(milliseconds: 500), (_) => _tick());
    } else {
      _timer?.cancel();
      _timer = null;
    }
  }

  /// A cache bump while the tick is off: one coalesced repaint.
  void _bumped() {
    if (_timer != null || _bump != null || !mounted) return;
    _bump = Timer(const Duration(milliseconds: 500), () {
      _bump = null;
      if (mounted) setState(() {});
    });
  }

  @override
  void dispose() {
    statusLineExportStarted.removeListener(_tick);
    proxyJobChanged.removeListener(_tick);
    _ui.playing.removeListener(_tick);
    _ui.cacheChanged.removeListener(_bumped);
    _ui.frameArrived.removeListener(_bumped);
    _timer?.cancel();
    _bump?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final state = Provider.of<LumitState>(context);
    return Container(
      height: 20,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      child: Row(
        key: const ValueKey('status-line'),
        children: [
          _savedState(t, state),
          _divider(t),
          // Deliberately NOT const: a const child is skipped by the tick's
          // rebuild, which froze the meter at whatever it first read. Three
          // sync stat reads a second is the whole cost of keeping it live.
          //
          // Inside a horizontal scroll view that cannot be scrolled: the meter
          // is three tiers wide now, and on a window too narrow for all of them
          // the last one should be cut off quietly. A plain Row would report an
          // overflow instead, which is a striped warning across the strip.
          Flexible(
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              physics: const NeverScrollableScrollPhysics(),
              // ignore: prefer_const_constructors
              child: CacheMeterFrb(),
            ),
          ),
          _divider(t),
          // The render-time switch, beside the meters for the same reason they
          // are here: it governs the whole session and it costs something to
          // have on (K-276). It began life as a glyph in the Timeline's column
          // header, which is where nobody found it.
          const RenderTimingsToggle(),
          _divider(t),
          Expanded(
            child: Row(
              children: [
                Flexible(child: _notice(t, state)),
                const Spacer(),
                ..._proxySection(t),
                ..._exportSection(t),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _divider(LumitTheme t) => Container(
        width: 1,
        height: 12,
        margin: const EdgeInsets.symmetric(horizontal: 8),
        color: t.hairline,
      );

  /// Saved / unsaved, at the far left. Being unsaved is a fact, not a fault,
  /// so it reads in the ordinary text colour — the muted tint is for the
  /// states where nothing is at risk.
  Widget _savedState(LumitTheme t, LumitState state) {
    final project = state.project;
    final dirty = project?.isDirty() ?? false;
    final label = project == null
        ? l10n.noProject
        : dirty
            ? l10n.unsavedChanges
            : project.path() == null
                ? l10n.notSavedYet
                : l10n.saved;
    return Text(
      label,
      key: const ValueKey('status-saved'),
      style: dirty ? t.small : t.small.copyWith(color: t.textMuted),
    );
  }

  /// The latest notice, with the close button every notice carries.
  Widget _notice(LumitTheme t, LumitState state) {
    return ValueListenableBuilder<LumitNotice?>(
      valueListenable: state.notice,
      builder: (context, notice, _) {
        if (notice == null) return const SizedBox.shrink();
        return Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Flexible(
              child: Text(
                notice.message,
                key: const ValueKey('status-notice'),
                style:
                    notice.error ? t.small.copyWith(color: t.warning) : t.small,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            const SizedBox(width: 4),
            HouseButton(
              key: const ValueKey('status-notice-close'),
              small: true,
              frameless: true,
              onPressed: () => state.notice.value = null,
              child: Text('×', style: t.small.copyWith(color: t.textMuted)),
            ),
          ],
        );
      },
    );
  }

  /// The running MAKE-PROXY, drawn exactly as the export beside it is
  /// (K-501): what it is doing, how far along, and a Cancel that works from
  /// anywhere. It sits **before** the export section, so a long export that
  /// starts later never pushes the shorter job off the strip.
  List<Widget> _proxySection(LumitTheme t) => switch (_proxy) {
        BridgeProxyState_Idle() => const [],
        BridgeProxyState_Running(:final frame, :final total) => [
            Flexible(
              child: Text(
                l10n.makingProxyFrame('$frame', '$total'),
                key: const ValueKey('status-proxy-progress'),
                style: t.small,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            const SizedBox(width: 8),
            SizedBox(
              width: 120,
              child: HouseProgressBar(
                fraction: total == BigInt.zero
                    ? 0.0
                    : frame.toDouble() / total.toDouble(),
              ),
            ),
            const SizedBox(width: 8),
            HouseButton(
              key: const ValueKey('status-proxy-cancel'),
              small: true,
              frameless: true,
              onPressed: proxyCancel,
              child: Text(l10n.cancel, style: t.small),
            ),
            const SizedBox(width: 8),
          ],
        BridgeProxyState_Done(:final path) => [
            Flexible(
              child: Text(
                l10n.proxyMade(path),
                key: const ValueKey('status-proxy-done'),
                style: t.small,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            const SizedBox(width: 8),
          ],
        BridgeProxyState_Failed(:final error) => [
            Flexible(
              child: Text(
                error == 'cancelled'
                    ? l10n.proxyCancelled
                    : l10n.proxyFailed(error),
                key: const ValueKey('status-proxy-failed'),
                style: t.small.copyWith(color: t.warning),
                overflow: TextOverflow.ellipsis,
              ),
            ),
            const SizedBox(width: 8),
          ],
      };

  List<Widget> _exportSection(LumitTheme t) => switch (_export) {
        BridgeExportState_Idle() => const [],
        BridgeExportState_Running(:final frame, :final total, :final encoder) =>
          [
            Flexible(
              child: Text(
                l10n.exportingFrame('$frame', '$total', encoder),
                key: const ValueKey('status-export-progress'),
                style: t.small,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            const SizedBox(width: 8),
            SizedBox(
              width: 120,
              child: HouseProgressBar(
                fraction: total == BigInt.zero
                    ? 0.0
                    : frame.toDouble() / total.toDouble(),
              ),
            ),
            const SizedBox(width: 8),
            HouseButton(
              key: const ValueKey('status-export-cancel'),
              small: true,
              frameless: true,
              onPressed: exportCancel,
              child: Text(l10n.cancel, style: t.small),
            ),
          ],
        BridgeExportState_Done(:final path) => [
            Flexible(
              child: Text(
                l10n.exportedTo(path),
                key: const ValueKey('status-export-done'),
                style: t.small,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ],
        BridgeExportState_Failed(:final error) => [
            Flexible(
              child: Text(
                error == 'cancelled'
                    ? l10n.exportCancelled
                    : l10n.exportFailed(error),
                key: const ValueKey('status-export-failed'),
                style: t.small.copyWith(color: t.warning),
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ],
      };
}

/// How full each tier of the frame cache is — one bar per tier, with the
/// megabytes beside it. Clicking a tier's bar empties that tier (the disk one
/// asks first: it deletes files rather than costing a re-render).
///
/// **Why one bar each.** The three tiers hold different things and fill at
/// different rates: the card's cache fills first and fastest, memory takes what
/// the card evicts, and disk takes what memory does — so a merged number cannot
/// answer "what is cached" for any of them, and does not try to. (Before the
/// demotion ladder the RAM tier was only the Scopes' own, and a Viewer busily
/// banking frames on the card reported "nothing held" here and looked broken.)
///
/// Lives on the status line rather than under the Timeline: it measures the
/// whole store, not one comp's frames. Redrawn on the line's own half-second
/// tick rather than per paint — the lock `cacheStats` takes is the one a
/// render holds.
///
/// Named a *meter*, not a bar: the **cache bar** is the stripe under the time
/// ruler showing which frames are held (`TimelineCacheBar`, and the glossary's
/// own definition). This measures how full the store is, which is a different
/// question.
class CacheMeterFrb extends StatelessWidget {
  const CacheMeterFrb({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ram = cacheStats();
    final vram = vramCacheStats();
    final disk = diskCacheStats();
    final requests = ram.hits.toInt() + ram.misses.toInt();

    return Row(
      key: const ValueKey('cache-meter'),
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(l10n.cache, style: t.small.copyWith(color: t.textMuted)),
        const SizedBox(width: 6),
        _TierMeter(
          keyName: 'cache-meter-ram',
          label: l10n.ramTier,
          used: ram.usedBytes.toInt(),
          budget: ram.budgetBytes.toInt(),
          tip: requests == 0
              ? l10n.tipCacheEmpty
              : l10n.tipCacheRam('${ram.hits}', '${ram.misses}'),
          onClear: clearCache,
        ),
        const SizedBox(width: 8),
        _TierMeter(
          keyName: 'cache-meter-vram',
          label: l10n.vramTier,
          used: vram.usedBytes.toInt(),
          budget: vram.budgetBytes.toInt(),
          tip: l10n.tipCacheVram,
          onClear: clearVramCache,
        ),
        const SizedBox(width: 8),
        _TierMeter(
          keyName: 'cache-meter-disk',
          label: l10n.diskTier,
          used: disk.usedBytes.toInt(),
          budget: disk.budgetBytes.toInt(),
          tip: disk.root.isEmpty
              ? l10n.tipCacheDiskNone
              : l10n.tipCacheDisk(disk.root),
          // The one tier whose clear destroys files rather than costing a
          // re-render, so it asks first (docs/07 §15).
          onClear: () => confirmClearDiskCache(context),
        ),
      ],
    );
  }
}

/// One tier: its name, how full it is, and the megabytes. Its own widget, so all
/// three tiers are one layout rather than three copies of it.
class _TierMeter extends StatelessWidget {
  final String keyName;
  final String label;
  final int used;
  final int budget;
  final String tip;
  final VoidCallback onClear;

  const _TierMeter({
    required this.keyName,
    required this.label,
    required this.used,
    required this.budget,
    required this.tip,
    required this.onClear,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final fraction = budget <= 0 ? 0.0 : (used / budget).clamp(0.0, 1.0);
    return LumitTooltip(
      message: tip,
      child: GestureDetector(
        key: ValueKey<String>(keyName),
        behavior: HitTestBehavior.opaque,
        onTap: onClear,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(label, style: t.small.copyWith(color: t.textMuted)),
            const SizedBox(width: 4),
            SizedBox(width: 32, child: HouseProgressBar(fraction: fraction)),
            const SizedBox(width: 4),
            // Used only: the budget is in the tooltip and in Settings, and the
            // status line is one line shared with the notices and export.
            Text('${_mibText(used)} MB',
                style: t.small.copyWith(color: t.textMuted)),
          ],
        ),
      ),
    );
  }
}

/// Bytes as whole megabytes, for the meter's readouts and its tooltips.
String _mibText(int bytes) => (bytes / (1 << 20)).toStringAsFixed(0);
