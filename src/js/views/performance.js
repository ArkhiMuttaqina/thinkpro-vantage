// Performance View
const { invoke } = window.__TAURI__.core;
import { setPowerProfile, setCpuGovernor, setTurboBoost, bindOnce } from '../hardwareControls.js';
import { showStatus } from '../utils.js';

export async function loadPerformanceInfo() {
  try {
    const cpuResponse = await invoke('get_cpu_info');
    if (cpuResponse.success && cpuResponse.data) {
      displayCpuInfo(cpuResponse.data);
    }

    const profileResponse = await invoke('get_power_profile');
    if (profileResponse.success && profileResponse.data) {
      displayPowerProfiles(profileResponse.data);
    }

    const turboResponse = await invoke('get_turbo_boost_status');
    if (turboResponse.success) {
      displayTurboStatus(turboResponse.data);
    }

    await loadClockLimits();
  } catch (error) {
    console.error('[Performance] Load failed:', error);
  }
}

function displayCpuInfo(info) {
  document.getElementById('cpu-governor').textContent = info.governor;
  document.getElementById('cpu-freq').textContent = `${info.current_freq} MHz`;
  document.getElementById('cpu-freq-range').textContent = `${info.min_freq} - ${info.max_freq} MHz`;

  const container = document.getElementById('governor-buttons');
  container.innerHTML = '';

  info.available_governors.forEach((gov) => {
    const btn = document.createElement('button');
    btn.className = `option-btn ${gov === info.governor ? 'active' : ''}`;
    btn.textContent = gov.charAt(0).toUpperCase() + gov.slice(1);
    btn.onclick = () => setCpuGovernor(gov, loadPerformanceInfo, Array.from(container.children));
    container.appendChild(btn);
  });
}

function displayPowerProfiles(profileData) {
  const container = document.getElementById('profile-buttons');
  container.innerHTML = '';

  profileData.available.forEach((profile) => {
    const btn = document.createElement('button');
    btn.className = `option-btn ${profile === profileData.current ? 'active' : ''}`;
    btn.textContent = profile
      .split('-')
      .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
      .join(' ');
    btn.onclick = () => setPowerProfile(profile, loadPerformanceInfo);
    container.appendChild(btn);
  });
}

function displayTurboStatus(enabled) {
  const statusText = document.getElementById('turbo-status-text');
  const toggle = document.getElementById('turbo-toggle');

  if (statusText) {
    statusText.textContent = enabled ? 'Enabled' : 'Disabled';
    statusText.style.color = enabled ? 'var(--red-primary)' : 'var(--text-secondary)';
  }

  if (toggle) {
    toggle.checked = enabled;
    // Rebound on every render, so bindOnce rather than add/remove of a named
    // handler -- the Home copy of this leaked a listener per render.
    bindOnce(toggle, 'change', (e) =>
      setTurboBoost(e.target.checked, loadPerformanceInfo, e.target)
    );
  }
}

// ---------------------------------------------------------------------------
// Clock limits — TLP's ceilings, one card per power source
// ---------------------------------------------------------------------------
//
// TLP owns the clock ceiling. CPU_SCALING_{MAX,MIN}_FREQ_ON_{AC,BAT,SAV} are
// TLP's settings: it writes them to scaling_max_freq/scaling_min_freq on every
// boot and again on every switch between mains and battery, so a ceiling kept
// only inside this app decays the first time the cable moves or the machine
// restarts. This card therefore reads those keys out of TLP's merged
// configuration and writes them through the same writer the Power view uses.
//
// The axis is the power source, not the power profile: TLP holds one ceiling per
// source and has no way to express a per-profile one. Power saver's keys fall
// back to the battery keys while they are unset, and each card says so rather
// than showing Power saver as uncapped when TLP is applying a limit there.
//
// A card writes to TLP, never to the kernel. The writer rebuilds the app's whole
// drop-in file from the changes it is handed, so every other key that file
// already holds is sent back unchanged -- a key left out would be deleted.

const DROPIN_PATH = '/etc/tlp.d/99-thinkpro-vantage.conf';

/** '0' is TLP's own spelling for "no limit"; so is leaving the key out. */
const NO_LIMIT = '0';

const CEILING_SOURCES = [
  {
    id: 'AC',
    label: 'On AC',
    // The field this source's GPU ceiling lives in on `gpu_held_levels`.
    gpuKey: 'ac',
    maxKey: 'CPU_SCALING_MAX_FREQ_ON_AC',
    minKey: 'CPU_SCALING_MIN_FREQ_ON_AC'
  },
  {
    id: 'BAT',
    label: 'On battery',
    gpuKey: 'bat',
    maxKey: 'CPU_SCALING_MAX_FREQ_ON_BAT',
    minKey: 'CPU_SCALING_MIN_FREQ_ON_BAT'
  },
  {
    id: 'SAV',
    label: 'Power saver',
    gpuKey: 'sav',
    maxKey: 'CPU_SCALING_MAX_FREQ_ON_SAV',
    minKey: 'CPU_SCALING_MIN_FREQ_ON_SAV',
    // TLP reads an unset _ON_SAV key as the battery one.
    fallsBackTo: 'BAT'
  }
];

/** The two knobs each card carries: the ceiling at the top, the floor below. */
const LIMIT_SIDES = [
  { side: 'max', selector: '.cap-cpu-max' },
  { side: 'min', selector: '.cap-cpu-min' }
];

let clockLimits = null;
let tlpConfig = null;

/** One preview per card at a time, so a slow answer cannot overwrite a newer one. */
const previewTokens = new Map();

/** The subscription that re-reads the ceilings when the active profile moves. */
let profileWatch = null;

/** The subscription that re-reads them when the app settles the GPU. */
let gpuWatch = null;

function unwrap(response) {
  if (!response || response.success === false) {
    throw new Error((response && response.error) || 'The command failed');
  }
  return response.data;
}

function sourceById(id) {
  return CEILING_SOURCES.find((source) => source.id === id) || null;
}

/**
 * The power source a CPU card, or anything inside it, stands for.
 *
 * The GPU columns carry the same `data-source` ids, so every CPU lookup is
 * scoped to the CPU cards. Without that, a GPU column would resolve to the CPU
 * card of the same name and the two controls would answer for each other.
 */
function sourceOf(node) {
  const card = node.closest('.clock-cards .clock-card');
  return card ? sourceById(card.dataset.source) : null;
}

function cardOf(source) {
  return document.querySelector('.clock-cards .clock-card[data-source="' + source.id + '"]');
}

function sliderOf(card, side) {
  return card.querySelector(side === 'max' ? '.cap-cpu-max' : '.cap-cpu-min');
}

function keyFor(source, side) {
  return side === 'max' ? source.maxKey : source.minKey;
}

/** What TLP's merged configuration holds for one key. */
function tlpValue(key) {
  const values = (tlpConfig && tlpConfig.values) || {};
  const value = values[key];
  return value === undefined || value === null ? '' : String(value).trim();
}

/** Where TLP read that value from. */
function tlpSource(key) {
  const provenance = (tlpConfig && tlpConfig.provenance) || {};
  return provenance[key] || null;
}

/** TLP writes nothing, or 0, to say "this key asks for no limit". */
function carriesNoLimit(value) {
  const text = String(value === undefined || value === null ? '' : value).trim();
  return text === '' || text === NO_LIMIT;
}

/** Whether TLP reads `path` after the app's own file, so a value there wins. */
function winsOverDropin(path) {
  if (!path || path === DROPIN_PATH) {
    return false;
  }
  if (path === '/etc/tlp.conf') {
    return true;
  }
  const dir = '/etc/tlp.d/';
  if (path.indexOf(dir) === 0) {
    return path.slice(dir.length) > DROPIN_PATH.slice(dir.length);
  }
  return false;
}

function tlpAvailable() {
  return Boolean(tlpConfig && tlpConfig.available !== false && tlpConfig.values);
}

/**
 * What one key is applying, and whether it is this card's own value: an unset
 * Power saver key means TLP is applying the battery one, which is a limit all
 * the same and is shown as what it is.
 */
function fileLimit(source, side) {
  const own = tlpValue(keyFor(source, side));
  if (!carriesNoLimit(own)) {
    return { value: Number(own), inherited: false };
  }

  if (source.fallsBackTo) {
    const parent = sourceById(source.fallsBackTo);
    const inherited = tlpValue(keyFor(parent, side));
    if (!carriesNoLimit(inherited)) {
      return { value: Number(inherited), inherited: true };
    }
  }

  return { value: null, inherited: false };
}

/**
 * The limit a card is showing, in kHz, or null when there is none: a ceiling at
 * the hardware maximum and a floor at the hardware minimum are not limits, so
 * they read as "none" instead of as this chip's number of the day.
 */
function shownLimit(source, side) {
  const limit = fileLimit(source, side);
  if (limit.value === null || !Number.isFinite(limit.value)) {
    return { khz: null, inherited: limit.inherited };
  }
  if (side === 'max' && limit.value >= clockLimits.cpu_hw_max_khz) {
    return { khz: null, inherited: limit.inherited };
  }
  if (side === 'min' && limit.value <= clockLimits.cpu_hw_min_khz) {
    return { khz: null, inherited: limit.inherited };
  }
  return { khz: limit.value, inherited: limit.inherited };
}

/** A frequency in kHz, as a number of GHz a person can read. */
function freqLabel(khz) {
  return (khz / 1000000).toFixed(2) + ' GHz';
}

/** Whether the backend sent no ladder entry: null, or absent. */
function isUnset(value) {
  return value === null || value === undefined;
}

/**
 * How the CPU sliders address frequencies.
 *
 * The sliders stop at indices, not kHz. The backend sends the ladder of
 * frequencies this CPU actually offers, and a slider position then picks an
 * entry, so every stop is a clock the governor can settle on instead of a kHz
 * value the driver would silently round to something else. The ends of the
 * ladder are the exact hardware minimum and maximum, and that is what keeps the
 * top of the slider meaning "no ceiling".
 *
 * An older backend sends no ladder. The range still slides and its two ends
 * still mean the same things, so the slider falls back to raw kHz rather than
 * refusing to render.
 */
function cpuScale() {
  const ladder = clockLimits && clockLimits.cpu_frequencies;
  const frequencies = ladder && Array.isArray(ladder.frequencies) ? ladder.frequencies.slice() : [];

  if (frequencies.length === 0) {
    return { indexed: false, frequencies: null, lastIndex: 0, lowestNonlinearKhz: null };
  }

  frequencies.sort((a, b) => a - b);

  return {
    indexed: true,
    frequencies,
    lastIndex: frequencies.length - 1,
    lowestNonlinearKhz: isUnset(ladder.lowest_nonlinear_khz) ? null : ladder.lowest_nonlinear_khz
  };
}

/** The frequency a slider sits on, in kHz. */
function sliderKhz(slider, scale) {
  if (!scale.indexed) {
    return Number(slider.value);
  }

  const index = Math.min(Math.max(Number(slider.value) || 0, 0), scale.lastIndex);
  return scale.frequencies[index];
}

/** What a card's slider is asking TLP for, in kHz: null means no limit at all. */
function pendingLimit(source, side) {
  const scale = cpuScale();
  const value = sliderKhz(sliderOf(cardOf(source), side), scale);

  if (side === 'max' ? value >= clockLimits.cpu_hw_max_khz : value <= clockLimits.cpu_hw_min_khz) {
    return null;
  }
  return Number.isFinite(value) ? value : null;
}

/** The index whose entry is closest to `khz`. */
function nearestIndex(frequencies, khz) {
  let best = 0;
  for (let i = 1; i < frequencies.length; i += 1) {
    if (Math.abs(frequencies[i] - khz) < Math.abs(frequencies[best] - khz)) {
      best = i;
    }
  }
  return best;
}

/**
 * Where `khz` falls between entries, in index units: 7.12 is 12% of the way
 * from entry 7 to entry 8.
 *
 * The steps are uneven at the ends of the ladder, and a frequency from the
 * kernel such as the efficiency knee usually sits between two entries, so an
 * index rounded to the nearest stop would name the wrong frequency.
 */
function fractionalIndex(frequencies, khz) {
  const last = frequencies.length - 1;

  if (khz <= frequencies[0]) {
    return 0;
  }
  if (khz >= frequencies[last]) {
    return last;
  }

  for (let i = 1; i <= last; i += 1) {
    if (khz <= frequencies[i]) {
      const lower = frequencies[i - 1];
      const span = frequencies[i] - lower;
      return span > 0 ? i - 1 + (khz - lower) / span : i - 1;
    }
  }

  return last;
}

/** Seat a slider on the entry closest to `khz`. */
function setSliderKhz(slider, khz, scale) {
  slider.value = scale.indexed ? String(nearestIndex(scale.frequencies, khz)) : String(khz);
}

/** Whether this card asks TLP for something TLP is not already applying. */
function cardDiffers(source) {
  return LIMIT_SIDES.some(({ side }) => {
    const pending = pendingLimit(source, side);
    const shown = shownLimit(source, side).khz;
    if (pending === null) {
      return shown !== null;
    }
    return shown === null || pending !== shown;
  });
}

/**
 * The changes one card sends: its own two keys, plus every other key the app's
 * drop-in already holds, unchanged.
 *
 * The writer rebuilds that file from the changes it is handed, so a key left out
 * of this list would be deleted from it. Only keys TLP itself reports as coming
 * from the app's file are sent back, and an empty value is not something a
 * change can carry, so such a line is left alone rather than emptied.
 */
function changesForSource(source) {
  const changes = [];
  const provenance = (tlpConfig && tlpConfig.provenance) || {};
  const own = LIMIT_SIDES.map(({ side }) => keyFor(source, side));

  Object.keys(provenance)
    .filter((key) => provenance[key] && provenance[key].path === DROPIN_PATH)
    .filter((key) => own.indexOf(key) === -1)
    .forEach((key) => {
      const held = tlpValue(key);
      if (held !== '') {
        changes.push({ key, value: held });
      }
    });

  LIMIT_SIDES.forEach(({ side }) => {
    const pending = pendingLimit(source, side);
    changes.push({
      key: keyFor(source, side),
      value: pending === null ? NO_LIMIT : String(pending)
    });
  });

  return changes;
}

/** 'CPU maximum (On AC)' — the name a key goes by on the cards. */
function keyLabel(key) {
  for (const source of CEILING_SOURCES) {
    for (const { side } of LIMIT_SIDES) {
      if (keyFor(source, side) === key) {
        return (side === 'max' ? 'CPU maximum ' : 'CPU minimum ') + '(' + source.label + ')';
      }
    }
  }
  return key;
}

/** A value TLP is about to hold, in the words of the card it came from. */
function valueLabel(key, value) {
  if (!carriesNoLimit(value)) {
    return freqLabel(Number(value));
  }
  return key.indexOf('_MAX_') === -1 ? 'no floor' : 'no ceiling';
}

function listOf(items) {
  return items.join(' · ');
}

/**
 * Re-read the ceilings when the daemon moves the active profile on its own: TLP
 * switches its power source with it, so which of these limits is in force can
 * change under the card without anyone touching it. A staged edit is never
 * thrown away to do that.
 */
function watchPowerProfile() {
  const event = window.__TAURI__ && window.__TAURI__.event;
  if (profileWatch || !event) {
    return;
  }

  profileWatch = event.listen('power-profile-changed', () => {
    if (CEILING_SOURCES.some((source) => cardOf(source) && cardDiffers(source))) {
      return;
    }
    loadClockLimits();
  });
}

/**
 * Re-read the ceilings when the app settles the GPU.
 *
 * `lactd` starting takes the card from this app and re-applies its own level, and
 * a power-source change moves which column the app is enforcing; the app notices
 * both on its own pass and writes, which is a change to the GPU that the columns
 * above cannot see. A staged edit is never thrown away to report it.
 */
function watchGpuClock() {
  const event = window.__TAURI__ && window.__TAURI__.event;
  if (gpuWatch || !event) {
    return;
  }

  gpuWatch = event.listen('gpu-clock-settled', () => {
    if (CEILING_SOURCES.some((source) => cardOf(source) && cardDiffers(source))) {
      return;
    }
    loadClockLimits();
  });
}

/**
 * Read the hardware's clock ranges, TLP's ceilings and the GPU levels together:
 * no one of them is enough to draw a card.
 */
async function loadClockLimits() {
  const container = document.querySelector('.clock-cards');
  if (!container) {
    return;
  }

  setupClockHandlers(container);
  setupGpuHandlers();
  watchPowerProfile();
  watchGpuClock();

  let limits;
  let config;
  try {
    const [limitsResponse, configResponse] = await Promise.all([
      invoke('get_clock_limits'),
      invoke('tlp_config')
    ]);
    limits = unwrap(limitsResponse);
    config = unwrap(configResponse);
  } catch (error) {
    showStatus('Could not read the clock limits: ' + error.message, 'error');
    return;
  }

  if (!limits) {
    showStatus('Could not read the clock limits', 'error');
    return;
  }

  clockLimits = limits;
  tlpConfig = config || null;
  previewTokens.clear();

  CEILING_SOURCES.forEach((source) => {
    const card = cardOf(source);
    const line = card && card.querySelector('[data-role="pending"]');
    if (line) {
      line.hidden = true;
      line.textContent = '';
      line.classList.remove('is-warn');
    }
  });

  renderClockCards();
}

/**
 * Mark where this CPU stops scaling efficiently, on the maximum slider.
 *
 * amd_pstate's lowest non-linear frequency is where the CPU leaves its linear
 * region; below it every extra MHz costs disproportionately more voltage. The
 * marker is a tick with a tooltip, never a stop: the knee is not one of the
 * ladder's entries and must not become a selectable frequency.
 */
function renderKnee(card, scale) {
  const marker = card.querySelector('[data-role="cpu-knee"]');
  const note = card.querySelector('[data-role="cpu-knee-note"]');
  if (!marker || !note) {
    return;
  }

  const knee = scale.lowestNonlinearKhz;
  if (!scale.indexed || isUnset(knee)) {
    marker.hidden = true;
    note.hidden = true;
    return;
  }

  // The thumb is 22px wide, so its centre travels from 11px to width - 11px.
  // Insetting the marker by the same amount keeps it under the thumb at the
  // frequency it names.
  const position =
    scale.lastIndex > 0 ? fractionalIndex(scale.frequencies, knee) / scale.lastIndex : 0;
  marker.style.left =
    'calc(' + (position * 100).toFixed(2) + '% + ' + (11 - 22 * position).toFixed(2) + 'px)';
  marker.title = 'Efficiency knee at ' + freqLabel(knee);
  marker.hidden = false;

  note.textContent =
    'Below ' +
    freqLabel(knee) +
    ' the CPU runs in its linear region, where scaling costs more ' +
    'voltage per MHz.';
  note.hidden = true;
}

/**
 * Where this card's numbers come from, when that is neither TLP's own default
 * nor the app's file: an unset Power saver card follows the battery one, and a
 * key set in /etc/tlp.conf keeps winning because TLP reads that file last.
 */
function renderSourceNote(card, source) {
  const note = card.querySelector('[data-role="source"]');
  if (!note) {
    return;
  }

  if (!tlpAvailable()) {
    note.textContent = "TLP's configuration could not be read, so these limits cannot be shown.";
    note.hidden = false;
    return;
  }

  if (LIMIT_SIDES.some(({ side }) => fileLimit(source, side).inherited)) {
    note.textContent = 'Not set for Power saver: TLP applies the on-battery limits here.';
    note.hidden = false;
    return;
  }

  const elsewhere = LIMIT_SIDES.map(({ side }) => keyFor(source, side)).find((key) => {
    const entry = tlpSource(key);
    return Boolean(entry) && winsOverDropin(entry.path) && !carriesNoLimit(tlpValue(key));
  });

  if (elsewhere) {
    note.textContent =
      'TLP also reads this one from ' +
      tlpSource(elsewhere).path +
      ", which comes after this app's file, so that file wins.";
    note.hidden = false;
    return;
  }

  note.hidden = true;
  note.textContent = '';
}

/** The three cards: their ranges, their seats, their knee and their wording. */
function renderClockCards() {
  const hwMin = clockLimits.cpu_hw_min_khz;
  const hwMax = clockLimits.cpu_hw_max_khz;
  const usable = hwMax > hwMin && tlpAvailable();
  const scale = cpuScale();

  CEILING_SOURCES.forEach((source) => {
    const card = cardOf(source);
    if (!card) {
      return;
    }

    LIMIT_SIDES.forEach(({ side }) => {
      const slider = sliderOf(card, side);

      if (scale.indexed) {
        // Positions are entries in the ladder, so the last stop is the exact
        // hardware maximum. A kHz slider with a fixed step cannot say that: the
        // browser stops at the largest min + k * step, which turned the uncapped
        // position into a real ceiling of 4.21 GHz on a 4.29 GHz chip.
        slider.min = '0';
        slider.max = String(scale.lastIndex);
        slider.step = '1';
      } else {
        slider.min = String(hwMin);
        slider.max = String(hwMax);
        slider.step = 'any';
      }

      // Without cpufreq, or without TLP to write to, there is nothing honest to
      // offer, so the controls say so by being unusable rather than by
      // pretending.
      slider.disabled = !usable;

      if (usable) {
        const shown = shownLimit(source, side).khz;
        const end = side === 'max' ? hwMax : hwMin;
        setSliderKhz(slider, shown === null ? end : shown, scale);
      }
    });

    card.querySelectorAll('.cap-apply, .cap-uncap').forEach((button) => {
      button.disabled = !usable;
    });

    renderKnee(card, scale);
    renderSourceNote(card, source);
    updateClockReadouts(card);
  });

  renderGpuControl();
}

/** A GPU level's clock, read off the levels the card offers. */
function gpuFreqLabel(levels, level) {
  const entry = levels.find((one) => one.level === Number(level));
  return entry ? entry.freq_mhz + ' MHz' : 'level ' + level;
}

/**
 * The graphics-clock columns: one ceiling per power source, none of them a TLP
 * setting.
 *
 * The columns are not staged and previewed the way the CPU cards are. TLP applies
 * the CPU limits itself, on its own schedule, so a change there is a request until
 * Apply; a GPU level is written by this app the moment it is chosen and is
 * therefore already in force. What the columns do share is the source list: the
 * ceiling worth asking for depends on where the power comes from, and the machine
 * moves between those sources on its own.
 *
 * A column that is not the one in force still takes a value — it is a promise for
 * the next time the machine is there, and the pass in the backend applies it then.
 * The column that is in force says so, because the value alone cannot.
 */
function renderGpuControl() {
  const cards = Array.from(document.querySelectorAll('.gpu-cards .gpu-card'));
  if (!cards.length) {
    return;
  }

  const found = Boolean(clockLimits.gpu_found);
  const levels = (clockLimits.gpu_levels || []).slice().sort((a, b) => a.level - b.level);
  const fastest = levels[levels.length - 1];
  const held = clockLimits.gpu_held_levels || {};

  const hint = document.querySelector('[data-role="gpu-hint"]');
  const none = document.querySelector('[data-role="gpu-none"]');
  const columns = document.querySelector('.gpu-cards');
  if (hint) {
    hint.hidden = !found;
  }
  if (none) {
    none.hidden = found;
  }
  if (columns) {
    columns.hidden = !found;
  }

  if (found) {
    cards.forEach((card) => {
      const source = sourceById(card.dataset.source);
      const select = card.querySelector('.cap-gpu-level');
      if (!source || !select) {
        return;
      }

      select.innerHTML = '';

      const uncapped = document.createElement('option');
      uncapped.value = '';
      uncapped.textContent = 'No ceiling — every level, up to ' + fastest.freq_mhz + ' MHz';
      select.appendChild(uncapped);

      levels.forEach((level) => {
        const option = document.createElement('option');
        option.value = String(level.level);
        option.textContent = 'Up to ' + level.freq_mhz + ' MHz';
        select.appendChild(option);
      });

      // The ceiling this app holds for the column is what it shows, because it is
      // what the app enforces: the live level is only where the driver happens to
      // be. Nothing held means nothing enforced, and the column says that too.
      const ceiling = held[source.gpuKey];
      select.value = isUnset(ceiling) ? '' : String(ceiling);
      select.disabled = false;

      const label = isUnset(ceiling) ? 'No ceiling' : 'Up to ' + gpuFreqLabel(levels, ceiling);
      const active = card.dataset.source === clockLimits.gpu_source;
      card.classList.toggle('is-active', active);

      const state = card.querySelector('[data-role="gpu-state"]');
      if (state) {
        state.textContent = (active ? 'In force now · ' : '') + label;
        state.classList.toggle('is-capped', !isUnset(ceiling));
      }

      const readout = card.querySelector('[data-role="gpu-readout"]');
      if (readout) {
        readout.textContent = label;
      }
    });
  }

  const handback = document.querySelector('.gpu-handback');
  if (handback) {
    bindOnce(handback, 'click', handBackGpu);
  }
  renderGpuOwner();
}

/**
 * Say who writes the GPU's level, because the control above cannot show it.
 *
 * A level written here only survives while LACT is not managing the card: lactd
 * re-applies its own on the next tick otherwise. The app asks it to let go before
 * capping, so this line reports which of the two writes the knob right now — the
 * one fact about this control that sysfs cannot answer.
 *
 * Who holds it is not the whole story, and the wording has to carry the rest:
 * LACT letting go is not a setting that stays let go. It is a runtime detach,
 * and the next `lactd` start — a reboot, an upgrade, a `systemctl restart` —
 * takes the card back and re-applies `low`. So the ceiling is held by this app
 * re-writing it, not by a promise LACT made, and every line here says that in
 * the terms that apply: the app re-applies the level it holds while it runs.
 *
 * The line answers for the columns as a whole, because the card is one card: a
 * ceiling held for any source is a claim on the GPU. It is what keeps the line
 * honest when no column holds one — then this app is writing nothing to the card
 * at all, and saying who is would be inventing an owner.
 */
function renderGpuOwner() {
  const line = document.querySelector('[data-role="gpu-owner"]');
  const handback = document.querySelector('.gpu-handback');
  if (!line) {
    return;
  }

  if (!clockLimits.gpu_found) {
    line.hidden = true;
    line.textContent = '';
    if (handback) {
      handback.hidden = true;
    }
    return;
  }

  const held = clockLimits.gpu_held_levels || {};
  const holds = CEILING_SOURCES.some((source) => !isUnset(held[source.gpuKey]));
  if (!holds) {
    line.hidden = false;
    line.textContent =
      'No column holds a ceiling, so this app is writing nothing to the GPU: the card is ' +
      "LACT's, or the driver's, until one is chosen. Choosing one takes the card from LACT " +
      'while the app holds it. The exception is a ceiling written for another source and ' +
      'still in force when the machine moves here: that one is given up on the next check.';
    if (handback) {
      handback.hidden = true;
    }
    return;
  }

  const says = {
    uncontested:
      'Nothing but this app writes this level: LACT is not running here. A ceiling chosen ' +
      'above is written once and re-applied while the app runs.',
    lact:
      'LACT is managing this GPU right now and applies its own level to it, so it would ' +
      'revert a ceiling set here. Choosing one takes the GPU from LACT first — and if that ' +
      'happens to a ceiling this app is already holding, the app takes the card back and ' +
      'writes the ceiling again within a few seconds.',
    thinkpro:
      'This app is holding this GPU: LACT let go of the card and the ceiling above is in ' +
      'force. That is not a setting LACT keeps — its next restart takes the card back and ' +
      're-applies its own level — so the app re-applies the ceiling it holds within a few ' +
      'seconds of losing it, for as long as it is running. Hand the GPU back to LACT to ' +
      'give the card up.',
    unknown:
      'LACT is running but did not answer, so which of the two writes this level is not ' +
      'decided. The ceiling this app holds is re-applied either way while it runs.'
  };

  line.hidden = false;
  line.textContent = says[clockLimits.gpu_owner] || says.unknown;

  // The button is the hand-back that does not also remove the ceiling; the
  // "No ceiling" option at the top of the list is the hand-back that does.
  if (handback) {
    handback.hidden = clockLimits.gpu_owner !== 'thinkpro';
  }
}

/**
 * Redraw a card's numbers from its own controls, so an edit that has not been
 * applied yet still shows what it would do.
 */
function updateClockReadouts(card) {
  const scale = cpuScale();
  const hwMin = clockLimits.cpu_hw_min_khz;
  const hwMax = clockLimits.cpu_hw_max_khz;

  const maxValue = sliderKhz(card.querySelector('.cap-cpu-max'), scale);
  const minValue = sliderKhz(card.querySelector('.cap-cpu-min'), scale);

  card.querySelector('[data-role="cpu-max-value"]').textContent =
    maxValue >= hwMax ? 'No ceiling' : freqLabel(maxValue);
  card.querySelector('[data-role="cpu-min-value"]').textContent =
    minValue <= hwMin ? 'Hardware floor' : freqLabel(minValue);

  // The knee is a frequency, not an index: on this machine it sits between two
  // of the ladder's entries, so it is compared in kHz.
  const knee = scale.lowestNonlinearKhz;
  const kneeNote = card.querySelector('[data-role="cpu-knee-note"]');
  if (kneeNote) {
    kneeNote.hidden = isUnset(knee) || maxValue >= knee;
  }

  const parts = [];
  if (maxValue < hwMax) {
    parts.push('CPU at or below ' + freqLabel(maxValue));
  }
  if (minValue > hwMin) {
    parts.push('CPU at or above ' + freqLabel(minValue));
  }

  const state = card.querySelector('[data-role="state"]');
  state.textContent = parts.length ? parts.join(' · ') : 'No limit from here';
  state.classList.toggle('is-capped', parts.length > 0);
}

function setClockBusy(busy) {
  document.querySelectorAll('.clock-card, .clock-gpu').forEach((block) => {
    block.classList.toggle('is-busy', busy);
    block.querySelectorAll('button').forEach((button) => {
      button.disabled = busy;
    });
  });

  document.querySelectorAll('.cap-gpu-level').forEach((select) => {
    select.disabled = busy || !clockLimits.gpu_found;
  });
}

/**
 * Ask TLP whether it would accept what this card is asking for, and say so in
 * the card. Nothing privileged happens here: a preview only reads.
 */
async function previewSource(source) {
  const card = cardOf(source);
  const line = card.querySelector('[data-role="pending"]');

  if (!cardDiffers(source)) {
    line.hidden = true;
    line.textContent = '';
    line.classList.remove('is-warn');
    return;
  }

  const token = (previewTokens.get(source.id) || 0) + 1;
  previewTokens.set(source.id, token);
  line.hidden = false;
  line.classList.remove('is-warn');
  line.textContent = 'Checking what TLP would accept...';

  let preview;
  try {
    preview = unwrap(await invoke('power_preview_changes', { changes: changesForSource(source) }));
  } catch (error) {
    if (previewTokens.get(source.id) === token) {
      line.classList.add('is-warn');
      line.textContent = 'These values could not be checked: ' + error.message;
    }
    return;
  }

  if (previewTokens.get(source.id) !== token) {
    return;
  }

  if (!preview || preview.available === false) {
    line.classList.add('is-warn');
    line.textContent =
      (preview && preview.reason) || 'These values cannot be checked against the installed TLP.';
    return;
  }

  // Only this card's own keys are spoken about here: the rest of the list is the
  // file's other settings, sent back untouched.
  const own = LIMIT_SIDES.map(({ side }) => keyFor(source, side));
  const mine = (rows) => (rows || []).filter((row) => own.indexOf(row.key) !== -1);
  const rejected = mine(preview.rejected);
  const accepted = mine(preview.accepted);
  const lost = mine(preview.overridden).filter((row) => winsOverDropin(row.path));
  const notes = [];

  if (rejected.length) {
    notes.push(
      'TLP will not take ' +
        listOf(rejected.map((row) => keyLabel(row.key) + ': ' + row.reason)) +
        '.'
    );
  }
  if (accepted.length) {
    notes.push(
      'Ready to write ' +
        listOf(accepted.map((row) => keyLabel(row.key) + ' at ' + valueLabel(row.key, row.value))) +
        '.'
    );
  }
  if (lost.length) {
    notes.push(
      lost[0].path +
        ' sets ' +
        listOf(lost.map((row) => keyLabel(row.key))) +
        ' too, and TLP reads that file later, so it wins.'
    );
  }
  if ((preview.accepted || []).length > accepted.length) {
    notes.push("The file's other settings are sent back as they are.");
  }

  line.classList.toggle('is-warn', rejected.length > 0 || lost.length > 0);
  line.textContent = notes.length ? notes.join(' ') : 'There is nothing to write for this card.';
}

/** What the writer reported, said in one line. */
function appliedMessage(applied) {
  const written = applied.applied.length;
  const parts = [
    written + (written === 1 ? ' setting' : ' settings') + ' written to ' + applied.path
  ];

  parts.push(
    applied.tlp_restarted
      ? 'TLP was restarted, so they are in effect now'
      : 'TLP is not running, so they take effect when it next starts'
  );

  const lost = (applied.overridden || []).filter((row) => winsOverDropin(row.path));
  if (lost.length) {
    parts.push(
      lost[0].path + ' also sets ' + listOf(lost.map((row) => keyLabel(row.key))) + ' and wins'
    );
  }
  (applied.rejected || []).forEach((row) => {
    parts.push('TLP refused ' + keyLabel(row.key) + ': ' + row.reason);
  });

  return parts.join('. ') + '.';
}

/**
 * Write one card's settings to TLP. The privileged step raises a PolicyKit
 * password prompt, so a dismissed prompt is reported as "nothing was written"
 * rather than as a failure, and the card is then read back from TLP's own file
 * so it shows what the machine holds rather than what it was told.
 */
async function writeSource(source) {
  if (!tlpAvailable()) {
    showStatus('There is no TLP configuration to write to', 'error');
    return;
  }
  if (!cardDiffers(source)) {
    showStatus('Nothing to write: this card already reads that way', 'info');
    return;
  }

  setClockBusy(true);
  try {
    const changes = changesForSource(source);
    const preview = unwrap(await invoke('power_preview_changes', { changes }));

    if (!preview || preview.available === false) {
      throw new Error((preview && preview.reason) || 'The installed TLP cannot be read.');
    }
    if (!preview.accepted.length) {
      const first = (preview.rejected || [])[0];
      throw new Error(first ? keyLabel(first.key) + ': ' + first.reason : 'Nothing was accepted.');
    }

    const applied = unwrap(await invoke('power_apply_changes', { changes: preview.accepted }));
    showStatus(appliedMessage(applied), 'success');
  } catch (error) {
    showStatus(
      /dismiss|not authorized|cancel/i.test(error.message)
        ? 'Nothing was written: the password prompt was dismissed.'
        : error.message,
      'error'
    );
  } finally {
    setClockBusy(false);
    await loadClockLimits();
  }
}

/**
 * Put one card back to no ceiling and no floor, and write that to TLP.
 *
 * TLP treats an unset key and a key of 0 the same way -- it leaves the knob
 * alone -- so 0 is what says "no limit here" in the file.
 */
async function clearSource(source) {
  const card = cardOf(source);
  const scale = cpuScale();

  setSliderKhz(sliderOf(card, 'max'), clockLimits.cpu_hw_max_khz, scale);
  setSliderKhz(sliderOf(card, 'min'), clockLimits.cpu_hw_min_khz, scale);
  updateClockReadouts(card);

  await writeSource(source);
}

/**
 * Write the graphics clock ceiling for one power source.
 *
 * It is an amdgpu level rather than a TLP setting, so the column that is in force
 * takes effect now and the machine feels it immediately; a column that is not in
 * force is remembered for the next time the machine is there. Clearing a column is
 * its own verb: "no ceiling here" and "a ceiling of level 0" are different
 * answers, and the backend is what writes the difference to the GPU.
 */
async function writeGpuLevel(source, raw) {
  setClockBusy(true);
  try {
    const response =
      raw === ''
        ? await invoke('clear_gpu_clock_cap', { source: source.id })
        : await invoke('set_gpu_clock_level', { source: source.id, level: Number(raw) });
    showStatus(unwrap(response) || 'GPU clock ceiling updated', 'success');
  } catch (error) {
    showStatus(error.message, 'error');
  } finally {
    setClockBusy(false);
    await loadClockLimits();
  }
}

/**
 * Bind the GPU columns once.
 *
 * One `change` on a select is the whole interaction. Unlike the CPU cards there
 * is nothing to stage and no preview to ask for: the write is this app's own, and
 * the command answers with what actually happened.
 */
function setupGpuHandlers() {
  return bindOnce(document.querySelector('.gpu-cards'), 'change', (event) => {
    if (!event.target.matches('.cap-gpu-level')) {
      return;
    }

    const card = event.target.closest('.gpu-card');
    const source = card ? sourceById(card.dataset.source) : null;
    if (source) {
      writeGpuLevel(source, event.target.value);
    }
  });
}

/**
 * Give the card up: LACT manages the GPU again and this app stops holding a
 * ceiling.
 *
 * Different from choosing "No ceiling": that writes every level back to `auto`
 * before handing the card over, where this leaves the level list alone and lets
 * LACT's own config take it from there. What both do is end the claim — the held
 * ceiling is dropped with the card, or the app would take the GPU straight back
 * a few seconds later and the button would look broken.
 */
async function handBackGpu() {
  setClockBusy(true);
  try {
    showStatus(unwrap(await invoke('reattach_gpu')) || 'Handed the GPU back to LACT', 'success');
  } catch (error) {
    showStatus(error.message, 'error');
  } finally {
    setClockBusy(false);
    await loadClockLimits();
  }
}

function setupClockHandlers(container) {
  // bindOnce drops the previous listeners by cloning the node, and `cloneNode`
  // copies children but never listeners — so it may be used once per element and
  // every listener has to hang off the clone it returns. Binding the container
  // three times in a row detached it on the first call and threw on the second,
  // before either clock command could run.
  const fresh = bindOnce(container, 'input', (event) => {
    const source = sourceOf(event.target);
    if (source && event.target.matches('.cap-cpu-max, .cap-cpu-min')) {
      updateClockReadouts(cardOf(source));
    }
  });
  if (!fresh) {
    return;
  }

  // Letting go of a slider stages the value and asks TLP what it would do with
  // it. Only Apply writes, so nothing privileged happens while a slider moves.
  fresh.addEventListener('change', (event) => {
    const source = sourceOf(event.target);
    if (!source || !event.target.matches('.cap-cpu-max, .cap-cpu-min')) {
      return;
    }
    updateClockReadouts(cardOf(source));
    previewSource(source);
  });

  fresh.addEventListener('click', (event) => {
    const source = sourceOf(event.target);
    if (!source) {
      return;
    }

    if (event.target.matches('.cap-apply')) {
      writeSource(source);
    } else if (event.target.matches('.cap-uncap')) {
      clearSource(source);
    }
  });
}
