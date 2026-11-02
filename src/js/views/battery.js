// Battery View
const { invoke } = window.__TAURI__.core;
import { elements } from '../dom.js';
import { showStatus, escapeHtml } from '../utils.js';
import { setState, getState } from '../state.js';

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
}

export async function loadBatteryInfo() {
  try {
    let isAcOnline = false;

    // Fetch power consumption and runtime metrics
    try {
      const powerResponse = await invoke('get_power_consumption');
      if (powerResponse.success && powerResponse.data) {
        isAcOnline = powerResponse.data.is_ac_online;
        displayPowerConsumption(powerResponse.data);
      }
    } catch (err) {
      console.error('[Battery] Power consumption query failed:', err);
    }

    // Fetch battery hardware details
    const response = await invoke('get_battery_info');
    if (response.success && response.data) {
      displayBatteries(response.data, isAcOnline);
    }

    const thresholdResponse = await invoke('get_battery_thresholds');
    if (thresholdResponse.success && thresholdResponse.data) {
      if (elements.thresholdStart && document.activeElement !== elements.thresholdStart) {
        elements.thresholdStart.value = thresholdResponse.data.start;
      }
      if (elements.thresholdStop && document.activeElement !== elements.thresholdStop) {
        elements.thresholdStop.value = thresholdResponse.data.stop;
      }
      if (elements.thresholdStartValue && document.activeElement !== elements.thresholdStart) {
        elements.thresholdStartValue.textContent = thresholdResponse.data.start + '%';
      }
      if (elements.thresholdStopValue && document.activeElement !== elements.thresholdStop) {
        elements.thresholdStopValue.textContent = thresholdResponse.data.stop + '%';
      }
    }
  } catch (error) {
    console.error('[Battery] Load failed:', error);
  }
}

function formatHours(hours) {
  if (!hours || isNaN(hours) || hours <= 0) {
    return '-- h';
  }
  const totalMinutes = Math.round(hours * 60);
  const h = Math.floor(totalMinutes / 60);
  const m = totalMinutes % 60;
  if (h === 0) {
    return `${m}m`;
  }
  if (m === 0) {
    return `${h}h`;
  }
  return `${h}h ${m}m`;
}

function displayPowerConsumption(data) {
  const drawEl = document.getElementById('power-draw');
  const runtimeEl = document.getElementById('runtime-estimate');

  if (drawEl) {
    if (data.is_charging) {
      drawEl.textContent = `+${data.power_w.toFixed(1)} W (Charging)`;
      drawEl.title = `Charging at +${data.power_w.toFixed(1)} W`;
    } else if (data.status === 'Discharging') {
      drawEl.textContent = `${data.power_w.toFixed(1)} W (Discharging)`;
      drawEl.title = `Discharging at ${data.power_w.toFixed(1)} W`;
    } else if (data.is_ac_online) {
      drawEl.textContent =
        data.power_w > 0 ? `${data.power_w.toFixed(1)} W` : '0.0 W (On AC Power)';
      drawEl.title =
        data.power_w > 0
          ? `${data.power_w.toFixed(1)} W`
          : 'Battery is idle; laptop is powered directly by AC';
    } else {
      drawEl.textContent = `${data.power_w.toFixed(1)} W`;
      drawEl.title = '';
    }
  }

  if (runtimeEl) {
    if (data.is_ac_online) {
      if (data.is_charging && data.time_to_full_hours && data.time_to_full_hours > 0) {
        runtimeEl.textContent = `~${formatHours(data.time_to_full_hours)} to full`;
      } else {
        runtimeEl.textContent = 'AC Connected (Unlimited)';
      }
    } else if (data.status === 'Discharging') {
      if (data.runtime_hours && data.runtime_hours > 0) {
        runtimeEl.textContent = `~${formatHours(data.runtime_hours)} remaining`;
      } else {
        runtimeEl.textContent = 'Calculating...';
      }
    } else if (data.status === 'Full') {
      runtimeEl.textContent = 'Battery Full';
    } else {
      runtimeEl.textContent = '-- h';
    }
  }
}

function displayBatteries(batteries, isAcOnline = false) {
  const container = document.getElementById('battery-cards');
  if (!container) {
    return;
  }
  container.innerHTML = '';

  batteries.forEach((battery) => {
    const card = document.createElement('div');
    card.className = 'battery-card';

    // Format cycles: display count if > 0, otherwise N/A with title explaining firmware limitation
    const cyclesText = battery.cycles && battery.cycles > 0 ? String(battery.cycles) : 'N/A';
    const cyclesTitle =
      battery.cycles > 0
        ? `${battery.cycles} charge cycles completed`
        : 'Cycle count is not reported by battery hardware';

    // Format power: clarify whether power draw is discharging, charging, or AC connected
    let powerText;
    let powerTitle;
    if (battery.status === 'Charging') {
      powerText = `+${battery.power.toFixed(1)}W`;
      powerTitle = `Charging battery at +${battery.power.toFixed(1)}W`;
    } else if (battery.status === 'Discharging') {
      powerText = `${battery.power.toFixed(1)}W`;
      powerTitle = `Discharging battery at ${battery.power.toFixed(1)}W`;
    } else if (isAcOnline || battery.status === 'Not charging' || battery.status === 'Full') {
      powerText = battery.power > 0 ? `${battery.power.toFixed(1)}W` : '0.0W (AC)';
      powerTitle =
        battery.power > 0
          ? `${battery.power.toFixed(1)}W`
          : 'Battery is idle; laptop is powered by AC';
    } else {
      powerText = `${battery.power.toFixed(1)}W`;
      powerTitle = '';
    }

    card.innerHTML = `
      <div class="battery-header">
        <span class="battery-name">${escapeHtml(battery.name)}</span>
        <span class="battery-status">${escapeHtml(battery.status)}</span>
      </div>
      <div class="battery-capacity">${battery.capacity}%</div>
      <div class="battery-details">
        <div class="battery-detail">
          <span class="battery-detail-label">Health</span>
          <span class="battery-detail-value">${battery.health}%</span>
        </div>
        <div class="battery-detail">
          <span class="battery-detail-label">Cycles</span>
          <span class="battery-detail-value" title="${escapeHtml(cyclesTitle)}">${escapeHtml(cyclesText)}</span>
        </div>
        <div class="battery-detail">
          <span class="battery-detail-label">Power</span>
          <span class="battery-detail-value" title="${escapeHtml(powerTitle)}">${escapeHtml(powerText)}</span>
        </div>
        <div class="battery-detail">
          <span class="battery-detail-label">Technology</span>
          <span class="battery-detail-value">${escapeHtml(battery.technology)}</span>
        </div>
      </div>
    `;
    container.appendChild(card);
  });
}

async function applyBatteryThresholds() {
  const start = parseInt(elements.thresholdStart.value);
  const stop = parseInt(elements.thresholdStop.value);

  try {
    showStatus('Setting battery thresholds...', 'info');
    const response = await invoke('set_battery_thresholds', { start, stop });

    if (response.success) {
      showStatus(`✓ Thresholds set: ${start}%-${stop}%`, 'success');
      try {
        const { updateSetting } = await import('../settingsManager.js');
        await updateSetting('battery_start_threshold', start);
        await updateSetting('battery_stop_threshold', stop);
      } catch (err) {
        console.error('[Battery] Failed to update settings store:', err);
      }
    } else {
      showStatus(`Error: ${response.error}`, 'error');
    }
  } catch (error) {
    showStatus(`Error: ${error}`, 'error');
  }
}
