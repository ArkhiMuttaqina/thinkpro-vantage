// Titlebar Window Controls
const { invoke } = window.__TAURI__.core;
import { setState, getState } from './state.js';

export function setupTitlebar() {
  console.log('[Titlebar] Setting up window controls');

  const titlebar = document.querySelector('.titlebar');
  const minimizeBtn = document.getElementById('minimize-btn');
  const maximizeBtn = document.getElementById('maximize-btn');
  const closeBtn = document.getElementById('close-btn');

  // Load and display machine model in titlebar
  loadMachineModel();

  if (!minimizeBtn || !maximizeBtn || !closeBtn) {
    console.error('[Titlebar] Buttons not found');
    return;
  }

  if (titlebar) {
    titlebar.addEventListener('mousedown', async (e) => {
      if (e.target.closest('.titlebar-button')) {
        return;
      }
      try {
        await invoke('start_drag');
      } catch (e) {
        console.error('[Titlebar] Drag failed:', e);
      }
    });

    titlebar.addEventListener('dblclick', async (e) => {
      if (e.target.closest('.titlebar-button')) {
        return;
      }
      try {
        await invoke('toggle_maximize');
      } catch (e) {
        console.error('[Titlebar] Double-click maximize failed:', e);
      }
    });
  }

  minimizeBtn.addEventListener('click', async () => {
    try {
      await invoke('minimize_window');
    } catch (e) {
      console.error('[Titlebar] Minimize failed:', e);
    }
  });

  maximizeBtn.addEventListener('click', async () => {
    try {
      await invoke('toggle_maximize');
    } catch (e) {
      console.error('[Titlebar] Maximize failed:', e);
    }
  });

  closeBtn.addEventListener('click', async () => {
    try {
      await invoke('close_window');
    } catch (e) {
      console.error('[Titlebar] Close failed:', e);
    }
  });

  // Start live hardware monitoring in the titlebar
  startTitlebarStats();
}

export function startTitlebarStats() {
  stopTitlebarStats();
  updateTitlebarStats();
  const interval = setInterval(updateTitlebarStats, 2000);
  setState('titlebarInterval', interval);
}

export function stopTitlebarStats() {
  const interval = getState('titlebarInterval');
  if (interval) {
    clearInterval(interval);
    setState('titlebarInterval', null);
  }
}

async function updateTitlebarStats() {
  try {
    const tempEl = document.getElementById('titlebar-temp');
    const tempStat = document.getElementById('titlebar-temp-stat');
    const fanEl = document.getElementById('titlebar-fan');
    const batteryEl = document.getElementById('titlebar-battery');

    // 1. Fetch sensor data (temps & fans)
    const sensorResponse = await invoke('get_sensor_data');
    if (sensorResponse.success && sensorResponse.data) {
      const { temps, fans } = sensorResponse.data;

      // Extract CPU temperature
      let cpuTemp = null;
      for (const [key, val] of Object.entries(temps)) {
        const k = key.toLowerCase();
        if (
          k.includes('cpu') ||
          k.includes('package') ||
          k.includes('tctl') ||
          k.includes('core')
        ) {
          const num = parseInt(val);
          if (!isNaN(num) && (cpuTemp === null || num > cpuTemp)) {
            cpuTemp = num;
          }
        }
      }
      if (cpuTemp === null && Object.keys(temps).length > 0) {
        for (const val of Object.values(temps)) {
          const num = parseInt(val);
          if (!isNaN(num) && num > 0 && num < 125) {
            cpuTemp = num;
            break;
          }
        }
      }

      if (tempEl && cpuTemp !== null) {
        tempEl.textContent = `${cpuTemp}°C`;
        if (tempStat) {
          tempStat.classList.remove('warning', 'danger');
          if (cpuTemp >= 85) {
            tempStat.classList.add('danger');
          } else if (cpuTemp >= 70) {
            tempStat.classList.add('warning');
          }
        }
      }

      // Extract fan RPM and mode
      let fanSpeed = null;
      for (const [key, val] of Object.entries(fans)) {
        if (key.toLowerCase().includes('fan')) {
          const num = parseInt(val, 10);
          if (!isNaN(num)) {
            fanSpeed = num;
            break;
          }
        }
      }
      if (fanEl && fanSpeed !== null) {
        let modeLabel = '';
        const level = (fans.level || '').toString().toLowerCase();
        if (level === 'auto') {
          modeLabel = ' (Auto)';
        } else if (level === 'full-speed' || level === 'disengaged') {
          modeLabel = ' (Max)';
        } else if (!isNaN(parseInt(level, 10))) {
          modeLabel = ` (L${level})`;
        }
        fanEl.textContent = `${fanSpeed} RPM${modeLabel}`;
      }
    }

    // 2. Fetch battery percentage
    if (batteryEl) {
      try {
        const powerResponse = await invoke('get_power_consumption');
        if (powerResponse.success && powerResponse.data) {
          const { capacity, is_ac_online } = powerResponse.data;
          batteryEl.textContent = is_ac_online ? `${capacity}% (AC)` : `${capacity}%`;
        }
      } catch (_) {
        // Battery info unavailable
      }
    }
  } catch (e) {
    console.error('[Titlebar] Stats update failed:', e);
  }
}

async function loadMachineModel() {
  try {
    const response = await invoke('get_system_info');
    if (response.success && response.data) {
      const titleElement = document.querySelector('.titlebar-title');
      if (titleElement) {
        titleElement.textContent = `ThinkUtils - ${response.data.model}`;
      }
    }
  } catch (e) {
    console.error('[Titlebar] Failed to load machine model:', e);
  }
}
