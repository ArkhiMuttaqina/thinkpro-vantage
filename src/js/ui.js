// Shared UI utilities: version stamp, toasts, busy states.
//
// Every backend call that the user can see must be able to say "working" and
// "failed". Before this, a click either produced a result or nothing at all.
const app = window.__TAURI__?.app;
const core = window.__TAURI__?.core;

/** Print the app version under the rail title. */
export async function mountAppVersion() {
  const el = document.getElementById('app-version');
  if (!el) {
    return;
  }
  try {
    const version = await app?.getVersion?.();
    if (version) {
      el.textContent = `v${version}`;
    }
  } catch (error) {
    // Version is decoration; a missing API must not break the shell.
    el.textContent = 'dev';
    console.warn('[UI] getVersion unavailable:', error);
  }
}

function stack() {
  let el = document.querySelector('.toast-stack');
  if (!el) {
    el = document.createElement('div');
    el.className = 'toast-stack';
    document.body.appendChild(el);
  }
  return el;
}

/**
 * Show a transient message.
 * @param {string} title
 * @param {{detail?: string, kind?: 'ok'|'warn'|'danger'|'info', timeout?: number}} [options]
 */
export function toast(title, options = {}) {
  const { detail = '', kind = 'info', timeout = 4200 } = options;
  const el = document.createElement('div');
  el.className = `toast is-${kind}`;
  const body = document.createElement('div');
  body.className = 'toast-body';
  const head = document.createElement('div');
  head.className = 'toast-title';
  head.textContent = title;
  body.appendChild(head);
  if (detail) {
    const sub = document.createElement('div');
    sub.className = 'toast-detail';
    sub.textContent = detail;
    body.appendChild(sub);
  }
  el.appendChild(body);
  stack().appendChild(el);
  setTimeout(() => el.remove(), timeout);
  return el;
}

/** Pull the human-readable message out of whatever a command rejected with. */
export function errorText(error) {
  if (!error) {
    return 'Unknown error';
  }
  if (typeof error === 'string') {
    return error;
  }
  return error.message || error.error || JSON.stringify(error);
}

/**
 * Run a backend call with visible state.
 *
 * The element that triggered the call is disabled and marked busy while the
 * call is in flight, and any failure is surfaced as a toast instead of ending
 * up only in the console.
 *
 * @param {HTMLElement|null} trigger
 * @param {() => Promise<any>} work
 * @param {{title?: string, success?: string, spinner?: boolean}} [options]
 */
export async function runWithState(trigger, work, options = {}) {
  const { success = '', title = 'Action failed', spinner = true } = options;
  const host =
    trigger?.closest('.card, .tile, .hero, .settings-group, section') || trigger?.parentElement;
  let spinnerEl = null;

  if (trigger) {
    trigger.disabled = true;
    trigger.classList.add('is-busy');
    if (spinner) {
      spinnerEl = document.createElement('span');
      spinnerEl.className = 'spinner';
      trigger.prepend(spinnerEl);
    }
  }
  if (host) {
    host.setAttribute('aria-busy', 'true');
  }

  try {
    const result = await work();
    if (success) {
      toast(success, { kind: 'ok' });
    }
    return result;
  } catch (error) {
    toast(title, { detail: errorText(error), kind: 'danger' });
    throw error;
  } finally {
    if (trigger) {
      trigger.disabled = false;
      trigger.classList.remove('is-busy');
    }
    spinnerEl?.remove();
    if (host) {
      host.removeAttribute('aria-busy');
    }
  }
}

/** Global Tauri invoke, guarded so a missing bridge is loud but not fatal. */
export function invoke(command, args) {
  if (!core?.invoke) {
    return Promise.reject(new Error(`Tauri bridge unavailable (cannot run ${command})`));
  }
  return core.invoke(command, args);
}
