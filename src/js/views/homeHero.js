// Home hero: one status block that answers "what is this machine doing".
//
// Data comes from commands that already exist (get_power_consumption,
// get_battery_info, get_power_profile, get_fan_capability, get_cpu_info), so
// the hero adds no new backend surface. It refreshes on the same cadence as
// the rest of the Home view and only while Home is visible.
import { invoke, runWithState } from '../ui.js';

const MODES = [
  { id: 'power-saver', label: 'Power Saver' },
  { id: 'balanced', label: 'Balanced' },
  { id: 'performance', label: 'Performance' }
];

function el(id) {
  return document.getElementById(id);
}

function setText(id, value) {
  const node = el(id);
  if (node) {
    node.textContent = value;
  }
}

function formatWatt(powerW) {
  if (typeof powerW !== 'number' || Number.isNaN(powerW)) {
    return '--';
  }
  return `${powerW.toFixed(2)} W`;
}

function formatHours(hours) {
  if (typeof hours !== 'number' || !Number.isFinite(hours) || hours <= 0) {
    return '--';
  }
  const totalMinutes = Math.round(hours * 60);
  const h = Math.floor(totalMinutes / 60);
  const m = totalMinutes % 60;
  return h > 0 ? `${h} h ${m} min` : `${m} min`;
}

/** Render the mode pills once the available profiles are known. */
function renderModePills(current, available) {
  const host = el('hero-modes');
  if (!host) {
    return;
  }
  const known = new Set(available?.length ? available : MODES.map((m) => m.id));
  host.textContent = '';
  for (const mode of MODES) {
    if (!known.has(mode.id)) {
      continue;
    }
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'pill';
    button.dataset.mode = mode.id;
    button.textContent = mode.label;
    if (mode.id === current) {
      button.classList.add('is-active');
      button.setAttribute('aria-pressed', 'true');
    } else {
      button.setAttribute('aria-pressed', 'false');
    }
    host.appendChild(button);
  }
}

/** Wire the hero's own controls. Called once at startup. */
export function setupHeroActions() {
  const host = el('hero-modes');
  if (!host) {
    return;
  }
  host.addEventListener('click', async (event) => {
    const button = event.target.closest('button[data-mode]');
    if (!button || button.classList.contains('is-active')) {
      return;
    }
    const mode = button.dataset.mode;
    try {
      await runWithState(button, () => invoke('set_power_profile', { profile: mode }), {
        title: 'Gagal mengganti profil daya'
      });
      await updateHomeHero();
    } catch {
      // runWithState has already reported it.
    }
  });
}

/** Refresh hero values. Cheap enough to run alongside updateHomeView(). */
export async function updateHomeHero() {
  const hero = el('home-hero');
  if (!hero) {
    return;
  }

  const [power, profile, battery, cpu] = await Promise.allSettled([
    invoke('get_power_consumption'),
    invoke('get_power_profile'),
    invoke('get_battery_info'),
    invoke('get_cpu_info')
  ]);

  if (power.status === 'fulfilled' && power.value?.success) {
    const data = power.value.data;
    setText('hero-power', formatWatt(data.power_w));
    setText(
      'hero-runtime',
      data.is_ac_online ? 'On AC' : data.is_charging ? 'Charging' : formatHours(data.runtime_hours)
    );
  } else {
    setText('hero-power', '--');
    setText('hero-runtime', 'unavailable');
  }

  if (profile.status === 'fulfilled' && profile.value?.success) {
    const { current, available } = profile.value.data;
    const label = MODES.find((m) => m.id === current)?.label || current || 'Unknown';
    setText('hero-mode', label);
    renderModePills(current, available);
  } else {
    setText('hero-mode', 'Unknown');
  }

  if (battery.status === 'fulfilled' && battery.value?.success) {
    // get_battery_info returns one entry per battery slot (BAT0/BAT1); the hero
    // reports the primary pack, the same one the rest of the app leads with.
    const list = Array.isArray(battery.value.data) ? battery.value.data : [];
    const data = list[0];
    if (data) {
      setText('hero-battery', `${data.capacity ?? '--'}%`);
      // `health` is a whole percent from the backend; the energy ratio is the
      // fallback for kernels that do not report it.
      const health =
        data.health ??
        (data.energy_full && data.energy_design
          ? Math.round((data.energy_full / data.energy_design) * 1000) / 10
          : null);
      setText('hero-battery-meta', health ? `health ${health}%` : data.status || '--');
    }
  }

  if (cpu.status === 'fulfilled' && cpu.value?.success) {
    const data = cpu.value.data;
    // Only fields get_cpu_info actually returns. EPP has no backend command
    // yet, so it is not displayed rather than shown as a permanent "--".
    setText('hero-cpu', data.governor || '--');
    // get_cpu_info reports frequencies in kHz.
    setText('hero-cpu-meta', data.max_freq ? `max ${(data.max_freq / 1e6).toFixed(2)} GHz` : '');
  }
}
