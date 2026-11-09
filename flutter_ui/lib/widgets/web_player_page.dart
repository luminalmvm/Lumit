/// The page: lottie-web for a Lottie, the SVG itself for an SVG, and the
/// transport's events in. `window.lumit` is put there before this script
/// runs: by the extension page host on the desktop, and by web_player_web.dart
/// in the browser.
const String webPlayerPage = r'''<!doctype html>
<html><head><meta charset="utf-8">
<style>
html,body{margin:0;height:100%;overflow:hidden;background:transparent}
#s{width:100%;height:100%}
#s svg{width:100%;height:100%;display:block}
</style></head>
<body><div id="s"></div>
<script src="lottie_svg.min.js"></script>
<script>
(function () {
  var stage = document.getElementById('s');
  var anim = null, svg = null, fps = 30, frames = 1, loop = true;
  var playing = false, raf = 0, last = -1;
  function report(f) {
    if (f === last) return;
    last = f;
    lumit.call('frame', { frame: f });
  }
  function tick() {
    if (!svg || !playing) return;
    var f = Math.floor(svg.getCurrentTime() * fps);
    if (!loop && f >= frames) {
      report(frames - 1);
      ended();
      return;
    }
    report(f % frames);
    raf = requestAnimationFrame(tick);
  }
  function ended() {
    pause();
    lumit.call('ended', {});
  }
  function ground(css) { document.body.style.background = css || 'transparent'; }
  function clear() {
    if (anim) { anim.destroy(); anim = null; }
    svg = null;
    stage.innerHTML = '';
    cancelAnimationFrame(raf);
    last = -1;
  }
  function play() {
    playing = true;
    if (anim) anim.play();
    if (svg) { svg.unpauseAnimations(); cancelAnimationFrame(raf); raf = requestAnimationFrame(tick); }
  }
  function pause() {
    playing = false;
    if (anim) anim.pause();
    if (svg) { svg.pauseAnimations(); cancelAnimationFrame(raf); }
  }
  function seek(f) {
    if (anim) { if (playing) anim.goToAndPlay(f, true); else anim.goToAndStop(f, true); }
    if (svg) { svg.setCurrentTime(f / fps); report(f); }
  }
  function load(d) {
    clear();
    ground(d.ground);
    fps = d.fps; frames = d.frames; loop = d.loop; playing = d.playing;
    if (d.kind === 'svg') {
      fetch(d.path).then(function (r) { return r.text(); }).then(function (text) {
        stage.innerHTML = text;
        svg = stage.querySelector('svg');
        if (!svg) return;
        svg.setAttribute('width', '100%');
        svg.setAttribute('height', '100%');
        svg.pauseAnimations();
        svg.setCurrentTime(d.frame / fps);
        lumit.call('ready', {});
        if (playing) play();
      });
      return;
    }
    anim = lottie.loadAnimation({
      container: stage, renderer: 'svg', loop: loop, autoplay: false, path: d.path,
      rendererSettings: { preserveAspectRatio: 'xMidYMid meet' }
    });
    anim.addEventListener('DOMLoaded', function () {
      lumit.call('ready', {});
      if (playing) anim.goToAndPlay(d.frame, true); else anim.goToAndStop(d.frame, true);
    });
    anim.addEventListener('enterFrame', function () { report(Math.floor(anim.currentFrame)); });
    anim.addEventListener('complete', function () { if (!loop) ended(); });
  }
  function start() {
    lumit.on('load', load);
    lumit.on('play', play);
    lumit.on('pause', pause);
    lumit.on('seek', function (d) { seek(d.frame); });
    lumit.on('loop', function (d) { loop = d.on; if (anim) anim.loop = d.on; });
    lumit.on('ground', function (d) { ground(d.css); });
    lumit.call('up', {});
  }
  if (window.lumit) start(); else window.addEventListener('lumitready', start);
})();
</script></body></html>
''';
