// Power: why a setting will or will not take effect.
//
// Two backends feed this page. `power_owners` is the arbiter's answer to "who
// owns each knob right now" -- claims and conflicts between TLP,
// power-profiles-daemon, lactd and the app itself. `power_doctor` is the
// diagnosis: one finding per thing that is wrong, each with the remedy that
// actually applies to it.
//
// The page never applies a system change on its own. A remedy is either an action
// the app may take (and the user presses for), or a command that belongs to the
// administrator, which is offered for copying.

import { invoke, runWithState, errorText, toast } from '../ui.js';

const STATE_LABELS = {
  ok: 'OK',
  notice: 'Notice',
  problem: 'Problem',
  blocking: 'Blocking'
};

const CAPABILITY_LABELS = {
  full_tlp: 'TLP in charge',
  limited_ppd: 'power-profiles-daemon only',
  limited_direct: 'direct control only'
};

function unwrap(response) {
  if (!response || response.success === false) {
    throw new Error(errorText(response && response.error));
  }
  return response.data;
}

async function copyText(text) {
  if (navigator.clipboard && navigator.clipboard.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
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

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) {
    node.className = className;
  }
  if (text !== undefined && text !== null) {
    node.textContent = text;
  }
  return node;
}

function renderSummary(diagnosis) {
  const headline = document.getElementById('power-headline');
  const text = document.getElementById('power-summary-text');
  const capability = document.getElementById('power-capability');
  if (headline) {
    headline.textContent =
      diagnosis.blocking > 0
        ? 'Power settings cannot be trusted on this machine yet'
        : diagnosis.problems > 0
          ? 'Some settings will not take effect'
          : 'Power management is healthy';
  }
  if (text) {
    text.textContent = diagnosis.summary || '';
  }
  if (capability) {
    capability.textContent = CAPABILITY_LABELS[diagnosis.capability] || diagnosis.capability || '—';
  }
  const panel = document.getElementById('power-summary');
  if (panel) {
    panel.setAttribute('aria-busy', 'false');
  }
}

function renderClaims(ownership) {
  const list = document.getElementById('power-claim-list');
  const panel = document.getElementById('power-claims-panel');
  if (!list) {
    return;
  }
  const claims = (ownership && ownership.claims) || [];
  list.replaceChildren(
    ...claims.map((claim) => {
      const item = element('li', 'power-claim');
      item.append(element('span', 'power-claim-label', claim.label));
      item.append(element('span', 'power-claim-owner', claim.owner));
      if (claim.detail) {
        item.append(element('span', 'power-claim-detail', claim.detail));
      }
      return item;
    })
  );
  if (!claims.length) {
    list.append(element('li', 'power-claim', 'No knob is claimed by more than one owner.'));
  }
  if (panel) {
    panel.setAttribute('aria-busy', 'false');
  }
}

function remedyButton(remedy, reload) {
  if (!remedy) {
    return null;
  }
  const button = element('button', 'power-remedy-btn', remedy.label);
  button.type = 'button';
  button.dataset.kind = remedy.kind;

  button.addEventListener('click', async () => {
    if (remedy.kind === 'action') {
      try {
        await runWithState(
          button,
          async () => {
            unwrap(await invoke(remedy.value));
            await reload();
          },
          { success: 'Done', title: 'That did not work' }
        );
      } catch {
        /* reported by runWithState */
      }
      return;
    }
    if (remedy.kind === 'command') {
      try {
        await copyText(remedy.value);
        toast('Command copied', { kind: 'ok', detail: remedy.value });
      } catch (error) {
        toast('Could not copy the command', { kind: 'danger', detail: errorText(error) });
      }
    }
  });

  return button;
}

function renderFindings(diagnosis, reload) {
  const list = document.getElementById('power-finding-list');
  if (!list) {
    return;
  }
  const findings = diagnosis.findings || [];

  list.replaceChildren(
    ...findings.map((finding) => {
      const item = element('li', 'power-finding');
      item.dataset.state = finding.state;

      const head = element('div', 'power-finding-head');
      head.append(
        element('span', 'power-state-chip', STATE_LABELS[finding.state] || finding.state)
      );
      head.append(element('span', 'power-finding-title', finding.title));
      item.append(head);

      if (finding.detail) {
        item.append(element('p', 'power-finding-detail', finding.detail));
      }

      if (finding.locations && finding.locations.length) {
        item.append(element('p', 'power-finding-location', finding.locations.join('\n')));
      }

      const remedy = finding.remedy;
      if (remedy) {
        const row = element('div', 'power-finding-remedy');
        if (remedy.kind === 'manual') {
          row.append(element('span', 'power-remedy-note', remedy.value));
        } else {
          // An action's value is the command name the frontend invokes, which is
          // not something a user should read as a shell command; only a command
          // remedy is worth showing as text.
          if (remedy.kind === 'command') {
            row.append(element('code', 'power-remedy-command', remedy.value));
          }
          const button = remedyButton(remedy, reload);
          if (button) {
            row.append(button);
          }
        }
        item.append(row);
      }

      return item;
    })
  );

  const note = document.getElementById('power-findings-note');
  if (note) {
    note.textContent = findings.length ? '' : 'Nothing to report: every check passed.';
  }
}

/**
 * What this application has written to its own drop-in.
 *
 * The file is the whole record: TLP's provenance says which of the effective
 * values came from it, so the list is read back rather than remembered.
 */
async function renderDropin() {
  const list = document.getElementById('power-dropin-list');
  const note = document.getElementById('power-dropin-note');
  const panel = document.getElementById('power-dropin-panel');
  if (!list) {
    return;
  }

  try {
    const config = unwrap(await invoke('tlp_config'));
    const path = '/etc/tlp.d/99-thinkpro-vantage.conf';
    const pathLabel = document.getElementById('power-dropin-path');
    if (pathLabel && config && config.files) {
      const match = config.files.find((file) => file.includes('thinkpro-vantage'));
      if (match) {
        pathLabel.textContent = match;
      }
    }
    const mine = [];
    for (const [key, source] of Object.entries((config && config.provenance) || {})) {
      if (source && source.path && source.path.includes('thinkpro-vantage')) {
        mine.push([key, (config.values && config.values[key]) || '', source.line]);
      }
    }
    mine.sort((left, right) => left[0].localeCompare(right[0]));

    list.replaceChildren(
      ...mine.map(([key, value, line]) => {
        const item = element('li', 'power-claim');
        item.append(element('span', 'power-claim-label', key));
        item.append(element('span', 'power-claim-owner', value || '—'));
        item.append(element('span', 'power-claim-detail', 'line ' + line));
        return item;
      })
    );
    if (note) {
      note.textContent = mine.length
        ? ''
        : config && config.available
          ? 'Nothing yet. Settings you change here are written to this one file, and removing it undoes all of them.'
          : 'TLP is not installed, so there is nothing to write.';
    }
  } catch (error) {
    if (note) {
      note.textContent = errorText(error);
    }
  }

  if (panel) {
    panel.setAttribute('aria-busy', 'false');
  }
}

async function refreshPower() {
  const [diagnosis, ownership] = await Promise.all([
    invoke('power_doctor'),
    invoke('power_owners')
  ]);
  const doctor = unwrap(diagnosis);
  renderClaims(unwrap(ownership));
  renderSummary(doctor);
  renderFindings(doctor, refreshPower);
  await renderDropin();
}

export async function loadPowerView() {
  const button = document.getElementById('btn-power-recheck');
  if (button && !button.dataset.bound) {
    button.dataset.bound = 'true';
    button.addEventListener('click', async () => {
      try {
        await runWithState(button, refreshPower, {
          success: '',
          title: 'The check did not finish'
        });
      } catch {
        /* reported by runWithState */
      }
    });
  }

  const rollback = document.getElementById('btn-power-rollback');
  if (rollback && !rollback.dataset.bound) {
    rollback.dataset.bound = 'true';
    rollback.addEventListener('click', async () => {
      try {
        await runWithState(
          rollback,
          async () => {
            unwrap(await invoke('power_rollback_dropin'));
            await refreshPower();
          },
          { success: 'Rolled back', title: 'Could not roll back' }
        );
      } catch {
        /* reported by runWithState */
      }
    });
  }

  try {
    await refreshPower();
  } catch (error) {
    console.error('[Power] Diagnosis failed:', error);
    const headline = document.getElementById('power-headline');
    if (headline) {
      headline.textContent = 'The diagnosis could not run';
    }
    const text = document.getElementById('power-summary-text');
    if (text) {
      text.textContent = errorText(error);
    }
    for (const id of ['power-summary', 'power-claims-panel', 'power-findings-panel']) {
      document.getElementById(id)?.setAttribute('aria-busy', 'false');
    }
  }
}
