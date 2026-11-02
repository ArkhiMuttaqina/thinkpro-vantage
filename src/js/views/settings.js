// Settings & Backup View
import { showStatus } from '../utils.js';
import { applyAllSettings } from '../settingsManager.js';

const invoke = (...args) => window.__TAURI__?.core?.invoke(...args);

let statusTimeout = null;
let gnomeStatusTimeout = null;
let currentGnomeStatus = null;

export function setupSettingsHandlers() {
  const btnExport = document.getElementById('btn-export-settings');
  const btnImport = document.getElementById('btn-import-settings');
  const btnReset = document.getElementById('btn-reset-settings');
  const fileInput = document.getElementById('input-import-settings');

  if (btnExport) {
    btnExport.addEventListener('click', handleExportSettings);
  }

  if (btnImport && fileInput) {
    btnImport.addEventListener('click', () => {
      fileInput.click();
    });

    fileInput.addEventListener('change', handleImportFileSelected);
  }

  if (btnReset) {
    btnReset.addEventListener('click', handleResetSettings);
  }

  setupGnomeExtensionHandlers();
}

function setupGnomeExtensionHandlers() {
  const btnToggle = document.getElementById('btn-toggle-gnome-ext');
  const btnReinstall = document.getElementById('btn-reinstall-gnome-ext');
  const btnUninstall = document.getElementById('btn-uninstall-gnome-ext');

  if (btnToggle) {
    btnToggle.addEventListener('click', handleToggleGnomeExtension);
  }

  if (btnReinstall) {
    btnReinstall.addEventListener('click', handleReinstallGnomeExtension);
  }

  if (btnUninstall) {
    btnUninstall.addEventListener('click', handleUninstallGnomeExtension);
  }
}

/**
 * Load settings view data and display current configurations
 */
export async function loadSettingsView() {
  try {
    const settings = await invoke('load_app_settings');
    updateSettingsDisplay(settings);
  } catch (error) {
    console.error('[Settings View] Failed to load settings:', error);
  }

  await loadGnomeExtensionStatus();
}

export async function loadGnomeExtensionStatus() {
  try {
    const status = await invoke('get_gnome_extension_status');
    currentGnomeStatus = status;
    updateGnomeExtensionDisplay(status);
  } catch (error) {
    console.error('[Settings View] Failed to load GNOME extension status:', error);
  }
}

function updateGnomeExtensionDisplay(status) {
  if (!status) {
    return;
  }

  const badgeEl = document.getElementById('gnome-ext-badge');
  const desktopEl = document.getElementById('gnome-ext-desktop');
  const statusTextEl = document.getElementById('gnome-ext-status-text');
  const pathEl = document.getElementById('gnome-ext-path');
  const restartNoticeEl = document.getElementById('gnome-ext-restart-notice');
  const btnToggleTextEl = document.getElementById('btn-toggle-gnome-ext-text');
  const btnToggleEl = document.getElementById('btn-toggle-gnome-ext');
  const cfgGnomeEl = document.getElementById('cfg-gnome-extension');

  // Desktop environment label
  if (desktopEl) {
    if (status.is_gnome) {
      desktopEl.textContent = status.shell_version
        ? `GNOME Shell ${status.shell_version}`
        : 'GNOME Shell';
    } else {
      desktopEl.textContent = 'Non-GNOME Desktop';
    }
  }

  // Installation Path
  if (pathEl && status.install_path) {
    pathEl.textContent = status.install_path;
  }

  // Status Badge and text
  if (badgeEl && statusTextEl) {
    badgeEl.className = 'extension-badge';

    if (!status.is_gnome) {
      badgeEl.classList.add('badge-neutral');
      badgeEl.textContent = 'Unsupported Desktop';
      statusTextEl.textContent = 'GNOME Desktop not detected';
    } else if (status.is_active) {
      badgeEl.classList.add('badge-success');
      badgeEl.textContent = 'Active';
      statusTextEl.textContent = 'Active in GNOME top panel';
    } else if (status.needs_restart) {
      badgeEl.classList.add('badge-warning');
      badgeEl.textContent = 'Restart Needed';
      statusTextEl.textContent = 'Enabled (Re-login required on Wayland)';
    } else if (status.is_enabled) {
      badgeEl.classList.add('badge-success');
      badgeEl.textContent = 'Enabled';
      statusTextEl.textContent = 'Enabled in GNOME Shell';
    } else if (status.is_installed) {
      badgeEl.classList.add('badge-neutral');
      badgeEl.textContent = 'Disabled';
      statusTextEl.textContent = 'Installed (Disabled)';
    } else {
      badgeEl.classList.add('badge-neutral');
      badgeEl.textContent = 'Not Installed';
      statusTextEl.textContent = 'Not installed';
    }
  }

  // Session restart reminder banner
  if (restartNoticeEl) {
    restartNoticeEl.style.display = status.needs_restart ? 'flex' : 'none';
  }

  // Toggle button state
  if (btnToggleTextEl && btnToggleEl) {
    if (status.is_enabled) {
      btnToggleTextEl.textContent = 'Disable Indicator';
      btnToggleEl.className = 'btn-secondary';
    } else {
      btnToggleTextEl.textContent = status.is_installed ? 'Enable Indicator' : 'Install & Enable';
      btnToggleEl.className = 'btn-primary';
    }
  }

  // Active configuration overview item
  if (cfgGnomeEl) {
    if (status.is_active) {
      cfgGnomeEl.textContent = 'Active';
    } else if (status.is_enabled) {
      cfgGnomeEl.textContent = 'Enabled';
    } else if (status.is_installed) {
      cfgGnomeEl.textContent = 'Disabled';
    } else {
      cfgGnomeEl.textContent = 'Not Installed';
    }
  }
}

async function handleToggleGnomeExtension() {
  const shouldEnable = !currentGnomeStatus || !currentGnomeStatus.is_enabled;
  try {
    showGnomeExtBanner(
      shouldEnable ? 'Enabling GNOME indicator...' : 'Disabling GNOME indicator...',
      'info'
    );
    const updated = await invoke('enable_gnome_extension', { enable: shouldEnable });
    currentGnomeStatus = updated;
    updateGnomeExtensionDisplay(updated);

    if (shouldEnable) {
      const msg = updated.needs_restart
        ? '✓ Extension enabled! Please log out and back in to view it in the top bar.'
        : '✓ Extension enabled successfully!';
      showGnomeExtBanner(msg, 'success');
      showStatus('✓ Top panel indicator enabled', 'success');
    } else {
      showGnomeExtBanner('✓ Extension disabled', 'info');
      showStatus('Top panel indicator disabled', 'info');
    }
  } catch (error) {
    console.error('[Settings] Toggle GNOME extension failed:', error);
    showGnomeExtBanner(`Failed to change extension state: ${error}`, 'error');
    showStatus('Operation failed', 'error');
  }
}

async function handleReinstallGnomeExtension() {
  try {
    showGnomeExtBanner('Reinstalling extension files...', 'info');
    const updated = await invoke('install_gnome_extension');
    currentGnomeStatus = updated;
    updateGnomeExtensionDisplay(updated);

    showGnomeExtBanner('✓ Extension files reinstalled and updated successfully!', 'success');
    showStatus('✓ Extension files updated', 'success');
  } catch (error) {
    console.error('[Settings] Reinstall GNOME extension failed:', error);
    showGnomeExtBanner(`Failed to reinstall extension: ${error}`, 'error');
    showStatus('Reinstall failed', 'error');
  }
}

async function handleUninstallGnomeExtension() {
  const confirmed = confirm(
    'Are you sure you want to uninstall the ThinkUtils GNOME top bar indicator extension?\n\nThis will remove the extension files from your user directory.'
  );

  if (!confirmed) {
    return;
  }

  try {
    showGnomeExtBanner('Uninstalling extension...', 'info');
    const updated = await invoke('uninstall_gnome_extension');
    currentGnomeStatus = updated;
    updateGnomeExtensionDisplay(updated);

    showGnomeExtBanner('✓ Extension uninstalled from user directory', 'success');
    showStatus('Extension uninstalled', 'info');
  } catch (error) {
    console.error('[Settings] Uninstall GNOME extension failed:', error);
    showGnomeExtBanner(`Failed to uninstall extension: ${error}`, 'error');
    showStatus('Uninstall failed', 'error');
  }
}

function showGnomeExtBanner(message, type = 'info') {
  const banner = document.getElementById('gnome-ext-action-message');
  if (!banner) {
    return;
  }

  if (gnomeStatusTimeout) {
    clearTimeout(gnomeStatusTimeout);
  }

  banner.textContent = message;
  banner.className = `settings-status-banner ${type}`;
  banner.style.display = 'block';

  if (type === 'success' || type === 'info') {
    gnomeStatusTimeout = setTimeout(() => {
      banner.style.display = 'none';
    }, 6000);
  }
}

async function handleExportSettings() {
  try {
    showSettingsBanner('Preparing settings export...', 'info');
    const jsonString = await invoke('export_settings');

    // Trigger browser/webview file download
    const blob = new Blob([jsonString], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    const dateStr = new Date().toISOString().slice(0, 10);
    a.href = url;
    a.download = `thinkutils-settings-${dateStr}.json`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);

    // Refresh display to show the synchronized exported settings
    const activeSettings = await invoke('load_app_settings');
    updateSettingsDisplay(activeSettings);

    showSettingsBanner('✓ Settings exported successfully to JSON file', 'success');
    showStatus('✓ Settings exported', 'success');
  } catch (error) {
    console.error('[Settings] Export failed:', error);
    showSettingsBanner(`Failed to export settings: ${error}`, 'error');
    showStatus('Export failed', 'error');
  }
}

async function handleImportFileSelected(event) {
  const file = event.target.files && event.target.files[0];
  if (!file) {
    return;
  }

  try {
    showSettingsBanner('Reading backup file...', 'info');
    const text = await file.text();

    const imported = await invoke('import_settings', { jsonContent: text });
    console.log('[Settings] Imported settings:', imported);

    // Apply the imported settings across the system, hardware, and active views
    await applyAllSettings(imported, true);

    // Refresh UI display
    updateSettingsDisplay(imported);

    showSettingsBanner('✓ Settings successfully imported and applied to hardware!', 'success');
    showStatus('✓ Settings imported & applied', 'success');
  } catch (error) {
    console.error('[Settings] Import failed:', error);
    showSettingsBanner(`Import failed: ${error}`, 'error');
    showStatus('Import failed', 'error');
  } finally {
    // Reset file input so re-selecting the same file fires change event
    event.target.value = '';
  }
}

async function handleResetSettings() {
  const confirmed = confirm(
    'Are you sure you want to reset all settings to default values?\n\nThis will restore factory defaults for fan control, battery thresholds, and performance profiles.'
  );

  if (!confirmed) {
    return;
  }

  try {
    showSettingsBanner('Resetting settings to defaults...', 'info');
    const defaults = await invoke('reset_settings');
    await applyAllSettings(defaults, true);
    updateSettingsDisplay(defaults);

    showSettingsBanner('✓ Settings have been reset to factory defaults', 'success');
    showStatus('✓ Reset to defaults', 'success');
  } catch (error) {
    console.error('[Settings] Reset failed:', error);
    showSettingsBanner(`Failed to reset settings: ${error}`, 'error');
    showStatus('Reset failed', 'error');
  }
}

function updateSettingsDisplay(settings) {
  if (!settings) {
    return;
  }

  setText('cfg-fan-mode', settings.fan_mode);
  setText('cfg-fan-level', settings.fan_level);
  setText(
    'cfg-fan-curve',
    settings.fan_curve_enabled
      ? `Enabled (${settings.fan_curve_points ? settings.fan_curve_points.length : 0} pts)`
      : 'Disabled'
  );

  setText('cfg-battery-start', `${settings.battery_start_threshold}%`);
  setText('cfg-battery-stop', `${settings.battery_stop_threshold}%`);

  setText('cfg-cpu-governor', settings.cpu_governor || 'default');
  setText('cfg-turbo-boost', settings.turbo_boost_enabled ? 'Enabled' : 'Disabled');
  setText('cfg-power-profile', settings.power_profile || 'default');

  setText('cfg-theme', settings.theme || 'system');
  setText('cfg-autostart', settings.auto_start ? 'Enabled' : 'Disabled');
  setText('cfg-minimize-tray', settings.minimize_to_tray ? 'Enabled' : 'Disabled');
}

function setText(id, value) {
  const el = document.getElementById(id);
  if (el) {
    el.textContent = value !== undefined && value !== null ? String(value) : '--';
  }
}

function showSettingsBanner(message, type = 'info') {
  const banner = document.getElementById('settings-status-message');
  if (!banner) {
    return;
  }

  if (statusTimeout) {
    clearTimeout(statusTimeout);
  }

  banner.textContent = message;
  banner.className = `settings-status-banner ${type}`;
  banner.style.display = 'block';

  // Auto-hide success or info banners after 6 seconds
  if (type === 'success' || type === 'info') {
    statusTimeout = setTimeout(() => {
      banner.style.display = 'none';
    }, 6000);
  }
}
