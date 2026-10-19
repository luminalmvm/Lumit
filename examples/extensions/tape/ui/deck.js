// The Tape deck: four knobs and a switch drawn on one canvas, driving the
// Tape effect's own rows through window.lumit. The rows hold the values;
// the deck only shows them and asks for changes.

'use strict';

const KNOBS = ['wobble', 'bleed', 'noise', 'scanlines'];
const NAMES = { wobble: 'Wobble', bleed: 'Bleed', noise: 'Noise', scanlines: 'Scanlines' };
const STYLE = 'style';

// The deck is drawn at this size and scaled to fit the panel.
const W = 420;
const H = 260;

const DOOR = { x: 16, y: 16, w: 276, h: 126 };
const PANEL = { x: 302, y: 16, w: 102, h: 126 };
const SWITCH = { x: 302, y: 16, w: 102, h: 86 };
const KNOB_Y = 194;
const KNOB_R = 25;
const knobX = (i) => 62 + i * 98;
// How far a knob turns either side of straight up.
const SWEEP = Math.PI * 0.75;

const FONT = '"Segoe UI", system-ui, -apple-system, "Helvetica Neue", Arial, sans-serif';
const MONO = 'ui-monospace, "Cascadia Mono", Consolas, Menlo, monospace';
const INK = '#ff8a3d';
const CREAM = '#efe6cf';
const MUTED = '#6f695f';

const canvas = document.getElementById('deck');
const ctx = canvas.getContext('2d');

let rows = null; // by id, while an instance is followed
let held = null; // the knob being turned
let shown = null; // the knob the counter shows
let queued = null; // a value waiting for the next frame
let sending = false;
let reel = 0; // how far the reels have turned
let spin = 0; // and how fast they're turning
let ticking = false;
let view = { scale: 1, x: 0, y: 0, dpr: 1 };

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
const numeric = (row) => !!row && typeof row.value === 'number';

// Where a row's value sits between its ends, 0 to 1.
function travel(row) {
  if (!numeric(row)) return 0;
  const lo = row.min ?? 0;
  const hi = row.max ?? 1;
  return hi === lo ? 0 : clamp((row.value - lo) / (hi - lo), 0, 1);
}

function format(row) {
  if (!numeric(row)) return '---';
  const v = row.value;
  return Math.abs(v) < 10 && row.kind !== 'int' ? v.toFixed(1) : String(Math.round(v));
}

// ---------------------------------------------------------------- talking

function call(method, params) {
  if (!window.lumit) return Promise.reject(new Error('no lumit'));
  return window.lumit.call(method, params);
}

async function load() {
  try {
    const list = await call('effect.rows');
    rows = Object.fromEntries(list.map((row) => [row.id, row]));
  } catch (error) {
    rows = null;
  }
  held = null;
  draw();
}

function listen() {
  window.lumit.on('effect.changed', ({ values }) => {
    if (!rows) return;
    for (const [id, value] of Object.entries(values || {})) {
      const row = rows[id];
      if (!row || (held && held.id === id)) continue;
      if (numeric(row) && typeof value === 'number') {
        wind((value - row.value) / ((row.max ?? 1) - (row.min ?? 0) || 1));
      }
      row.value = value;
    }
    draw();
  });
  window.lumit.on('effect.target', ({ id }) => {
    held = null;
    if (id == null) {
      rows = null;
      draw();
    } else {
      load();
    }
  });
  load();
}

function send() {
  if (!queued) return;
  const { id, value } = queued;
  queued = null;
  call('effect.set', { id, value }).catch(() => {});
}

// ---------------------------------------------------------------- hands

function toDeck(event) {
  const r = canvas.getBoundingClientRect();
  return {
    x: (event.clientX - r.left - view.x) / view.scale,
    y: (event.clientY - r.top - view.y) / view.scale,
  };
}

function knobAt(p) {
  return KNOBS.find((id, i) => Math.hypot(p.x - knobX(i), p.y - KNOB_Y) <= KNOB_R + 10) ?? null;
}

function inside(p, box) {
  return p.x >= box.x && p.x <= box.x + box.w && p.y >= box.y && p.y <= box.y + box.h;
}

canvas.addEventListener('pointerdown', (event) => {
  if (!rows || event.button !== 0) return;
  const p = toDeck(event);
  const id = knobAt(p);
  if (id && numeric(rows[id])) {
    canvas.setPointerCapture(event.pointerId);
    held = { id, y: event.clientY, t: travel(rows[id]) };
    shown = id;
    call('effect.begin', { id }).catch(() => {});
    draw();
    return;
  }
  const style = rows[STYLE];
  if (inside(p, SWITCH) && numeric(style) && style.choices) {
    style.value = (style.value + 1) % style.choices.length;
    call('effect.set', { id: STYLE, value: style.value }).catch(() => {});
    wind(0.4);
    draw();
  }
});

canvas.addEventListener('pointermove', (event) => {
  if (!held) {
    const p = toDeck(event);
    canvas.style.cursor = !rows ? 'default' : knobAt(p) ? 'ns-resize' : inside(p, SWITCH) ? 'pointer' : 'default';
    return;
  }
  const row = rows && rows[held.id];
  if (!row) return;
  // 160 pixels of drag from one end to the other, ten times finer with Shift.
  held.t = clamp(held.t + (held.y - event.clientY) / (event.shiftKey ? 1600 : 160), 0, 1);
  held.y = event.clientY;
  let value = (row.min ?? 0) + held.t * ((row.max ?? 1) - (row.min ?? 0));
  value = row.kind === 'int' ? Math.round(value) : Math.round(value * 100) / 100;
  wind((held.t - travel(row)) * 3);
  row.value = value;
  queued = { id: held.id, value };
  if (!sending) {
    sending = true;
    requestAnimationFrame(() => {
      sending = false;
      send();
    });
  }
  draw();
});

function release() {
  if (!held) return;
  send();
  held = null;
  call('effect.end').catch(() => {});
  draw();
}

canvas.addEventListener('pointerup', release);
canvas.addEventListener('pointercancel', release);
canvas.addEventListener('lostpointercapture', release);

// The reels turn when anything moves, and coast to a stop.
function wind(amount) {
  spin = clamp(spin + amount * 0.5, -0.5, 0.5);
  if (!ticking) {
    ticking = true;
    requestAnimationFrame(tick);
  }
}

function tick() {
  reel += spin;
  spin *= 0.93;
  if (Math.abs(spin) < 0.002) {
    spin = 0;
    ticking = false;
  } else {
    requestAnimationFrame(tick);
  }
  draw();
}

// ---------------------------------------------------------------- drawing

function shape(x, y, w, h, r, path = new Path2D()) {
  path.moveTo(x + r, y);
  path.arcTo(x + w, y, x + w, y + h, r);
  path.arcTo(x + w, y + h, x, y + h, r);
  path.arcTo(x, y + h, x, y, r);
  path.arcTo(x, y, x + w, y, r);
  path.closePath();
  return path;
}

function disc(x, y, r, fill) {
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  ctx.fillStyle = fill;
  ctx.fill();
}

function text(words, x, y, font, fill, align = 'center', spacing = '0px') {
  ctx.font = font;
  ctx.fillStyle = fill;
  ctx.textAlign = align;
  ctx.letterSpacing = spacing;
  ctx.fillText(words, x, y);
  ctx.letterSpacing = '0px';
}

function screw(x, y, turn) {
  const g = ctx.createRadialGradient(x - 1, y - 1, 0.5, x, y, 4);
  g.addColorStop(0, '#8d8981');
  g.addColorStop(1, '#3b3935');
  disc(x, y, 3.6, g);
  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(turn);
  ctx.fillStyle = '#1c1b19';
  ctx.fillRect(-2.6, -0.5, 5.2, 1);
  ctx.restore();
}

function body() {
  const outline = shape(3, 3, W - 6, H - 6, 14);
  const g = ctx.createLinearGradient(0, 0, 0, H);
  g.addColorStop(0, '#3b3833');
  g.addColorStop(0.55, '#2a2824');
  g.addColorStop(1, '#1d1b18');
  ctx.fillStyle = g;
  ctx.fill(outline);
  // Brushed: faint lines from a fixed sequence, the same every time.
  ctx.save();
  ctx.clip(outline);
  let s = 7;
  for (let i = 0; i < 160; i++) {
    s = (s * 16807) % 2147483647;
    ctx.fillStyle = s & 1 ? 'rgba(255,255,255,0.025)' : 'rgba(0,0,0,0.06)';
    ctx.fillRect(0, (s % 2600) / 10, W, 1);
  }
  ctx.restore();
  ctx.strokeStyle = 'rgba(255,255,255,0.08)';
  ctx.lineWidth = 1;
  ctx.stroke(shape(3.5, 3.5, W - 7, H - 7, 14));
  ctx.fillStyle = 'rgba(0,0,0,0.22)';
  ctx.fill(shape(22, 152, W - 44, 98, 10));
  screw(11, 11, 0.4);
  screw(W - 11, 11, 1.9);
  screw(11, H - 11, 2.7);
  screw(W - 11, H - 11, 0.9);
}

function spool(x, y, r, turn) {
  const g = ctx.createRadialGradient(x, y, 6, x, y, r);
  g.addColorStop(0, '#24180f');
  g.addColorStop(1, '#4d3525');
  disc(x, y, r, g);
  ctx.strokeStyle = 'rgba(0,0,0,0.22)';
  ctx.lineWidth = 1;
  for (let ring = 11; ring < r; ring += 3) {
    ctx.beginPath();
    ctx.arc(x, y, ring, 0, Math.PI * 2);
    ctx.stroke();
  }
  disc(x, y, 8.5, '#e7e2d6');
  disc(x, y, 6, '#141416');
  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(turn);
  ctx.fillStyle = '#e7e2d6';
  for (let i = 0; i < 6; i++) {
    ctx.rotate(Math.PI / 3);
    ctx.fillRect(-1.1, -6.2, 2.2, 3);
  }
  ctx.restore();
}

function cassette() {
  ctx.fillStyle = '#232326';
  ctx.fill(shape(30, 27, 248, 108, 6));
  // The reels, through the window in the label. The tape winds from one to
  // the other as the knobs go up.
  const hole = shape(92, 68, 124, 32, 5);
  const used = KNOBS.reduce((sum, id) => sum + travel(rows[id]), 0) / KNOBS.length;
  ctx.save();
  ctx.clip(hole);
  ctx.fillStyle = '#101012';
  ctx.fillRect(92, 68, 124, 32);
  spool(122, 84, 30 - 14 * used, reel);
  spool(186, 84, 16 + 14 * used, reel * 1.4);
  ctx.restore();
  ctx.strokeStyle = 'rgba(0,0,0,0.5)';
  ctx.stroke(hole);
  // The label, with the window cut out of it.
  const label = shape(38, 33, 232, 74, 4);
  shape(92, 68, 124, 32, 5, label);
  const paper = ctx.createLinearGradient(0, 33, 0, 107);
  paper.addColorStop(0, CREAM);
  paper.addColorStop(1, '#ddd2b6');
  ctx.fillStyle = paper;
  ctx.fill(label, 'evenodd');
  ctx.save();
  ctx.clip(label, 'evenodd');
  for (const [y, colour] of [[54, '#f2b632'], [57.5, '#ec7a2a'], [61, '#d2412e']]) {
    ctx.fillStyle = colour;
    ctx.fillRect(38, y, 232, 3);
  }
  ctx.restore();
  text('Tape', 48, 49, 'italic 700 17px Georgia, "Times New Roman", serif', '#2b2420', 'left');
  const style = rows[STYLE];
  if (numeric(style) && style.choices) {
    text(style.choices[style.value] ?? '', 260, 48, `700 9px ${FONT}`, '#2b2420', 'right', '1.5px');
  }
  // Where the heads meet the tape.
  ctx.beginPath();
  ctx.moveTo(82, 135);
  ctx.lineTo(96, 112);
  ctx.lineTo(212, 112);
  ctx.lineTo(226, 135);
  ctx.closePath();
  ctx.fillStyle = '#2d2d31';
  ctx.fill();
  disc(112, 124, 3, '#0c0c0d');
  disc(196, 124, 3, '#0c0c0d');
  ctx.fillStyle = '#4a3324';
  ctx.fillRect(100, 132, 108, 3);
  screw(40, 125, 0.2);
  screw(268, 125, 1.1);
}

function door() {
  const { x, y, w, h } = DOOR;
  ctx.fillStyle = '#0d0c0b';
  ctx.fill(shape(x - 2, y - 2, w + 4, h + 4, 10));
  const glass = shape(x, y, w, h, 8);
  const g = ctx.createLinearGradient(0, y, 0, y + h);
  g.addColorStop(0, '#17181b');
  g.addColorStop(1, '#0b0b0d');
  ctx.fillStyle = g;
  ctx.fill(glass);
  ctx.save();
  ctx.clip(glass);
  if (rows) {
    cassette();
  } else {
    // An empty door: the two spindles and nothing on them.
    for (const sx of [122, 186]) {
      disc(sx, 84, 7, '#2a2a2d');
      disc(sx, 84, 3, '#151517');
    }
    text('NO TAPE', 154, 120, `600 10px ${FONT}`, MUTED, 'center', '2px');
  }
  ctx.fillStyle = 'rgba(24, 18, 12, 0.25)';
  ctx.fillRect(x, y, w, h);
  const shine = ctx.createLinearGradient(x, y, x + w * 0.55, y + h);
  shine.addColorStop(0, 'rgba(255,255,255,0.11)');
  shine.addColorStop(0.38, 'rgba(255,255,255,0.03)');
  shine.addColorStop(0.39, 'rgba(255,255,255,0)');
  ctx.fillStyle = shine;
  ctx.fillRect(x, y, w, h);
  ctx.restore();
  ctx.strokeStyle = 'rgba(255,255,255,0.07)';
  ctx.stroke(shape(x + 0.5, y + 0.5, w - 1, h - 1, 8));
}

function panel() {
  const { x, y, w, h } = PANEL;
  ctx.fillStyle = 'rgba(0,0,0,0.2)';
  ctx.fill(shape(x, y, w, h, 8));
  const style = rows && rows[STYLE];
  const live = numeric(style) && style.choices;
  const pick = live ? style.value : 0;
  const choices = live ? style.choices : ['PAL', 'NTSC'];
  // The switch: up for the first style, down for the second.
  text(choices[0] ?? '', x + 14, y + 26, `700 10px ${FONT}`, live && pick === 0 ? CREAM : MUTED, 'left', '1.5px');
  text(choices[1] ?? '', x + 14, y + 74, `700 10px ${FONT}`, live && pick === 1 ? CREAM : MUTED, 'left', '1.5px');
  const px = x + 70;
  const py = y + 47;
  const nut = ctx.createRadialGradient(px - 3, py - 3, 1, px, py, 11);
  nut.addColorStop(0, '#d4d0c8');
  nut.addColorStop(1, '#4c4a45');
  disc(px, py, 11, '#151412');
  disc(px, py, 8.5, nut);
  const tip = pick === 0 ? py - 27 : py + 27;
  ctx.lineCap = 'round';
  ctx.strokeStyle = live ? '#c9c5bc' : '#6a665f';
  ctx.lineWidth = 5;
  ctx.beginPath();
  ctx.moveTo(px, py);
  ctx.lineTo(px, tip);
  ctx.stroke();
  const ball = ctx.createRadialGradient(px - 1.5, tip - 1.5, 0.5, px, tip, 5.5);
  ball.addColorStop(0, '#f4f1ea');
  ball.addColorStop(1, live ? '#8f8b83' : '#4f4c46');
  disc(px, tip, 5.5, ball);
  // The record light, on while a knob is being turned.
  ctx.save();
  if (held) {
    ctx.shadowColor = '#ff3b2f';
    ctx.shadowBlur = 10;
  }
  disc(x + w - 13, y + 13, 3.5, held ? '#ff4636' : '#4a1813');
  ctx.restore();
  // The counter, showing the knob last turned.
  ctx.fillStyle = '#080807';
  ctx.fill(shape(x + 10, y + h - 30, w - 20, 22, 3));
  text(rows && shown ? format(rows[shown]) : '---', x + w - 16, y + h - 13, `600 14px ${MONO}`, rows ? CREAM : MUTED, 'right', '2px');
}

function knob(i, id) {
  const x = knobX(i);
  const y = KNOB_Y;
  const row = rows && rows[id];
  const live = numeric(row);
  const t = travel(row);
  const turn = -SWEEP + 2 * SWEEP * t;
  ctx.lineCap = 'round';
  for (let k = 0; k <= 10; k++) {
    const a = -SWEEP + (2 * SWEEP * k) / 10;
    const big = k % 5 === 0;
    ctx.strokeStyle = live && k / 10 <= t + 1e-6 ? INK : 'rgba(233,223,201,0.22)';
    ctx.lineWidth = big ? 2 : 1.2;
    ctx.beginPath();
    ctx.moveTo(x + Math.sin(a) * (KNOB_R + 5), y - Math.cos(a) * (KNOB_R + 5));
    ctx.lineTo(x + Math.sin(a) * (KNOB_R + (big ? 10 : 8)), y - Math.cos(a) * (KNOB_R + (big ? 10 : 8)));
    ctx.stroke();
  }
  ctx.save();
  ctx.shadowColor = 'rgba(0,0,0,0.65)';
  ctx.shadowBlur = 9;
  ctx.shadowOffsetY = 3;
  disc(x, y, KNOB_R, '#141311');
  ctx.restore();
  // The knurled skirt turns with the knob.
  ctx.lineWidth = 1;
  for (let k = 0; k < 44; k++) {
    const a = turn + (k / 44) * Math.PI * 2;
    ctx.strokeStyle = k % 2 ? 'rgba(255,255,255,0.10)' : 'rgba(0,0,0,0.5)';
    ctx.beginPath();
    ctx.moveTo(x + Math.sin(a) * (KNOB_R - 5), y - Math.cos(a) * (KNOB_R - 5));
    ctx.lineTo(x + Math.sin(a) * (KNOB_R - 0.5), y - Math.cos(a) * (KNOB_R - 0.5));
    ctx.stroke();
  }
  const cap = ctx.createRadialGradient(x - 6, y - 8, 2, x, y, KNOB_R - 5);
  cap.addColorStop(0, live ? '#dedbd3' : '#8a877f');
  cap.addColorStop(0.55, live ? '#a29e94' : '#5f5c56');
  cap.addColorStop(1, '#55524c');
  disc(x, y, KNOB_R - 5, cap);
  ctx.strokeStyle = 'rgba(0,0,0,0.08)';
  for (let ring = 4; ring < KNOB_R - 5; ring += 2.5) {
    ctx.beginPath();
    ctx.arc(x, y, ring, 0, Math.PI * 2);
    ctx.stroke();
  }
  ctx.strokeStyle = live ? INK : '#47443f';
  ctx.lineWidth = 3;
  ctx.beginPath();
  ctx.moveTo(x + Math.sin(turn) * 6, y - Math.cos(turn) * 6);
  ctx.lineTo(x + Math.sin(turn) * (KNOB_R - 9), y - Math.cos(turn) * (KNOB_R - 9));
  ctx.stroke();
  const name = (row ? row.label : NAMES[id]).toUpperCase();
  const lit = held && held.id === id;
  text(name, x, y + KNOB_R + 21, `600 9px ${FONT}`, lit ? CREAM : live ? '#a59e91' : MUTED, 'center', '1.5px');
}

function draw() {
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  const s = view.scale * view.dpr;
  ctx.setTransform(s, 0, 0, s, view.x * view.dpr, view.y * view.dpr);
  body();
  door();
  panel();
  KNOBS.forEach((id, i) => knob(i, id));
}

function fit() {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  canvas.width = Math.round(w * dpr);
  canvas.height = Math.round(h * dpr);
  const scale = Math.min(w / W, h / H);
  view = { scale, x: (w - W * scale) / 2, y: (h - H * scale) / 2, dpr };
  draw();
}

window.addEventListener('resize', fit);
fit();
if (window.lumit) {
  listen();
} else {
  window.addEventListener('lumitready', listen, { once: true });
}
