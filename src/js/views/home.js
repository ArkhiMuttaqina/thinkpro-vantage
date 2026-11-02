// Home: what this machine is, and what it is doing right now.
//
// One poll of `get_system_overview` feeds the whole page. Two readings cost a
// privileged helper call and are therefore fetched once per session rather than
// polled: the DMI memory table and the NVMe SMART counters. Package power comes
// from the same helper but is cheap to sample, so it runs on a slower timer of
// its own.

import { invoke, runWithState, errorText } from '../ui.js';

const PACKAGE_POWER_MS = 15000;

// The overview poll is driven by app.js's tracked interval, which already gates
// on the active view. Only the slow package-power sample needs its own timer.
let powerTimer = null;
let lastOverview = null;
// Readings that cost a privileged helper call are held as state rather than
// written straight into the DOM: the 2-second overview poll rebuilds every
// metric list, so anything patched into the DOM alone disappears on the next
// tick. Keeping them here means one render path for all of it.
let packageWatts = null;
let memoryModules = null;
let memoryDetailFailed = false;
let storageHealth = null;
let storageDetailFailed = false;

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

function formatNumber(value, digits = 0) {
  if (value === null || value === undefined || Number.isNaN(value)) {
    return '—';
  }
  return Number(value).toFixed(digits);
}

function formatGb(gigabytes, digits = 1) {
  if (gigabytes === null || gigabytes === undefined) {
    return '—';
  }
  if (gigabytes >= 1000) {
    return formatNumber(gigabytes / 1000, 2) + ' TB';
  }
  return formatNumber(gigabytes, digits) + ' GB';
}

function formatMb(megabytes) {
  if (megabytes === null || megabytes === undefined) {
    return '—';
  }
  if (megabytes >= 1024) {
    return formatNumber(megabytes / 1024, 1) + ' GB';
  }
  return formatNumber(megabytes) + ' MB';
}

function formatWatts(watts, digits = 2) {
  if (watts === null || watts === undefined) {
    return '—';
  }
  return formatNumber(watts, digits) + ' W';
}

function formatMhz(mhz) {
  if (!mhz) {
    return '—';
  }
  if (mhz >= 1000) {
    return formatNumber(mhz / 1000, 2) + ' GHz';
  }
  return formatNumber(mhz) + ' MHz';
}

function formatHours(hours, digits = 0) {
  if (hours === null || hours === undefined) {
    return '—';
  }
  return Number(hours).toFixed(digits) + ' h';
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

function metric(label, value, meta) {
  const row = document.createElement('div');
  row.className = 'overview-metric';
  const key = document.createElement('dt');
  key.className = 'overview-metric-label';
  key.textContent = label;
  const amount = document.createElement('dd');
  amount.className = 'overview-metric-value';
  amount.textContent = value;
  if (meta) {
    const suffix = document.createElement('span');
    suffix.className = 'overview-metric-meta';
    suffix.textContent = meta;
    amount.append(' ', suffix);
  }
  row.append(key, amount);
  return row;
}

/** Replace a metric list wholesale; text content only, never markup. */
function setMetrics(containerId, rows) {
  const host = document.getElementById(containerId);
  if (!host) {
    return;
  }
  host.replaceChildren(
    ...rows.filter(Boolean).map(([label, value, meta]) => metric(label, value, meta))
  );
}

function setText(id, value) {
  const element = document.getElementById(id);
  if (element && value !== undefined && value !== null) {
    element.textContent = value;
  }
}

function setBusy(id, busy) {
  const element = document.getElementById(id);
  if (element) {
    element.setAttribute('aria-busy', busy ? 'true' : 'false');
  }
}

function renderIdentity(identity, helperAvailable) {
  setText('overview-vendor', identity.vendor || 'System');
  setText('overview-model', identity.model || 'Unknown model');
  setText('overview-model-code', identity.model_code || '');
  setText('overview-bios', identity.bios_version ? 'BIOS ' + identity.bios_version : '');
  setBusy('overview-identity', false);
  const button = document.getElementById('btn-copy-serial');
  if (button) {
    button.disabled = !helperAvailable;
    button.title = helperAvailable
      ? "Copy this machine's serial number"
      : 'The privileged helper is not installed, so the serial cannot be read';
  }
}

function renderCpu(cpu) {
  setText('cpu-model', cpu.model || 'Unknown processor');
  setText('cpu-governor', cpu.governor || '');
  setText('cpu-metrics-extra', '');
  setMetrics('cpu-metrics', [
    ['Clock', formatMhz(cpu.current_mhz), 'of ' + formatMhz(cpu.max_mhz)],
    ['Cores', cpu.cores ? cpu.cores + ' cores / ' + cpu.threads + ' threads' : '—'],
    ['Temperature', cpu.temperature_c !== null ? formatNumber(cpu.temperature_c, 1) + ' °C' : '—']
  ]);
  setBusy('card-cpu', false);
}

function renderGpu(gpu) {
  if (!gpu) {
    setText('gpu-name', 'No GPU reported');
    setText('gpu-driver', '');
    setMetrics('gpu-metrics', []);
    setBusy('card-gpu', false);
    return;
  }
  setText('gpu-name', gpu.name || 'Graphics');
  setText('gpu-driver', gpu.driver || '');
  setMetrics('gpu-metrics', [
    ['Clock', formatMhz(gpu.current_mhz), gpu.max_mhz ? 'of ' + formatMhz(gpu.max_mhz) : ''],
    ['Load', gpu.busy_percent !== null ? gpu.busy_percent + ' %' : '—'],
    ['Temperature', gpu.temperature_c !== null ? formatNumber(gpu.temperature_c, 1) + ' °C' : '—'],
    ['Power', formatWatts(gpu.watts)],
    [
      'VRAM',
      formatMb(gpu.vram_total_mb),
      gpu.vram_used_mb !== null ? formatMb(gpu.vram_used_mb) + ' used' : ''
    ],
    ['Performance level', gpu.dpm_level || '—']
  ]);
  setBusy('card-gpu', false);
}

function renderMemory(memory) {
  setText(
    'memory-summary',
    formatMb(memory.used_mb) + ' of ' + formatMb(memory.total_mb) + ' used'
  );
  const bar = document.getElementById('memory-bar');
  if (bar) {
    bar.style.width = Math.min(100, Math.max(0, memory.usage_percent || 0)) + '%';
  }
  const modules = memoryModules ?? memory.modules ?? [];
  // The overview carries no module speed; the DMI table does, once read. Prefer
  // it over the placeholder so the chip does not revert on the next poll.
  const speedSource = modules.find((module) => module.configured_speed_mts || module.speed_mts);
  const speedMts =
    memory.speed_mts ||
    (speedSource ? speedSource.configured_speed_mts || speedSource.speed_mts : null) ||
    null;
  setText('memory-speed', speedMts ? speedMts + ' MT/s' : 'speed unknown');
  const rows = [
    ['Usage', formatNumber(memory.usage_percent, 1) + ' %'],
    ['Available', formatMb(memory.available_mb)],
    [
      'Swap',
      memory.swap_total_mb
        ? formatMb(memory.swap_used_mb) + ' of ' + formatMb(memory.swap_total_mb)
        : 'not configured'
    ]
  ];
  for (const module of modules) {
    const moduleSpeed = module.configured_speed_mts || module.speed_mts;
    rows.push([
      module.locator || 'Module',
      formatMb(module.size_mb),
      [
        module.kind,
        moduleSpeed ? moduleSpeed + ' MT/s' : '',
        module.manufacturer,
        module.part_number
      ]
        .filter(Boolean)
        .join(' · ')
    ]);
  }
  if (!modules.length && memoryDetailFailed) {
    rows.push(['Modules', 'not readable without the helper']);
  }
  setMetrics('memory-metrics', rows);
  setBusy('card-memory', false);
}

function renderStorage(devices) {
  const device = (devices || [])[0];
  if (!device) {
    setText('storage-model', 'No storage reported');
    setMetrics('storage-metrics', []);
    setBusy('card-storage', false);
    return;
  }
  setText('storage-model', device.model || device.device);
  const health = storageHealth ?? device.health;
  setText(
    'storage-health',
    health ? (health.passed ? 'healthy' : 'SMART failure') : 'health unknown'
  );
  const rows = [
    ['Capacity', formatGb(device.size_gb)],
    [
      'Temperature',
      device.temperature_c !== null ? formatNumber(device.temperature_c, 1) + ' °C' : '—'
    ],
    ['Firmware', device.firmware || '—'],
    [
      'Read / write',
      (device.read_mb_s !== null ? formatNumber(device.read_mb_s, 1) : '—') +
        ' / ' +
        (device.write_mb_s !== null ? formatNumber(device.write_mb_s, 1) : '—') +
        ' MB/s'
    ]
  ];
  if (health) {
    rows.push(['Wear', health.percentage_used !== null ? health.percentage_used + ' % used' : '—']);
    rows.push([
      'Spare',
      health.available_spare_percent !== null
        ? health.available_spare_percent + ' % available'
        : '—'
    ]);
    rows.push([
      'Written',
      health.data_written_tb !== null ? formatNumber(health.data_written_tb, 1) + ' TB' : '—',
      health.power_on_hours !== null ? formatHours(health.power_on_hours) + ' powered' : ''
    ]);
    rows.push([
      'Errors',
      health.media_errors !== null ? String(health.media_errors) : '—',
      health.critical_warning ? 'critical warning ' + health.critical_warning : ''
    ]);
  } else if (storageDetailFailed) {
    rows.push(['SMART', 'not readable without the helper']);
  }
  setMetrics('storage-metrics', rows);
  setBusy('card-storage', false);
}

let profileValue = null;

function renderProfiles(profile) {
  const host = document.getElementById('overview-profiles');
  if (!host || !profile) {
    return;
  }
  const choices = profile.available || [];
  if (!choices.length) {
    host.replaceChildren();
    return;
  }
  profileValue = profile.current || profileValue;
  host.replaceChildren(
    ...choices.map((choice) => {
      const label = typeof choice === 'string' ? choice : choice.name || choice.id;
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'overview-profile' + (label === profileValue ? ' is-active' : '');
      button.dataset.profile = label;
      button.textContent = label.replace(/-/g, ' ');
      button.setAttribute('aria-pressed', label === profileValue ? 'true' : 'false');
      return button;
    })
  );
}

function renderPower(power, packagePower) {
  setText('power-source', power.ac_online ? 'AC connected' : 'On battery');
  const rows = [
    ['Battery', power.capacity !== null ? power.capacity + ' %' : '—', power.status || ''],
    [
      power.charging ? 'Charging at' : 'Discharging at',
      formatWatts(power.battery_watts, 1),
      power.voltage_v ? formatNumber(power.voltage_v, 2) + ' V' : ''
    ],
    ['Current', power.current_a ? formatNumber(power.current_a, 2) + ' A' : '—'],
    [
      'Energy',
      power.energy_full_wh
        ? formatNumber(power.energy_now_wh, 1) +
          ' of ' +
          formatNumber(power.energy_full_wh, 1) +
          ' Wh'
        : '—'
    ],
    [
      power.charging ? 'Full in' : 'Runtime',
      // get_power_consumption reports both of these in hours already.
      power.charging
        ? formatHours(power.time_to_full_hours, 1)
        : formatHours(power.runtime_hours, 1)
    ],
    [
      'CPU package',
      packagePower && packagePower.watts !== null && packagePower.watts !== undefined
        ? formatWatts(packagePower.watts, 1)
        : '—'
    ]
  ];
  setMetrics('power-metrics', rows);
  const note = document.getElementById('power-note');
  if (note) {
    if (packagePower && !packagePower.available && packagePower.reason) {
      note.textContent = 'CPU package power unavailable: ' + packagePower.reason;
    } else if (packagePower && packagePower.watts === null) {
      note.textContent = 'CPU package power is sampled every few seconds.';
    } else {
      note.textContent = '';
    }
  }
  setBusy('overview-power', false);
}

function render(overview) {
  renderIdentity(overview.identity || {}, overview.helper_available);
  renderCpu(overview.cpu || {});
  renderGpu(overview.gpu);
  renderMemory(overview.memory || {});
  renderStorage(overview.storage || []);
  renderPower(overview.power || {}, packageWatts);
}

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

function unwrap(response) {
  if (!response || response.success === false) {
    throw new Error(errorText(response && response.error));
  }
  return response.data;
}

function homeVisible() {
  const host = document.getElementById('home-view');
  return Boolean(host) && host.offsetParent !== null;
}

async function refreshOverview() {
  if (!homeVisible()) {
    return;
  }
  try {
    const overview = unwrap(await invoke('get_system_overview'));
    const firstPass = lastOverview === null;
    lastOverview = overview;
    render(overview);
    if (firstPass) {
      void loadMemoryDetail();
      void loadStorageHealth();
    }
    void loadProfile();
  } catch (error) {
    console.error('[Overview] Failed to read the system overview:', error);
  }
}

/**
 * The DMI memory table, read once per session.
 *
 * The result becomes state and the card is re-rendered, rather than the rows
 * being appended: appending would be wiped by the next two-second overview poll.
 */
async function loadMemoryDetail() {
  try {
    const detail = unwrap(await invoke('get_memory_speed'));
    const modules = (detail && detail.modules) || [];
    memoryModules = modules;
    memoryDetailFailed = modules.length === 0;
    if (lastOverview) {
      renderMemory(lastOverview.memory || {});
    }
  } catch (error) {
    memoryDetailFailed = true;
    if (lastOverview) {
      renderMemory(lastOverview.memory || {});
    }
    console.warn('[Overview] Memory details unavailable:', errorText(error));
  }
}

/**
 * NVMe SMART counters, read once per session.
 *
 * Held as state and rendered through the storage card for the same reason as the
 * memory table: the overview poll rebuilds that card every two seconds.
 */
async function loadStorageHealth() {
  try {
    const detail = unwrap(await invoke('get_storage_health'));
    const health = detail && detail.health;
    if (health) {
      storageHealth = health;
    } else {
      storageDetailFailed = true;
    }
  } catch (error) {
    storageDetailFailed = true;
    console.warn('[Overview] Storage health unavailable:', errorText(error));
  }
  if (lastOverview) {
    renderStorage(lastOverview.storage || []);
  }
}

async function loadProfile() {
  try {
    const profile = unwrap(await invoke('get_power_profile'));
    renderProfiles(profile);
  } catch (error) {
    console.warn('[Overview] Power profile unavailable:', errorText(error));
  }
}

/**
 * CPU package power, on its own slower timer.
 *
 * Every sample costs a privileged helper call, so it stays out of the
 * two-second overview poll.
 */
async function loadPackagePower() {
  if (!homeVisible()) {
    return;
  }
  try {
    packageWatts = unwrap(await invoke('get_package_watts'));
  } catch (error) {
    packageWatts = { available: false, reason: errorText(error), watts: null };
    console.warn('[Overview] Package power unavailable:', errorText(error));
  }
  if (lastOverview) {
    renderPower(lastOverview.power || {}, packageWatts);
  }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

async function readSerial() {
  const response = await invoke('get_device_serial');
  const serial = unwrap(response);
  if (!serial) {
    throw new Error('The firmware did not report a serial number');
  }
  setText('overview-serial', serial);
  return serial;
}

async function copyText(text) {
  if (navigator.clipboard && navigator.clipboard.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  // WebKit does not always grant the async clipboard API; the selection dance
  // still works and is invisible.
  const scratch = document.createElement('textarea');
  scratch.value = text;
  scratch.setAttribute('readonly', 'readonly');
  scratch.style.position = 'fixed';
  scratch.style.opacity = '0';
  document.body.append(scratch);
  scratch.select();
  const copied = document.execCommand('copy');
  scratch.remove();
  if (!copied) {
    throw new Error('The clipboard rejected the copy');
  }
}

async function copySerial(event) {
  const button = event?.currentTarget;
  try {
    // runWithState reports both outcomes, so nothing is toasted here; the
    // rethrow only stops the handler.
    await runWithState(
      button,
      async () => {
        const shown = document.getElementById('overview-serial')?.textContent || '';
        const serial = shown && shown !== 'Not read yet' ? shown : await readSerial();
        await copyText(serial);
      },
      { success: 'Serial number copied', title: 'Could not copy the serial number' }
    );
  } catch {
    /* already reported */
  }
}

async function onProfileClick(event) {
  const button = event.target.closest('.overview-profile');
  if (!button || button.classList.contains('is-active')) {
    return;
  }
  const profile = button.dataset.profile;
  try {
    await runWithState(
      button,
      async () => {
        unwrap(await invoke('set_power_profile', { profile }));
        profileValue = profile;
        document.querySelectorAll('.overview-profile').forEach((element) => {
          const active = element === button;
          element.classList.toggle('is-active', active);
          element.setAttribute('aria-pressed', active ? 'true' : 'false');
        });
      },
      {
        success: 'Power profile set to ' + profile.replace(/-/g, ' '),
        title: 'Could not change the power profile'
      }
    );
  } catch {
    /* already reported */
  }
}

export function setupHomeActions() {
  const copy = document.getElementById('btn-copy-serial');
  if (copy && !copy.dataset.bound) {
    copy.dataset.bound = 'true';
    copy.addEventListener('click', copySerial);
  }
  const profiles = document.getElementById('overview-profiles');
  if (profiles && !profiles.dataset.bound) {
    profiles.dataset.bound = 'true';
    profiles.addEventListener('click', onProfileClick);
  }
}

export async function updateHomeView() {
  setupHomeActions();
  await refreshOverview();
  if (powerTimer === null) {
    powerTimer = setInterval(() => {
      void loadPackagePower();
    }, PACKAGE_POWER_MS);
    void loadPackagePower();
  }
}
