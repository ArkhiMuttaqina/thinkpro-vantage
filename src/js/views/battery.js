// Battery View
const { invoke } = window.__TAURI__.core;
import { elements } from '../dom.js';
import { showStatus, escapeHtml } from '../utils.js';
import { setState, getState } from '../state.js';

/* -------------------------------------------------------------------------
 * The pack, drawn the way TLP Panel draws it.
 *
 * A canvas, not a styled div: the fill has to be clipped to the inside of the
 * shell, its right edge has to stay a square waterline at every level, and the
 * sweep and the motes have to be painted *inside* that clip. A CSS progress
 * bar can do none of those three.
 * ---------------------------------------------------------------------- */

const CANVAS_WIDTH = 240;
const CANVAS_HEIGHT = 58;

const LEVEL_GOOD = 50; // at or above this the fill is blue
const LEVEL_CRITICAL = 10; // the ramp bottoms out here and stays red
const LEVEL_BLINK = 5; // below this the fill pulses
const BLINK_MS = 900; // half a pulse
const BLINK_DIM = 0.35;
const WAVE_PERIOD_SECONDS = 5;
const WAVE_MS = 1200; // one sweep
const WAVE_BAND = 0.22; // half the band width, as a fraction of the fill
const WAVE_ALPHA = 0.22;
const PARTICLE_COUNT = 14;
const PARTICLE_CYCLE_MS = 4000;
const PARTICLE_ALPHA = 0.85;
const PARTICLE_FADE = 1.1;

/** Below this the pack is not really moving charge; no rate, no estimate. */
const IDLE_WATTS = 0.05;

/** Fixed seed: same motes, same drift, every session and every screenshot. */
const PARTICLE_SEED = 20260816;

/**
 * Fill colours, read from tokens.css so the canvas cannot drift from the rest
 * of the UI. The literals are the reference panel's own values and only apply
 * if a token is missing, which is the case on a harness page with no stylesheet.
 */
const COLOUR_FALLBACKS = {
  ok: [34, 197, 94],
  info: [59, 130, 246],
  warn: [245, 158, 11],
  danger: [239, 68, 68],
  fg: [245, 245, 245]
};

/**
 * Everything the canvas paints from. One object, so an update replaces the
 * whole picture at once and the animation reads the same fields the data does.
 */
const pack = {
  present: false,
  percent: 0,
  charging: false,
  discharging: false,
  blinkLevel: 1,
  wavePos: null,
  particlePhase: null
};

let canvas = null;
let ctx = null;
let frameHandle = null;
let clockStartedAt = 0;
let clockElapsed = 0;
let colours = null;
let reducedMotion = null;

/* --- colour ------------------------------------------------------------- */

function parseColour(raw) {
  if (!raw) {
    return null;
  }
  const value = raw.trim();

  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(value);
  if (hex) {
    const digits =
      hex[1].length === 3
        ? hex[1]
            .split('')
            .map((c) => c + c)
            .join('')
        : hex[1];
    return [0, 2, 4].map((i) => parseInt(digits.slice(i, i + 2), 16));
  }

  const rgb = /^rgba?\(([^)]+)\)$/i.exec(value);
  if (rgb) {
    const parts = rgb[1]
      .split(/[\s,/]+/)
      .filter(Boolean)
      .map(Number);
    if (parts.length >= 3 && parts.slice(0, 3).every(Number.isFinite)) {
      return parts.slice(0, 3);
    }
  }
  return null;
}

function readColours() {
  if (colours) {
    return colours;
  }
  let style = null;
  try {
    style = getComputedStyle(document.documentElement);
  } catch (error) {
    style = null;
  }
  const read = (name) =>
    parseColour(style ? style.getPropertyValue(name) : null) ?? COLOUR_FALLBACKS[name.slice(2)];

  colours = {
    ok: read('--ok'),
    info: read('--info'),
    warn: read('--warn'),
    danger: read('--danger'),
    fg: read('--fg')
  };
  return colours;
}

/* --- geometry ----------------------------------------------------------- */

function rgba(rgb, alpha) {
  return `rgba(${rgb[0]}, ${rgb[1]}, ${rgb[2]}, ${alpha})`;
}

function roundedRect(context, x, y, width, height, radius) {
  const r = Math.min(radius, width / 2, height / 2);
  context.beginPath();
  context.moveTo(x + r, y);
  context.lineTo(x + width - r, y);
  context.arcTo(x + width, y, x + width, y + r, r);
  context.lineTo(x + width, y + height - r);
  context.arcTo(x + width, y + height, x + width - r, y + height, r);
  context.lineTo(x + r, y + height);
  context.arcTo(x, y + height, x, y + height - r, r);
  context.lineTo(x, y + r);
  context.arcTo(x, y, x + r, y, r);
  context.closePath();
}

function mix(from, to, t) {
  return from.map((channel, index) => Math.round(channel + (to[index] - channel) * t));
}

/**
 * Green while charging; otherwise blue, sliding to amber then red.
 *
 * Red arrives at LEVEL_CRITICAL rather than at empty, and stays. Running the
 * ramp all the way down would spend the loudest colour on a battery that is
 * already flat, and make the difference between 8% and 2% look like a shade
 * rather than an emergency.
 */
function levelColour(percent, charging) {
  const { ok, info, warn, danger } = readColours();

  // A pack that is filling is good news at any level. The percentage already
  // says how empty it is, and a red bar on a battery that is actively
  // recovering is an alarm nobody can act on.
  if (charging) {
    return ok;
  }

  if (percent >= LEVEL_GOOD) {
    return info;
  }
  // Blue steps straight to amber at the threshold: interpolating between them
  // in RGB would pass through a muddy green.
  const span = (percent - LEVEL_CRITICAL) / (LEVEL_GOOD - LEVEL_CRITICAL);
  return mix(danger, warn, Math.max(0, Math.min(1, span)));
}

function easeInOutSine(t) {
  return (1 - Math.cos(Math.PI * t)) / 2;
}

/* --- particles ---------------------------------------------------------- */

function mulberry32(seed) {
  let a = seed >>> 0;
  return function next() {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * Fixed motes, built once.
 *
 * Heights and start times come off an even ladder that is then shuffled rather
 * than straight out of the generator: a handful of independent draws leaves
 * gaps and clumps often enough that trusting one to look spread out is a coin
 * toss. Everything that only adds texture — size, sway, speed — is random from
 * a fixed seed, so the drift is stable between frames.
 */
function makeParticles(count) {
  const random = mulberry32(PARTICLE_SEED);
  const spread = Math.max(1, count - 1);
  const heights = Array.from({ length: count }, (_, index) => 0.26 + (0.48 * index) / spread);

  for (let i = heights.length - 1; i > 0; i -= 1) {
    const j = Math.floor(random() * (i + 1));
    [heights[i], heights[j]] = [heights[j], heights[i]];
  }

  return heights.map((rest, index) => ({
    // Evenly spaced arrivals, nudged so they do not march in lockstep.
    start: (((index / count + (random() * 2 - 1) * 0.03) % 1) + 1) % 1,
    rest,
    // Whole numbers of passes per cycle, or the loop would jump.
    laps: [1, 1, 1, 2][Math.floor(random() * 4)],
    size: 1.6 + random() * 1.6,
    sway: 0.04 + random() * 0.07,
    cycles: 0.8 + random() * 1.0
  }));
}

const PARTICLES = makeParticles(PARTICLE_COUNT);

function drawParticles(context, x, y, fillWidth, height) {
  const phase = pack.particlePhase;
  if (phase === null) {
    return;
  }

  // Clipped to the fill, so a mote that outlives its fade still stops at the
  // waterline rather than floating out over the empty part of the shell.
  context.save();
  context.beginPath();
  context.rect(x, y, fillWidth, height);
  context.clip();

  for (const mote of PARTICLES) {
    const travel = (((phase * mote.laps + mote.start) % 1) + 1) % 1;
    // A quick fade in at the edge so nothing pops into existence, then a long
    // fade out across the rest of the trip.
    const alpha = PARTICLE_ALPHA * Math.min(1, travel / 0.12) * Math.pow(1 - travel, PARTICLE_FADE);
    if (alpha < 0.01) {
      continue;
    }
    const sway = mote.sway * height * Math.sin(2 * Math.PI * (mote.cycles * travel + mote.start));
    context.fillStyle = rgba([255, 255, 255], alpha * pack.blinkLevel);
    context.beginPath();
    context.arc(x + travel * fillWidth, y + mote.rest * height + sway, mote.size, 0, 2 * Math.PI);
    context.fill();
  }

  context.restore();
}

/** A soft band of light at the current sweep position. */
function waveGradient(context, x0, x1) {
  const span = 1 + 2 * WAVE_BAND;
  // Charging runs it left to right; draining runs it the other way, which is
  // the whole point of the effect.
  const progress = pack.charging ? pack.wavePos : 1 - pack.wavePos;
  const centre = -WAVE_BAND + progress * span;
  const clamp = (value) => Math.max(0, Math.min(1, value));

  // The band travels from just outside one end to just outside the other, so
  // it enters and leaves rather than appearing and vanishing mid-fill.
  const stops = [
    [clamp(centre - WAVE_BAND), 0],
    [clamp(centre), WAVE_ALPHA],
    [clamp(centre + WAVE_BAND), 0]
  ].sort((a, b) => a[0] - b[0]);

  const gradient = context.createLinearGradient(x0, 0, x1, 0);
  gradient.addColorStop(0, rgba([255, 255, 255], 0));
  for (const [offset, alpha] of stops) {
    gradient.addColorStop(offset, rgba([255, 255, 255], alpha));
  }
  gradient.addColorStop(1, rgba([255, 255, 255], 0));
  return gradient;
}

/* --- the drawing -------------------------------------------------------- */

function drawPack() {
  if (!ctx) {
    return;
  }

  const fg = readColours().fg;
  ctx.clearRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);

  const nubWidth = Math.max(4, CANVAS_WIDTH * 0.022);
  const border = 2.5;
  const bodyWidth = CANVAS_WIDTH - nubWidth - 2;
  const bodyHeight = CANVAS_HEIGHT;
  const radius = bodyHeight * 0.28;

  // Shell
  ctx.strokeStyle = rgba(fg, 0.55);
  ctx.lineWidth = border;
  roundedRect(ctx, border / 2, border / 2, bodyWidth - border, bodyHeight - border, radius);
  ctx.stroke();

  // Terminal
  const nubHeight = bodyHeight * 0.38;
  ctx.fillStyle = rgba(fg, 0.55);
  roundedRect(
    ctx,
    bodyWidth + 1,
    (bodyHeight - nubHeight) / 2,
    nubWidth,
    nubHeight,
    nubWidth * 0.45
  );
  ctx.fill();

  // Charge — a plain rectangle clipped to the inside of the shell. The clip
  // does three jobs a shaped path cannot: the fill meets the outline with no
  // gap, its right edge stays a square waterline at any level, and it rounds
  // off by itself once it reaches the far end.
  const inset = border; // the inner edge of the stroke, which straddles the path
  const innerWidth = bodyWidth - 2 * inset;
  const innerHeight = bodyHeight - 2 * inset;
  const fillWidth = (innerWidth * Math.max(0, Math.min(100, pack.percent))) / 100;

  if (fillWidth > 0.5) {
    const [red, green, blue] = levelColour(pack.percent, pack.charging);
    // Dimming rather than blanking: the geometry stays put, and the percentage
    // written across the fill stays readable throughout.
    const level = pack.blinkLevel;

    ctx.save();
    roundedRect(ctx, inset, inset, innerWidth, innerHeight, radius - border / 2);
    ctx.clip();

    const dim = (channel) => Math.round(channel * level);
    ctx.fillStyle = `rgb(${dim(red)}, ${dim(green)}, ${dim(blue)})`;
    ctx.fillRect(inset, inset, fillWidth, innerHeight);

    drawParticles(ctx, inset, inset, fillWidth, innerHeight);

    if (pack.wavePos !== null) {
      ctx.fillStyle = waveGradient(ctx, inset, inset + fillWidth);
      ctx.fillRect(inset, inset, fillWidth, innerHeight);
    }
    ctx.restore();
  }

  // Percentage, written inside the shell.
  const text = `${Math.round(pack.percent)}%`;
  const fontSize = bodyHeight * 0.42;
  ctx.font = `700 ${fontSize}px Cantarell, system-ui, sans-serif`;
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';

  const textWidth = ctx.measureText(text).width;
  const boltWidth = pack.charging ? bodyHeight * 0.34 : 0;
  const gap = pack.charging ? bodyHeight * 0.12 : 0;
  const startX = bodyWidth / 2 - (textWidth + boltWidth + gap) / 2;
  const midY = bodyHeight / 2;

  const paintContents = () => {
    if (pack.charging) {
      const unit = bodyHeight * 0.24;
      const bx = startX + boltWidth / 2;
      ctx.beginPath();
      ctx.moveTo(bx + unit * 0.25, midY - unit);
      ctx.lineTo(bx - unit * 0.55, midY + unit * 0.12);
      ctx.lineTo(bx - unit * 0.02, midY + unit * 0.12);
      ctx.lineTo(bx - unit * 0.25, midY + unit);
      ctx.lineTo(bx + unit * 0.55, midY - unit * 0.12);
      ctx.lineTo(bx + unit * 0.02, midY - unit * 0.12);
      ctx.closePath();
      ctx.fill();
    }
    ctx.fillText(text, startX + boltWidth + gap, midY);
  };

  // Drawn twice, clipped either side of the fill edge: white over the coloured
  // part, theme colour over the empty part. A single colour would vanish
  // wherever the text crosses that edge.
  const fillEdge = inset + fillWidth;

  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, fillEdge, bodyHeight);
  ctx.clip();
  ctx.fillStyle = 'rgb(255, 255, 255)';
  paintContents();
  ctx.restore();

  ctx.save();
  ctx.beginPath();
  ctx.rect(fillEdge, 0, bodyWidth - fillEdge, bodyHeight);
  ctx.clip();
  ctx.fillStyle = rgba(fg, 0.85);
  paintContents();
  ctx.restore();
}

/* --- the animation ------------------------------------------------------ */

function prefersReducedMotion() {
  if (reducedMotion === null) {
    reducedMotion =
      typeof window.matchMedia === 'function' &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  }
  return reducedMotion;
}

/** Whether anything on the canvas is still moving. */
function needsAnimation() {
  if (!pack.present) {
    return false;
  }
  // A pack climbing back from empty is recovering; flashing at someone who has
  // already plugged the cable in tells them nothing they can act on.
  if (pack.percent <= LEVEL_BLINK && !pack.charging) {
    return true;
  }
  return pack.charging || pack.discharging;
}

/**
 * Advance every phase from a single elapsed time.
 *
 * Each phase is a pure function of the clock rather than an accumulator, so a
 * frame that arrives late — or not at all — leaves the picture correct instead
 * of behind.
 */
function updatePhases(elapsed) {
  // Blink: a triangle wave over two half-pulses, so the fill breathes instead
  // of flicking between two states.
  if (pack.percent <= LEVEL_BLINK && !pack.charging) {
    const half = (elapsed / BLINK_MS) % 2;
    pack.blinkLevel = 1 + (BLINK_DIM - 1) * easeInOutSine(half <= 1 ? half : 2 - half);
  } else {
    pack.blinkLevel = 1;
  }

  // Wave: one sweep per period, with the rest of the period left still. A pack
  // sitting at its charge limit moves no charge, and has no direction to show.
  if (pack.charging || pack.discharging) {
    const sinceSweep = elapsed % (WAVE_PERIOD_SECONDS * 1000);
    pack.wavePos = sinceSweep < WAVE_MS ? easeInOutSine(sinceSweep / WAVE_MS) : null;
  } else {
    pack.wavePos = null;
  }

  // Particles only while charging — they are the energy coming in.
  pack.particlePhase = pack.charging ? (elapsed % PARTICLE_CYCLE_MS) / PARTICLE_CYCLE_MS : null;
}

/** The still picture: nothing sweeping, nothing drifting, nothing pulsing. */
function restPhases() {
  pack.blinkLevel = 1;
  pack.wavePos = null;
  pack.particlePhase = null;
}

function animate(timestamp) {
  frameHandle = null;

  if (!needsAnimation()) {
    // The pack settled between frames, or the view is going away: draw the
    // resting picture once and let the frame clock stop.
    clockElapsed = 0;
    restPhases();
    drawPack();
    return;
  }

  clockElapsed = timestamp - clockStartedAt;
  updatePhases(clockElapsed);
  drawPack();
  frameHandle = requestAnimationFrame(animate);
}

function startPackAnimation() {
  if (frameHandle !== null || prefersReducedMotion() || !needsAnimation()) {
    return;
  }
  // Resume from where the last run stopped, so the sweep does not teleport.
  clockStartedAt = performance.now() - clockElapsed;
  frameHandle = requestAnimationFrame(animate);
}

function stopPackAnimation() {
  if (frameHandle !== null) {
    cancelAnimationFrame(frameHandle);
    frameHandle = null;
  }
}

/**
 * Point the canvas at the live pack state.
 *
 * The loop runs only while there is something to show — a pack sitting full on
 * AC draws one frame and stops, rather than repainting a hidden page forever.
 */
function setPack({ present, percent, charging, discharging }) {
  pack.present = present;
  pack.percent = present ? Math.max(0, Math.min(100, percent)) : 0;
  pack.charging = present && charging;
  pack.discharging = present && discharging;

  // A dimmed fill must not survive into a pack that is filling or comfortably
  // above the critical level. updatePhases() corrects it on the next frame, but
  // the frame drawn at the end of this function would still show it.
  if (pack.charging || pack.percent > LEVEL_BLINK) {
    pack.blinkLevel = 1;
  }

  if (needsAnimation() && !prefersReducedMotion()) {
    startPackAnimation();
  } else {
    stopPackAnimation();
    clockElapsed = 0;
    restPhases();
  }

  if (canvas) {
    canvas.setAttribute('aria-label', `Battery at ${Math.round(pack.percent)} percent`);
  }
  drawPack();
}

function setupCanvas() {
  canvas = document.getElementById('battery-hero-canvas');
  if (!canvas) {
    return;
  }
  ctx = canvas.getContext('2d');
  // Back the canvas at device resolution so the 2.5px outline and the
  // percentage stay crisp; everything after this draws in CSS pixels.
  const ratio = window.devicePixelRatio || 1;
  canvas.width = Math.round(CANVAS_WIDTH * ratio);
  canvas.height = Math.round(CANVAS_HEIGHT * ratio);
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  restPhases();
  drawPack();
}

/* --- formatting --------------------------------------------------------- */

/**
 * Human readable duration, e.g. "5 h 30 min". null when there is no estimate
 * to give, so the caller can say so rather than print "0 min".
 */
function formatDuration(hours) {
  if (typeof hours !== 'number' || !Number.isFinite(hours) || hours <= 0) {
    return null;
  }
  const totalMinutes = Math.round(hours * 60);
  const h = Math.floor(totalMinutes / 60);
  const m = totalMinutes % 60;
  if (h && m) {
    return `${h} h ${m} min`;
  }
  if (h) {
    return `${h} h`;
  }
  return `${m} min`;
}

/**
 * Hours to move a given energy at a given rate, or null when there is no
 * estimate. Same rule as battery.rs::hours_at_rate: a rate at or below
 * IDLE_WATTS is a pack sitting still, and a non-positive delta is not an
 * instant trip.
 */
function hoursAtRate(deltaWh, watts) {
  if (!(watts > IDLE_WATTS) || !(deltaWh > 0)) {
    return null;
  }
  return deltaWh / watts;
}

function formatWh(value) {
  return `${value.toFixed(2)} Wh`;
}

const DASH = '—';

/* --- rendering ---------------------------------------------------------- */

function setText(id, text, title) {
  const el = document.getElementById(id);
  if (!el) {
    return;
  }
  el.textContent = text;
  if (title !== undefined) {
    if (title) {
      el.title = title;
    } else {
      el.removeAttribute('title');
    }
  }
}

function renderRows(container, rows) {
  if (!container) {
    return;
  }
  container.innerHTML = rows
    .map(
      (row) => `<div class="battery-row">
        <span class="battery-row-name">${escapeHtml(row.name)}</span>
        <span class="battery-row-value"${
          row.title ? ` title="${escapeHtml(row.title)}"` : ''
        }>${escapeHtml(row.value)}</span>
      </div>`
    )
    .join('');
}

/**
 * The hero, which IS the primary pack: its name, its status, its charge drawn
 * as a battery, and the pack's own instantaneous power. Nothing here is
 * invented — a pack sitting full on AC is 0.00 W, and says so.
 */
function renderHero(primary, power) {
  const status = (primary && primary.status) || (power && power.status) || '';
  const percent = primary ? primary.capacity : (power && power.capacity) || 0;
  const watts = primary ? primary.power : (power && power.power_w) || 0;
  const charging = status === 'Charging';
  const discharging = status === 'Discharging';

  setPack({
    present: Boolean(primary || power),
    percent,
    charging,
    discharging
  });

  setText('battery-hero-name', primary ? primary.name : 'Battery');

  const badge = document.getElementById('battery-hero-badge');
  if (badge) {
    badge.textContent = status || DASH;
    badge.dataset.status = status ? status.toLowerCase().replace(/\s+/g, '-') : 'unknown';
  }

  setText(
    'battery-hero-value',
    `${watts.toFixed(2)} W`,
    !(watts > 0) && power && power.is_ac_online
      ? 'The pack is idle and the laptop is running from AC'
      : ''
  );

  setText(
    'battery-hero-label',
    charging ? 'Charging power' : discharging ? 'Battery draw' : 'Power'
  );

  setText(
    'battery-hero-source',
    power ? (power.is_ac_online ? 'On AC power' : 'On battery') : DASH
  );
}

/** The pack block: what it is, how worn, and where the firmware caps it. */
function renderBatteryBlock(primary, power) {
  const rows = [];

  // The hero IS the primary pack, so its wear is its own: pairing one pack's
  // percentage with the machine's summed energy would describe neither. With a
  // single pack — the common case — these are the same numbers.
  const full = primary ? primary.energy_full : power ? power.energy_full_wh : 0;
  const design = primary ? primary.energy_design : power ? power.energy_design_wh : 0;
  const now = primary ? primary.energy_now : power ? power.energy_now_wh : 0;

  if (full > 0 && design > 0) {
    // The pack's own figure when the backend has one: 56.98 of 57.00 Wh is 99%
    // by truncation, and rounding it up to 100% would contradict the two
    // energies printed beside it.
    const health =
      primary && typeof primary.health === 'number'
        ? primary.health
        : Math.round((full / design) * 100);
    rows.push({
      name: 'Health',
      value: `${health}%  (${full.toFixed(2)} / ${design.toFixed(2)} Wh)`
    });
  } else {
    rows.push({
      name: 'Health',
      value: DASH,
      title: 'This pack does not report both its full and its design energy'
    });
  }

  const cycles = primary ? primary.cycles : 0;
  if (cycles > 0) {
    rows.push({ name: 'Cycles', value: String(cycles) });
  } else {
    rows.push({
      name: 'Cycles',
      value: 'N/A',
      title: 'Cycle count is not reported by battery hardware'
    });
  }

  const start = power ? power.start_threshold : null;
  const stop = power ? power.stop_threshold : null;
  if (stop !== null && stop !== undefined) {
    rows.push({
      name: 'Charge limit',
      value: start !== null && start !== undefined ? `${start}% – ${stop}%` : `${stop}%`,
      title: 'Firmware stops charging at the upper limit and resumes at the lower one'
    });
  } else {
    rows.push({
      name: 'Charge limit',
      value: 'Not adjustable on this machine',
      title: 'The firmware exposes no charge threshold control'
    });
  }

  rows.push({ name: 'Energy now', value: formatWh(now) });
  rows.push({ name: 'Voltage', value: primary ? `${primary.voltage.toFixed(2)} V` : DASH });
  rows.push({ name: 'Technology', value: (primary && primary.technology) || DASH });
  rows.push({ name: 'Manufacturer', value: (primary && primary.manufacturer) || DASH });

  renderRows(document.getElementById('battery-block'), rows);
}

/**
 * The reference's three projections, for whichever way the energy is moving.
 *
 * Keyed on the cable rather than on whether current is flowing: a pack sitting
 * at its charge limit reports "not charging", and the card should still be
 * answering the charging question.
 */
function renderRuntimeBlock(power) {
  const now = power.energy_now_wh;
  const full = power.energy_full_wh;
  const watts = power.power_w;
  const limit = power.stop_threshold;
  // A limit at 100% is no limit: the capped projection would just repeat the
  // full-charge one.
  const capped = limit !== null && limit !== undefined && limit < 100 && power.limit_energy_wh > 0;
  const limitEnergy = power.limit_energy_wh;

  const withPercent = (text, percent) => `${text} (${percent}%)`;
  const rows = [];

  if (power.is_ac_online) {
    // With the cable in, power_now is the charge rate, so the machine's own
    // consumption is unknown and "right now" has nothing honest to say.
    setText('battery-runtime-title', 'Estimated charging time');
    rows.push({
      name: 'Right now',
      value: DASH,
      title: 'While charging, the pack reports charge power, not what the system is drawing'
    });
    if (capped) {
      rows.push({
        name: 'To the charge limit',
        value: withPercent(formatDuration(hoursAtRate(limitEnergy - now, watts)) ?? DASH, limit)
      });
    }
    rows.push({
      name: 'To 100%',
      value: formatDuration(power.time_to_full_hours) ?? DASH
    });
  } else {
    setText('battery-runtime-title', 'Estimated runtime');
    rows.push({ name: 'Right now', value: formatDuration(power.runtime_hours) ?? DASH });
    if (capped) {
      rows.push({
        name: 'At the charge limit',
        value: withPercent(formatDuration(hoursAtRate(limitEnergy, watts)) ?? DASH, limit)
      });
    }
    rows.push({
      name: 'At 100%',
      value: formatDuration(hoursAtRate(full, watts)) ?? DASH
    });
  }

  renderRows(document.getElementById('battery-runtime-block'), rows);
}

/**
 * Extra packs, as compact rows. A second full card would repeat the hero's
 * shape for information the hero already carries for the machine.
 */
function renderExtraPacks(packs) {
  const container = document.getElementById('battery-extra-packs');
  if (!container) {
    return;
  }
  if (!packs.length) {
    container.hidden = true;
    container.innerHTML = '';
    return;
  }

  container.hidden = false;
  container.innerHTML = packs
    .map(
      (battery) => `<div class="battery-extra-pack">
        <span class="battery-extra-name">${escapeHtml(battery.name)}</span>
        <span class="battery-extra-status">${escapeHtml(battery.status)}</span>
        <span class="battery-extra-percent">${battery.capacity}%</span>
        <span class="battery-extra-power">${battery.power.toFixed(2)} W</span>
      </div>`
    )
    .join('');
}

/* --- view lifecycle ----------------------------------------------------- */

export function setupBatteryHandlers() {
  if (elements.thresholdStart) {
    elements.thresholdStart.addEventListener('input', (e) => {
      elements.thresholdStartValue.textContent = e.target.value + '%';
    });
  }
  if (elements.thresholdStop) {
    elements.thresholdStop.addEventListener('input', (e) => {
      elements.thresholdStopValue.textContent = e.target.value + '%';
    });
  }
  if (elements.btnApplyThresholds) {
    elements.btnApplyThresholds.addEventListener('click', applyBatteryThresholds);
  }

  setupCanvas();
}

export function startBatteryUpdate() {
  stopBatteryUpdate();
  loadBatteryInfo();
  const interval = setInterval(async () => {
    if (getState('currentView') === 'battery') {
      await loadBatteryInfo();
    }
  }, 2500);
  setState('batteryInterval', interval);
}

export function stopBatteryUpdate() {
  const interval = getState('batteryInterval');
  if (interval) {
    clearInterval(interval);
    setState('batteryInterval', null);
  }
  // The view registry calls this on hide. Without it the frame loop would keep
  // repainting a page nobody is looking at.
  stopPackAnimation();
}

export async function loadBatteryInfo() {
  try {
    // Fetch the live power reading for the hero
    let power = null;
    try {
      const powerResponse = await invoke('get_power_consumption');
      if (powerResponse.success && powerResponse.data) {
        power = powerResponse.data;
      }
    } catch (err) {
      console.error('[Battery] Power consumption query failed:', err);
    }

    // Fetch battery hardware details
    const response = await invoke('get_battery_info');
    const batteries = response.success && response.data ? response.data : [];
    const primary = batteries.length ? batteries[0] : null;

    renderHero(primary, power);
    if (power) {
      renderBatteryBlock(primary, power);
      renderRuntimeBlock(power);
    }
    renderExtraPacks(batteries.slice(1));

    const thresholdResponse = await invoke('get_battery_thresholds');
    if (thresholdResponse.success && thresholdResponse.data) {
      if (elements.thresholdStart && document.activeElement !== elements.thresholdStart) {
        elements.thresholdStart.value = thresholdResponse.data.start;
        elements.thresholdStartValue.textContent = thresholdResponse.data.start + '%';
      }
      if (elements.thresholdStop && document.activeElement !== elements.thresholdStop) {
        elements.thresholdStop.value = thresholdResponse.data.stop;
        elements.thresholdStopValue.textContent = thresholdResponse.data.stop + '%';
      }
    }
  } catch (error) {
    console.error('[Battery] Load failed:', error);
  }
}

async function applyBatteryThresholds() {
  const start = parseInt(elements.thresholdStart.value, 10);
  const stop = parseInt(elements.thresholdStop.value, 10);

  if (start >= stop) {
    showStatus('Start threshold must be lower than stop threshold', 'error');
    return;
  }

  showStatus('Setting battery thresholds...', 'info');

  try {
    const response = await invoke('set_battery_thresholds', { start, stop });
    if (response.success) {
      showStatus(`✓ Thresholds set: ${start}%-${stop}%`, 'success');
    } else {
      showStatus(`Error: ${response.error}`, 'error');
    }
  } catch (error) {
    showStatus(`Error: ${error}`, 'error');
  }
}
