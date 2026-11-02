// DOM Element References
export const elements = {};

export function initializeElements() {
  // Page header
  elements.pageTitle = document.getElementById('page-title');
  elements.pageSubtitle = document.getElementById('page-subtitle');

  // Fan control
  elements.tempMetrics = document.getElementById('temp-metrics');
  elements.fanMetrics = document.getElementById('fan-metrics');
  elements.slider = document.getElementById('fan-slider');
  elements.sliderValue = document.getElementById('slider-value');
  elements.sliderSection = document.getElementById('slider-section');
  elements.btnAuto = document.getElementById('btn-auto');
  elements.btnManual = document.getElementById('btn-manual');
  elements.btnCurve = document.getElementById('btn-curve');
  elements.btnFull = document.getElementById('btn-full');
  elements.currentMode = document.getElementById('current-mode');
  elements.statusMessage = document.getElementById('status-message');
  elements.fanStatusBanner = document.querySelector('.fan-status-banner');
  elements.curveSection = document.getElementById('curve-section');
  elements.aboutLink = document.getElementById('about-link');
  elements.aboutDialog = document.getElementById('about-dialog');
  elements.closeAbout = document.getElementById('close-about');
  elements.permissionHelper = document.getElementById('permission-helper');
  elements.btnGrantPermissions = document.getElementById('btn-grant-permissions');

  // Views
  elements.homeView = document.getElementById('home-view');
  elements.fanView = document.getElementById('fan-view');
  elements.settingsView = document.getElementById('settings-view');
  elements.systemView = document.getElementById('system-view');
  elements.batteryView = document.getElementById('battery-view');
  elements.performanceView = document.getElementById('performance-view');
  elements.monitorView = document.getElementById('monitor-view');
  elements.securityView = document.getElementById('security-view');
  elements.mcpView = document.getElementById('mcp-view');

  // Settings
  elements.btnExportSettings = document.getElementById('btn-export-settings');
  elements.btnImportSettings = document.getElementById('btn-import-settings');
  elements.btnResetSettings = document.getElementById('btn-reset-settings');
  elements.inputImportSettings = document.getElementById('input-import-settings');

  // Battery
  elements.thresholdStart = document.getElementById('threshold-start');
  elements.thresholdStop = document.getElementById('threshold-stop');
  elements.thresholdStartValue = document.getElementById('threshold-start-value');
  elements.thresholdStopValue = document.getElementById('threshold-stop-value');
  elements.btnApplyThresholds = document.getElementById('btn-apply-thresholds');
}
