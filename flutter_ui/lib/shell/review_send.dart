// Sending a composition for review: what to send, the wait while it is
// exported and uploaded, and the link at the end of it.
//
// This is one face of the Previews window. The form is an ordinary
// dialogue. The wait and the link stand in front of the Pro wash, because
// they are the server's part of the job.
//
// The work is [ReviewsState.send]. This holds the form's choices, starts it,
// and draws how it is going. What it reads from the engine is read once, as
// the form opens.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';

import '../l10n/strings.dart';
import '../panels/timeline_extras_frb.dart' show workAreaFrames;
import '../state/account.dart';
import '../state/external_links.dart';
import '../panels/effect_param_row_frb.dart' show effectLabelOf;
import '../state/reviews.dart';
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'pro_window.dart';
import 'profile_chip.dart';
import 'sign_in_window.dart';

/// The width of the window's narrow faces, which is the Lumit Pro window's.
const double reviewNarrowWidth = proWindowWidth;
const double _labelColumn = 96;

/// Whether [words] fit where the page shows a version's note beside the
/// title, which is 32 of its noughts wide in its 12 px sans. The page's
/// sans is Lumit's own, so the same words can be measured here.
bool _fitsThePage(String words) {
  double wide(String text) {
    final painter = TextPainter(
      text: TextSpan(
          text: text,
          style: const TextStyle(
              fontFamily: LumitTheme.fontFamily, fontSize: 12)),
      textDirection: TextDirection.ltr,
    )..layout();
    final width = painter.width;
    painter.dispose();
    return width;
  }

  return wide(words) <= 32 * wide('0');
}

/// What changed in a composition, in one line short enough to stand beside
/// a version on the page. A kind of change with one thing in it names the
/// thing, and one with more counts them: Title text changed, 2 layers
/// added. The two largest kinds are said, and a word that there was more.
/// Empty for nothing.
String reviewChangedNote(ReviewChanges changes) {
  // An effect as a person would call it: their own name for it, or the
  // effect's.
  String effect(String name) =>
      name.startsWith('=') ? name.substring(1) : effectLabelOf(name);
  // Each kind: how many, its place in a tie, its words by count, and its
  // words by name for when there is one of it.
  final kinds = [
    (
      changes.changed.length,
      0,
      l10n.reviewNoteLayersChanged,
      () => l10n.reviewNoteLayerChanged(changes.changed.single),
    ),
    (
      changes.added.length,
      1,
      l10n.reviewNoteLayersAdded,
      () => l10n.reviewNoteLayerAdded(changes.added.single),
    ),
    (
      changes.removed.length,
      2,
      l10n.reviewNoteLayersRemoved,
      () => l10n.reviewNoteLayerRemoved(changes.removed.single),
    ),
    (
      changes.effectsAdded.length,
      3,
      l10n.reviewNoteEffectsAdded,
      () => l10n.reviewNoteEffectAdded(effect(changes.effectsAdded.single.$1),
          changes.effectsAdded.single.$2),
    ),
    (
      changes.effectsRemoved.length,
      4,
      l10n.reviewNoteEffectsRemoved,
      () => l10n.reviewNoteEffectRemoved(
          effect(changes.effectsRemoved.single.$1),
          changes.effectsRemoved.single.$2),
    ),
  ].where((kind) => kind.$1 > 0).toList()
    // The most of anything first, and a tie in the order above.
    ..sort((a, b) =>
        a.$1 == b.$1 ? a.$2.compareTo(b.$2) : b.$1.compareTo(a.$1));
  if (kinds.isEmpty) return '';
  String said(int at, bool named) =>
      named && kinds[at].$1 == 1 ? kinds[at].$4() : kinds[at].$3(kinds[at].$1);
  String line({required bool named, required bool both}) =>
      switch (kinds.length) {
        1 => said(0, named),
        _ when !both => l10n.reviewNoteOneMore(said(0, named)),
        2 => l10n.reviewNoteTwo(said(0, named), said(1, named)),
        _ => l10n.reviewNoteMore(said(0, named), said(1, named)),
      };
  // By name and in full where that fits the page, and else the nearest
  // thing that does: one kind and a word for the rest, then counts.
  final tries = [
    line(named: true, both: true),
    line(named: true, both: false),
    line(named: false, both: true),
    line(named: false, both: false),
  ];
  return tries.firstWhere(_fitsThePage,
      orElse: () => tries.reduce((a, b) => a.length <= b.length ? a : b));
}

/// What a failure is called in the person's own language. The codes are the
/// server's, and the two the export can fail with here.
String reviewErrorText(String code) => switch (code) {
      'pro_required' => l10n.reviewErrorPro,
      'storage_full' => l10n.reviewErrorStorage,
      'too_large' => l10n.reviewErrorTooLarge,
      'too_many_versions' => l10n.reviewErrorVersions,
      'too_many_links' => l10n.reviewErrorLinks,
      'not_found' => l10n.reviewErrorGone,
      'last_version' => l10n.reviewErrorLastVersion,
      'too_many_comments' => l10n.reviewErrorComments,
      'export_refused' => l10n.reviewErrorExportRefused,
      'export_failed' => l10n.reviewErrorExportFailed,
      _ => cloudErrorText(CloudError(code)),
    };

class ReviewSendFace extends StatefulWidget {
  final LumitUiState ui;
  final LumitState app;

  /// The composition to send. Null when the window was opened on a send
  /// already under way, which has no form to go back to.
  final CompositionReference? comp;

  /// The link a new version is meant for, when the window was asked for one
  /// of a link it was showing. Without it, the composition's own is found.
  final Review? link;

  /// Leave without sending, and close the window.
  final VoidCallback onBack, onClose;

  const ReviewSendFace({
    super.key,
    required this.ui,
    required this.app,
    required this.comp,
    this.link,
    required this.onBack,
    required this.onClose,
  });

  @override
  State<ReviewSendFace> createState() => _ReviewSendFaceState();
}

class _ReviewSendFaceState extends State<ReviewSendFace> {
  final _title = TextEditingController();
  final _note = TextEditingController();
  int _edge = 1080;
  bool _workArea = false, _comments = true, _separate = false;

  // The composition as it stood when the form opened.
  String _name = '';
  int _width = 0, _height = 0, _fpsNum = 30, _fpsDen = 1, _frames = 0;
  ({int start, int end, bool whole}) _area = (start: 0, end: 0, whole: true);
  bool _lacking = false;

  ReviewsState get _reviews => widget.ui.reviews;

  @override
  void initState() {
    super.initState();
    final comp = widget.comp;
    if (comp == null) return;
    try {
      final settings = comp.getSettings();
      _name = settings.name;
      _width = settings.width;
      _height = settings.height;
      _fpsNum = settings.fpsNum;
      _fpsDen = settings.fpsDen;
      _frames = comp.durationFrames();
      _area = workAreaFrames(comp);
      _lacking = comp.shareLacking().isNotEmpty;
    } catch (_) {
      // A composition that has gone sends nothing, and the engine says so.
    }
    _title.text = _name;
    _workArea = !_area.whole;
    // What changed since the last version, said for the sender, who can
    // say it better or not at all. A composition never sent has no last.
    final link = _reviews.link(widget.link?.id) ?? _reviews.linkOf(comp);
    if (link == null) return;
    try {
      final changes = _reviews.changedSince(link, comp);
      if (changes != null) _note.text = reviewChangedNote(changes);
    } catch (_) {
      // The composition has gone, and the line stays empty.
    }
  }

  @override
  void dispose() {
    _title.dispose();
    _note.dispose();
    super.dispose();
  }

  (int, int) get _span =>
      _workArea ? (_area.start, _area.end) : (0, _frames);

  void _create(Review? into) {
    final comp = widget.comp;
    if (comp == null) return;
    final (start, end) = _span;
    final title = _title.text.trim();
    unawaited(_reviews.send(
      comp,
      into: into,
      title: title.isEmpty
          ? _name
          : title.substring(0, math.min(title.length, 120)),
      edge: _edge,
      start: start,
      end: end,
      comments: _comments,
      // A new link has nothing to have changed from.
      note: into == null ? '' : _note.text,
      project: widget.app.project?.path() ?? '',
    ));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ListenableBuilder(
      listenable: _reviews,
      builder: (context, _) {
        final send = _reviews.sending;
        return DialogFrame(
          width: reviewNarrowWidth,
          children: [
            dialogTitleBar(
              t,
              title: l10n.reviewSendTitle,
              subject: _name,
              onClose: widget.onClose,
              keyPrefix: 'review-send',
            ),
            if (send == null || send.stage == SendStage.failed)
              ..._form(t, send?.failure)
            else if (send.stage == SendStage.done)
              _landed(t, send.review!)
            else
              _working(t, send),
          ],
        );
      },
    );
  }

  // --- The form -------------------------------------------------------------

  List<Widget> _form(LumitTheme t, String? failure) {
    final existing = widget.comp == null
        ? null
        : _reviews.link(widget.link?.id) ?? _reviews.linkOf(widget.comp!);
    final into = _separate ? null : existing;
    final (width, height) = reviewSize(_width, _height, _edge);
    final (start, end) = _span;
    return [
      Padding(
        padding: const EdgeInsets.fromLTRB(
            dialogPadding, 10, dialogPadding, dialogPadding),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (existing != null)
              // The label stands beside the first of the two lines, and not
              // between them as a row's label otherwise would.
              Padding(
                padding: const EdgeInsets.only(top: 3),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    SizedBox(
                      width: _labelColumn,
                      height: 22,
                      child: Align(
                        alignment: Alignment.centerLeft,
                        child: Text(l10n.reviewSendAs, style: t.body),
                      ),
                    ),
                    const SizedBox(width: 10),
                    Expanded(
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          _choice(
                              t,
                              'version',
                              l10n.reviewSendVersion(existing.title),
                              !_separate,
                              () => setState(() => _separate = false)),
                          _choice(
                              t,
                              'separate',
                              l10n.reviewSendSeparate,
                              _separate,
                              () => setState(() => _separate = true)),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            if (into == null)
              dialogRow(
                t,
                l10n.reviewLinkTitle,
                SizedBox(
                  height: dialogControlHeight,
                  child: HouseTextField(
                    key: const ValueKey('review-send-title'),
                    controller: _title,
                    width: double.infinity,
                    padding: const EdgeInsets.symmetric(horizontal: 8),
                    fill: t.surface0,
                    hint: _name,
                    onSubmitted: (_) => _create(into),
                  ),
                ),
                labelColumn: _labelColumn,
              )
            else
              // One line for the people watching, and nothing has to go in it.
              dialogRow(
                t,
                l10n.reviewWhatChanged,
                SizedBox(
                  height: dialogControlHeight,
                  child: HouseTextField(
                    key: const ValueKey('review-send-note'),
                    controller: _note,
                    width: double.infinity,
                    padding: const EdgeInsets.symmetric(horizontal: 8),
                    fill: t.surface0,
                    onSubmitted: (_) => _create(into),
                  ),
                ),
                labelColumn: _labelColumn,
              ),
            dialogRow(
              t,
              l10n.size,
              Row(
                children: [
                  for (final edge in const [1080, 720]) ...[
                    HouseButton(
                      key: ValueKey<String>('review-send-$edge'),
                      small: true,
                      active: _edge == edge,
                      onPressed: () => setState(() => _edge = edge),
                      child: Text('${edge}p',
                          style: _edge == edge
                              ? TextStyle(fontSize: t.small.fontSize)
                              : t.small),
                    ),
                    const SizedBox(width: 6),
                  ],
                  const Spacer(),
                  Text('$width × $height', style: dialogMono(t)),
                ],
              ),
              labelColumn: _labelColumn,
            ),
            dialogRow(
              t,
              l10n.exportSpan,
              Align(
                alignment: Alignment.centerLeft,
                child: dialogDropdown<bool>(
                  t,
                  id: 'review-send-span',
                  value: _workArea,
                  options: [if (!_area.whole) true, false],
                  label: (work) => work
                      ? l10n.exportSpanWorkArea(
                          '${_area.start}', '${_area.end}')
                      : l10n.exportSpanWholeComp,
                  onChanged: (work) => setState(() => _workArea = work),
                ),
              ),
              labelColumn: _labelColumn,
            ),
            if (into == null)
              dialogRow(
                t,
                l10n.reviewReviewers,
                Row(
                  children: [
                    HouseToggle(
                      key: const ValueKey('review-send-comments'),
                      value: _comments,
                      onChanged: (on) => setState(() => _comments = on),
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                        child: Text(l10n.reviewCanComment, style: t.small)),
                  ],
                ),
                labelColumn: _labelColumn,
              ),
            if (_lacking) _line(t, l10n.reviewLacking),
            if (failure != null) _line(t, reviewErrorText(failure)),
          ],
        ),
      ),
      dialogFooter(
        t,
        // How long what is sent will run.
        summary: timecodeOfRate(end - start, _fpsNum, _fpsDen),
        keyPrefix: 'review-send',
        actions: [
          HouseButton(
            key: const ValueKey('review-send-cancel'),
            onPressed: widget.onBack,
            child: Text(l10n.cancel),
          ),
          dialogFilledAction(
            t,
            key: const ValueKey('review-send-create'),
            label: into == null ? l10n.reviewCreate : l10n.reviewAddVersion,
            onPressed: end > start ? () => _create(into) : null,
          ),
        ],
      ),
    ];
  }

  Widget _choice(LumitTheme t, String id, String words, bool chosen,
          VoidCallback pick) =>
      GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: pick,
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 22),
          child: Row(
            children: [
              HouseRadio(
                key: ValueKey<String>('review-send-$id'),
                selected: chosen,
                onChanged: pick,
              ),
              const SizedBox(width: 6),
              Expanded(
                child: Text(words,
                    style: t.small.copyWith(
                        color: chosen ? t.textPrimary : t.textSecondary),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis),
              ),
            ],
          ),
        ),
      );

  Widget _line(LumitTheme t, String words) => Padding(
        padding: const EdgeInsets.only(top: 6),
        child: Text(words,
            key: const ValueKey('review-send-error'),
            style: t.small.copyWith(color: t.warning, height: 1.4)),
      );

  // --- The wait, and the link -----------------------------------------------

  /// The ground the wait and the link stand on: the Pro wash, a little
  /// brighter once the link has landed.
  Widget _washed(LumitTheme t, {required bool landed, required Widget child}) =>
      ClipRect(
        child: CustomPaint(
          painter: ProWashPainter(t.surface1, landed ? 0.22 : 0.16),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(22, 30, 22, 22),
            child: child,
          ),
        ),
      );

  Widget _working(LumitTheme t, ReviewSend send) {
    final exporting = send.stage == SendStage.exporting;
    final heading = send.waiting
        ? l10n.reviewWaiting
        : exporting
            ? l10n.reviewExporting
            : l10n.reviewUploading;
    return _washed(
      t,
      landed: false,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          AnimatedSwitcher(
            duration: ThemeScope.of(context).motion.swap.duration,
            child: Text(
              heading,
              key: ValueKey<String>(heading),
              textAlign: TextAlign.center,
              style: t.heading.copyWith(fontSize: 20, height: 1.2),
            ),
          ),
          const SizedBox(height: 8),
          Text(
            // Nothing to count until the exporter has said how many there are.
            send.total <= 0 || send.waiting
                ? ''
                : exporting
                    ? l10n.reviewFrames(send.done, send.total)
                    : l10n.reviewOf(
                        writtenBytes(send.done), writtenBytes(send.total)),
            key: const ValueKey('review-send-count'),
            textAlign: TextAlign.center,
            style: t.mono.copyWith(fontSize: 11, color: t.textSecondary),
          ),
          const SizedBox(height: 20),
          HouseProgressBar(
            fraction: send.total <= 0 ? 0 : send.done / send.total,
            fill: proSweep,
          ),
          const SizedBox(height: 22),
          Center(
            child: SizedBox(
              height: 26,
              child: HouseButton(
                key: const ValueKey('review-send-stop'),
                padding: const EdgeInsets.symmetric(horizontal: 18),
                onPressed: _reviews.cancelSend,
                child: Text(l10n.cancel),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _landed(LumitTheme t, Review review) {
    final scope = ThemeScope.of(context);
    Widget action(String id, String label, VoidCallback onPressed,
            {bool primary = false}) =>
        Expanded(
          child: SizedBox(
            height: 30,
            child: HouseButton(
              key: ValueKey<String>('review-send-$id'),
              primary: primary,
              onPressed: onPressed,
              child: Text(label),
            ),
          ),
        );
    return _washed(
      t,
      landed: true,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Center(
            // The tick draws itself once, and is simply there when
            // animation is off.
            child: TweenAnimationBuilder<double>(
              tween: Tween(begin: 0, end: 1),
              duration: scope.animationLevel == AnimationLevel.none
                  ? Duration.zero
                  : const Duration(milliseconds: 700),
              curve: Curves.easeOutCubic,
              builder: (context, drawn, _) => CustomPaint(
                size: const Size.square(44),
                painter: _TickPainter(drawn, t.hairlineStrong),
              ),
            ),
          ),
          const SizedBox(height: 16),
          Text(
            l10n.reviewCopied,
            textAlign: TextAlign.center,
            style: t.heading.copyWith(fontSize: 20, height: 1.2),
          ),
          const SizedBox(height: 14),
          reviewLinkWell(t, review.url, centred: true),
          const SizedBox(height: 18),
          Row(
            children: [
              action('open', l10n.reviewOpen,
                  () => unawaited(openExternalLink(review.ownerUrl))),
              const SizedBox(width: 8),
              action('done', l10n.done, () {
                _reviews.clearSend();
                widget.onClose();
              }, primary: true),
            ],
          ),
        ],
      ),
    );
  }
}

/// A link's address, in mono, in a well of its own.
Widget reviewLinkWell(LumitTheme t, String url, {bool centred = false}) =>
    Container(
      height: 26,
      alignment: centred ? Alignment.center : Alignment.centerLeft,
      padding: const EdgeInsets.symmetric(horizontal: 10),
      decoration: BoxDecoration(
        color: t.surface0,
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        border: Border.all(color: t.hairline),
      ),
      child: Text(
        url,
        key: const ValueKey('review-link'),
        style: t.mono.copyWith(fontSize: 11, color: t.textPrimary),
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
      ),
    );

/// A ring and the tick inside it, in the sweep, drawn as far as [drawn] says:
/// the ring first, then the tick.
class _TickPainter extends CustomPainter {
  final double drawn;
  final Color track;
  const _TickPainter(this.drawn, this.track);

  @override
  void paint(Canvas canvas, Size size) {
    final box = (Offset.zero & size).deflate(1);
    final pen = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;
    canvas.drawOval(box, pen..color = track);
    pen.shader = proSweep.createShader(box);
    final ring = (drawn / 0.6).clamp(0.0, 1.0);
    canvas.drawArc(box, -math.pi / 2, math.pi * 2 * ring, false, pen);
    final tick = ((drawn - 0.45) / 0.55).clamp(0.0, 1.0);
    if (tick <= 0) return;
    final w = size.width;
    final whole = Path()
      ..moveTo(w * 0.31, w * 0.52)
      ..lineTo(w * 0.44, w * 0.65)
      ..lineTo(w * 0.70, w * 0.37);
    final measured = whole.computeMetrics().first;
    canvas.drawPath(
        measured.extractPath(0, measured.length * tick), pen..strokeWidth = 2);
  }

  @override
  bool shouldRepaint(_TickPainter old) =>
      old.drawn != drawn || old.track != track;
}
