// Template Loader - Load and inject HTML templates
const TEMPLATES = {
  titlebar: '/templates/titlebar.html',
  sidebar: '/templates/sidebar.html',
  dialogs: '/templates/dialogs.html',
  // Views
  homeView: '/templates/views/home.html',
  fanView: '/templates/views/fan.html',
  batteryView: '/templates/views/battery.html',
  performanceView: '/templates/views/performance.html',
  monitorView: '/templates/views/monitor.html',
  systemView: '/templates/views/system.html',
  settingsView: '/templates/views/settings.html',
  securityView: '/templates/views/security.html',
  mcpView: '/templates/views/mcp.html'
};

/**
 * Load all HTML templates
 * @returns {Promise<Object>} Object containing all loaded templates
 */
export async function loadTemplates() {
  console.log('[TemplateLoader] Loading templates...');

  const templates = {};
  const entries = Object.entries(TEMPLATES);

  const results = await Promise.allSettled(
    entries.map(async ([name, path]) => {
      const response = await fetch(path);
      if (!response.ok) {
        throw new Error(`Failed to load template: ${path} (HTTP ${response.status})`);
      }
      const html = await response.text();
      return { name, html };
    })
  );

  for (const res of results) {
    if (res.status === 'fulfilled') {
      templates[res.value.name] = res.value.html;
      console.log(`[TemplateLoader] ✓ Loaded ${res.value.name}`);
    } else {
      console.error('[TemplateLoader] ✗ Error loading template:', res.reason);
    }
  }

  console.log(
    `[TemplateLoader] ${Object.keys(templates).length}/${entries.length} templates loaded`
  );
  return templates;
}

/**
 * Inject templates into the DOM
 * @param {Object} templates - Object containing template HTML strings
 */
export function injectTemplates(templates) {
  console.log('[TemplateLoader] Injecting templates...');

  // Inject titlebar
  if (templates.titlebar) {
    const titlebarContainer = document.getElementById('titlebar-container');
    if (titlebarContainer) {
      titlebarContainer.innerHTML = templates.titlebar;
      console.log('[TemplateLoader] ✓ Injected titlebar');
    }
  }

  // Inject sidebar
  if (templates.sidebar) {
    const sidebarContainer = document.getElementById('sidebar-container');
    if (sidebarContainer) {
      sidebarContainer.innerHTML = templates.sidebar;
      console.log('[TemplateLoader] ✓ Injected sidebar');
    }
  }

  // Inject dialogs
  if (templates.dialogs) {
    const dialogsContainer = document.getElementById('dialogs-container');
    if (dialogsContainer) {
      dialogsContainer.innerHTML = templates.dialogs;
      console.log('[TemplateLoader] ✓ Injected dialogs');
    }
  }

  // Inject views
  const viewsContainer = document.getElementById('views-container');
  if (viewsContainer) {
    viewsContainer.innerHTML = '';
    const viewTemplates = [
      { key: 'homeView', id: 'home-view' },
      { key: 'fanView', id: 'fan-view' },
      { key: 'batteryView', id: 'battery-view' },
      { key: 'performanceView', id: 'performance-view' },
      { key: 'monitorView', id: 'monitor-view' },
      { key: 'systemView', id: 'system-view' },
      { key: 'settingsView', id: 'settings-view' },
      { key: 'securityView', id: 'security-view' },
      { key: 'mcpView', id: 'mcp-view' }
    ];

    viewTemplates.forEach(({ key, id }) => {
      if (templates[key]) {
        const viewDiv = document.createElement('div');
        viewDiv.className = 'content-view';
        viewDiv.id = id;
        viewDiv.style.display = id === 'home-view' ? 'block' : 'none';
        viewDiv.innerHTML = templates[key];
        viewsContainer.appendChild(viewDiv);
      }
    });
    console.log('[TemplateLoader] ✓ Injected views');
  }

  console.log('[TemplateLoader] All templates injected');
}

/**
 * Check if the app is using modular templates
 * @returns {boolean}
 */
export function isModularMode() {
  return document.getElementById('titlebar-container') !== null;
}
