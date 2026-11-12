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
  const distro = document.getElementById('power-distro');
  if (distro) {
    // Naming the detected distribution is what makes an install command credible:
    // the user can see which system the command was chosen for.
    distro.textContent = diagnosis.distro || '';
    distro.hidden = !diagnosis.distro;
  }
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

  if (remedy.kind === 'command') {
    // The label stays what the action is ("Install TLP"); what changes is that
    // pressing it opens a terminal with the command rather than only copying it.
    button.title = 'Opens a terminal with this command';
  }

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
      // A command the user has to copy, find a terminal for and paste is a command
      // most people never run, so this opens one with it. When there is no terminal
      // the command is still one copy away, which is reported rather than silently
      // doing nothing.
      try {
        await runWithState(
          button,
          async () => {
            const response = await invoke('open_terminal_with', { command: remedy.value });
            if (!response || response.success === false) {
              throw new Error(errorText(response && response.error));
            }
          },
          { success: 'Opened in a terminal', title: 'Could not open a terminal' }
        );
      } catch {
        try {
          await copyText(remedy.value);
          toast('Command copied instead', { kind: 'ok', detail: remedy.value });
        } catch (error) {
          toast('Could not copy the command', { kind: 'danger', detail: errorText(error) });
        }
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
            const code = element('code', 'power-remedy-command', remedy.value);
            code.title = 'Click to copy';
            code.addEventListener('click', async () => {
              try {
                await copyText(remedy.value);
                toast('Command copied', { kind: 'ok', detail: remedy.value });
              } catch (error) {
                toast('Could not copy the command', { kind: 'danger', detail: errorText(error) });
              }
            });
            row.append(code);
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

  // The editor reads the same configuration: keep its provenance current, but
  // never throw away edits that have not been written yet.
  if (editor.ready && !editor.writing) {
    try {
      await loadEditorData({ keepEdits: true });
    } catch (error) {
      console.warn('[Power] TLP settings could not be reloaded:', error);
    }
  }
}

// ---------------------------------------------------------------------------
// TLP settings editor
//
// The diagnosis above answers "why is this machine behaving like this". This
// half of the page is the knobs themselves: every setting the installed TLP
// ships, with the consequence written next to the control instead of in a
// manual.
//
// Three backend shapes decide the design:
//
// * tlp_schema() answers per key, and a key carries its power source in the
//   name (CPU_BOOST_ON_AC). The variants of one setting are rendered as one
//   setting with a row per variant, never as three unrelated search hits.
// * tlp_config() answers with the effective value and where it came from. That
//   provenance is printed as it is: TLP's precedence is not re-derived here.
// * power_preview_changes() is the authority on whether a value is acceptable
//   and on which of the requested keys will lose to another file. What it
//   answers is what the review panel shows, per change.
// ---------------------------------------------------------------------------

const DROPIN_PATH = '/etc/tlp.d/99-thinkpro-vantage.conf';
const DROPIN_NAME = '99-thinkpro-vantage.conf';

const SOURCE_LABELS = {
  ac: 'On AC',
  bat: 'On battery',
  sav: 'Power saver'
};

/** Mirrors the backend's Group::label() and Group::ORDER. */
const GROUP_ORDER = [
  'general',
  'cpu',
  'platform',
  'graphics',
  'disk',
  'pcie',
  'usb',
  'radio',
  'sound',
  'battery',
  'other'
];

const GROUP_TITLES = {
  general: 'General',
  cpu: 'CPU',
  platform: 'Platform',
  graphics: 'Graphics',
  disk: 'Disks',
  pcie: 'PCIe',
  usb: 'USB',
  radio: 'Radios',
  sound: 'Audio',
  battery: 'Battery care',
  other: 'Other'
};

/** One line per group: what it is for, and when a ThinkPad owner should care. */
const GROUP_BLURB = {
  general:
    'How TLP itself behaves: whether it manages power at all, which profile it picks, and how loudly it reports problems.',
  cpu: 'How hard the processor works. This is the biggest lever on battery life, heat and fan noise.',
  platform:
    'The firmware power profile the embedded controller honors. Coarse, but the most direct control over fan noise there is.',
  graphics:
    'Integrated GPU power limits, and the AMD backlight trick that saves real battery on a bright screen.',
  disk: 'Spin-down timers and I/O behaviour for internal drives. Matters for SATA disks; an NVMe drive mostly ignores it.',
  pcie: 'Idle power management for the PCIe bus. On a modern laptop this is where a surprising amount of standby power hides.',
  usb: 'Autosuspend for USB devices: a real battery win, with the occasional stubborn peripheral waking up slowly.',
  radio:
    'Wi-Fi power saving, Wake-on-LAN, and the radio device wizard that switches radios on dock, undock and connect.',
  sound:
    'Power saving for the HDA audio codec. Free battery; a few codecs add a click or a pop when waking.',
  battery:
    'Charge thresholds and the Lenovo firmware plugins. Keeping a docked ThinkPad at 80% is the single best thing you can do for its battery.',
  other: 'Settings TLP cannot place by name: suspend mode, and anything a newer TLP added.'
};

/** Settings whose mechanical name from the key reads badly, spelled out instead. */
const LABEL_OVERRIDES = {
  DISK_IDLE_SECS: 'Disk idle timeout',
  MAX_LOST_WORK_SECS: 'Disk writeback timeout',
  MEM_SLEEP: 'Suspend mode',
  WIFI_PWR: 'Wi-Fi power saving',
  SATA_LINKPWR: 'SATA link power management',
  SOUND_POWER_SAVE: 'Audio codec power saving',
  SOUND_POWER_SAVE_CONTROLLER: 'Audio controller power saving',
  TLP_DISABLE_DEFAULTS: 'Ignore TLP defaults',
  TLP_PS_IGNORE: 'Power supplies to ignore',
  DISK_APM_CLASS_DENYLIST: 'Disk classes excluded from APM',
  DISK_DEVICES: 'Disks these settings apply to',
  RADEON_DPM_PERF_LEVEL: 'Radeon power management level',
  CPU_HWP_DYN_BOOST: 'CPU dynamic boost (HWP)',
  RUNTIME_PM_DENYLIST: 'PCIe addresses excluded from autosuspend',
  RUNTIME_PM_DRIVER_DENYLIST: 'Drivers excluded from autosuspend',
  DEVICES_TO_DISABLE_ON_BAT_NOT_IN_USE: 'Radios to switch off on battery when idle',
  CPU_MIN_PERF: 'CPU minimum performance',
  CPU_MAX_PERF: 'CPU maximum performance',
  TLP_ENABLE: 'TLP power management',
  TLP_PROFILE_AC: 'TLP profile',
  TLP_PROFILE_BAT: 'TLP profile',
  TLP_PROFILE_DEFAULT: 'TLP profile fallback',
  WOL_DISABLE: 'Wake-on-LAN',
  RUNTIME_PM_ENABLE: 'PCIe addresses always autosuspended',
  RUNTIME_PM_DISABLE: 'PCIe addresses never autosuspended',
  DEVICES_TO_DISABLE: 'Radios to switch off',
  DEVICES_TO_ENABLE: 'Radios to switch on',
  RESTORE_THRESHOLDS: 'Reapply charge thresholds',
  NATACPI_ENABLE: 'NATACPI (ThinkPad ACPI) plugin',
  TPSMAPI_ENABLE: 'TPSMAPI (ThinkPad firmware) plugin',
  START_CHARGE_THRESH_BAT0: 'Battery 0: start charging below',
  STOP_CHARGE_THRESH_BAT0: 'Battery 0: stop charging at',
  START_CHARGE_THRESH_BAT1: 'Battery 1: start charging below',
  STOP_CHARGE_THRESH_BAT1: 'Battery 1: stop charging at'
};

/** Words a key is built from, in the form a person reads them. */
const WORD_MAP = {
  ac: 'AC',
  bat: 'battery',
  sav: 'power saver',
  tlp: 'TLP',
  cpu: 'CPU',
  gpu: 'GPU',
  usb: 'USB',
  pcie: 'PCIe',
  ahci: 'AHCI',
  sata: 'SATA',
  aspm: 'ASPM',
  dpm: 'DPM',
  abm: 'ABM',
  hwp: 'HWP',
  nmi: 'NMI',
  wifi: 'Wi-Fi',
  wwan: 'WWAN',
  wol: 'wake-on-LAN',
  pm: 'power management',
  pwr: 'power',
  ps: 'power supply',
  iosched: 'I/O scheduler',
  apm: 'advanced power management',
  secs: 'seconds',
  thresh: 'threshold',
  freq: 'frequency',
  dyn: 'dynamic',
  msg: 'message',
  colors: 'colors',
  auto: 'automatic',
  switch: 'switching',
  min: 'minimum',
  max: 'maximum',
  perf: 'performance',
  amdgpu: 'AMD GPU',
  radeon: 'Radeon',
  intel: 'Intel',
  spindown: 'spin-down',
  btusb: 'Bluetooth devices',
  denylist: 'denylist',
  lan: 'LAN',
  warn: 'warning',
  linkpwr: 'link power',
  poweroff: 'power-off'
};

/**
 * What each setting changes, in plain language, for a ThinkPad owner.
 *
 * Keyed by the key without its _ON_AC / _ON_BAT / _ON_SAV suffix: the variants
 * of one setting share a description and differ only in when they apply.
 * `meanings` explains the values a control offers; `unit`, `min`, `max` and
 * `choices` improve the control that ValueKind alone can describe, and `ladder`
 * marks the two rows whose value is one of the CPU's own frequencies.
 */
const COPY = {
  TLP_ENABLE: {
    text: 'The master switch. With it off, TLP still reads every setting on this page but never touches the hardware.',
    hint: 'Only turn it off if something else entirely manages power on this machine. Nothing else on this page then has any effect.'
  },
  TLP_WARN_LEVEL: {
    text: 'How loudly TLP complains when a setting cannot be applied.',
    min: 0,
    max: 3,
    meanings: {
      0: 'silent',
      1: 'background tasks log to syslog',
      2: 'commands print to the terminal',
      3: 'both, the default'
    }
  },
  TLP_MSG_COLORS: {
    text: 'ANSI colors for error, warning, notice and success messages, in that order.',
    hint: 'The value is a quoted list of ANSI codes, for example "91 93 1 92": red, yellow, bold, green.',
    placeholder: '"91 93 1 92"'
  },
  TLP_AUTO_SWITCH: {
    text: 'Whether TLP switches profile when the charger is plugged or unplugged.',
    min: 0,
    max: 2,
    meanings: {
      0: 'never switch',
      1: 'always switch to the matching profile',
      2: 'smart: keep a profile you picked by hand'
    }
  },
  TLP_PROFILE_AC: {
    text: 'The profile TLP switches to on AC power.',
    choices: ['PRF', 'BAL', 'SAV'],
    meanings: {
      PRF: 'performance',
      BAL: 'balanced',
      SAV: 'power saver'
    }
  },
  TLP_PROFILE_BAT: {
    text: 'The profile TLP switches to on battery.',
    choices: ['PRF', 'BAL', 'SAV'],
    meanings: {
      PRF: 'performance',
      BAL: 'balanced',
      SAV: 'power saver'
    }
  },
  TLP_PROFILE_DEFAULT: {
    text: 'The profile used when automatic switching is off, or when TLP cannot tell what the machine runs on.',
    choices: ['PRF', 'BAL', 'SAV'],
    meanings: {
      PRF: 'performance',
      BAL: 'balanced',
      SAV: 'power saver'
    }
  },
  TLP_PS_IGNORE: {
    text: 'Power supply classes ignored when TLP decides whether it is on AC or battery.',
    hint: 'Separated by spaces. Only useful on laptops where the power source is detected wrongly.'
  },
  TLP_DISABLE_DEFAULTS: {
    text: 'Read only your own configuration files and ignore every default TLP ships.',
    hint: 'For hand-built configurations. On any other machine it means writing all 61 defaults yourself, by hand.',
    switch: true
  },
  CPU_SCALING_GOVERNOR: {
    choices: [
      'performance',
      'powersave',
      'ondemand',
      'schedutil',
      'conservative',
      'userspace',
      'adaptive'
    ],
    text: 'Which kernel governor picks the CPU frequency. One that scales with load (schedutil, ondemand) is right for almost everyone; performance pins the clock high and saves nothing.',
    hint: 'Use tlp-stat -p to see which driver and governors this machine actually has.',
    meanings: {
      performance: 'always fast, always hot',
      powersave: 'always slow',
      ondemand: 'scales with load, efficient',
      schedutil: 'scales with load in the scheduler, efficient',
      conservative: 'scales up slowly on load',
      userspace: 'external program decides',
      adaptive: 'scales with load'
    }
  },
  CPU_MIN_PERF: {
    text: 'Floor for CPU performance, as a percentage of the maximum (intel_pstate and amd-pstate only).',
    number: true,
    unit: '%',
    min: 0,
    max: 100,
    clock: true
  },
  CPU_MAX_PERF: {
    text: 'Ceiling for CPU performance, as a percentage of the maximum (intel_pstate and amd-pstate only). A lower ceiling means a cooler, quieter machine and less burst speed.',
    number: true,
    unit: '%',
    min: 0,
    max: 100,
    clock: true
  },
  CPU_BOOST: {
    text: 'Turbo boost on Intel, core performance boost on AMD. Off keeps the CPU at or below its base clock: quieter, longer battery, noticeably slower in short bursts.',
    switch: true,
    min: 0,
    max: 1,
    meanings: {
      0: 'off: never exceed the base clock',
      1: 'on: allow boost clocks'
    }
  },
  CPU_HWP_DYN_BOOST: {
    text: 'Let the CPU power manager raise boost clocks on its own initiative, independent of the settings above (Intel HWP only).',
    switch: true,
    min: 0,
    max: 1,
    meanings: {
      0: 'off',
      1: 'on'
    }
  },
  CPU_ENERGY_PERF_POLICY: {
    choices: ['performance', 'balance_performance', 'default', 'balance_power', 'power'],
    text: 'How the CPU trades speed for power (the EPP/EPB hint). Values run from fastest to most frugal.',
    hint: 'Needs a recent Intel CPU or an AMD Zen 2 or newer with the amd-pstate driver in active mode.',
    meanings: {
      performance: 'fastest, warmest',
      balance_performance: 'lean towards speed',
      default: 'leave it to the firmware',
      balance_power: 'lean towards saving',
      power: 'save the most'
    }
  },
  CPU_DRIVER_OPMODE: {
    text: 'Operation mode of the CPU frequency driver: who actually picks the clock.',
    hint: 'active hands control to the hardware, passive gives it to the kernel governor, guided lets the hardware choose inside your limits.',
    meanings: {
      active: 'hardware decides',
      passive: 'kernel governor decides',
      guided: 'hardware decides within your bounds'
    }
  },
  CPU_SCALING_MIN_FREQ: {
    text: "Lowest CPU frequency the governor may pick. The stops are this CPU's own frequencies, so the hardware is never asked for a clock it does not have.",
    ladder: true,
    hint: 'Not recommended with intel_pstate: use CPU minimum performance instead. Set both AC and battery, or TLP refuses to apply either.'
  },
  CPU_SCALING_MAX_FREQ: {
    text: "Highest CPU frequency the governor may pick. A low ceiling is an effective way to make a hot ThinkPad quiet, and the stops are this CPU's own frequencies.",
    ladder: true,
    hint: 'Not recommended with intel_pstate: use CPU maximum performance instead. Set both AC and battery, or TLP refuses to apply either.'
  },
  PLATFORM_PROFILE: {
    choices: ['performance', 'balanced', 'low-power', 'cool', 'quiet'],
    text: 'The firmware power profile. This is the dial the embedded controller uses for fan curves, clock limits and package power, so it is the one setting that changes how loud the machine gets.',
    hint: 'Which values a ThinkPad accepts depends on the model; tlp-stat -s shows the active one.',
    meanings: {
      performance: 'fastest, warmest, loudest fan',
      balanced: 'the usual compromise',
      'low-power': 'coolest and quietest, slowest',
      cool: 'favours lower temperatures',
      quiet: 'favours fan noise, not speed'
    }
  },
  NMI_WATCHDOG: {
    text: 'The kernel watchdog that reboots a machine whose CPU has locked up. TLP ships it switched off, because the timer that feeds it wakes the CPU and costs a little battery on every tick.',
    min: 0,
    max: 1,
    meanings: {
      0: 'off, the TLP default',
      1: 'on: catches a hard lock-up, costs a little battery'
    }
  },
  MEM_SLEEP: {
    text: 'Which suspend mode to use: s2idle is a light software sleep, deep is suspend-to-RAM and saves noticeably more.',
    hint: 'TLP warns that changing this can make a machine unstable or lose data, and that not every machine offers both. If you are unsure, leave it unset and keep the firmware default.',
    choices: ['s2idle', 'deep'],
    meanings: {
      s2idle: 'software sleep, wakes fast, drains more',
      deep: 'suspend to RAM, drains less'
    }
  },
  DISK_DEVICES: {
    text: 'The disks every disk and SATA setting below applies to, in this order.',
    hint: 'Space-separated device names (nvme0n1 sda) or the disk IDs that tlp diskid prints. Values that come one per disk are matched against this order.',
    placeholder: 'nvme0n1 sda'
  },
  DISK_APM_LEVEL: {
    text: 'Advanced power management per disk: 1 is most aggressive, 254 least, 255 turns APM off. Aggressive levels spin the disk down more often, which is why they are only worth setting on rotating drives.',
    hint: 'One value per disk, in the order of DISK_DEVICES. The special value keep leaves that disk alone.',
    min: 1,
    max: 255
  },
  DISK_APM_CLASS_DENYLIST: {
    text: 'Disk classes left out of advanced power management. USB disks are excluded on purpose: APM can corrupt them.',
    hint: 'Space-separated classes: sata, ata, usb, ieee1394. Think twice before removing usb.'
  },
  DISK_IDLE_SECS: {
    text: 'How long a disk may stay idle before it is allowed to spin down. 0 disables spin-down, which is what an SSD wants.',
    unit: 's',
    min: 0
  },
  MAX_LOST_WORK_SECS: {
    text: 'How long written-but-unsaved data may stay in memory, in seconds. This is the window that is lost if power disappears.',
    unit: 's',
    min: 0
  },
  DISK_IOSCHED: {
    text: 'I/O scheduler per disk. none is the right answer for NVMe, bfq or mq-deadline for a rotating disk.',
    hint: 'One value per disk, in the order of DISK_DEVICES; keep leaves the kernel choice alone.',
    placeholder: 'bfq none'
  },
  DISK_SPINDOWN_TIMEOUT: {
    text: 'Spin-down timer per disk: 0 disables it, 1 to 240 count 5-second steps (up to 20 minutes), 241 to 251 count 30-minute steps (up to 3 hours).',
    hint: 'One value per disk, in the order of DISK_DEVICES.',
    min: 0,
    max: 251
  },
  SATA_LINKPWR: {
    text: 'SATA link power state per disk: min_power saves the most, max_performance the least. med_power_with_dipm is the usual compromise and only works with the AHCI runtime settings below.',
    hint: 'One value per disk, in the order of DISK_DEVICES.',
    meanings: {
      min_power: 'save the most',
      med_power_with_dipm: 'usual compromise',
      medium_power: 'middle ground',
      max_performance: 'no saving'
    }
  },
  SATA_LINKPWR_DENYLIST: {
    text: 'SATA link power devices to skip, by host device name.'
  },
  AHCI_RUNTIME_PM: {
    text: 'Runtime power management for SATA host controllers: auto lets an idle controller sleep, on keeps it awake.',
    choices: ['on', 'auto'],
    meanings: {
      on: 'keep controllers awake',
      auto: 'let idle controllers sleep'
    }
  },
  AHCI_RUNTIME_PM_TIMEOUT: {
    text: 'How long an idle SATA controller waits before it sleeps.',
    unit: 's',
    min: 0
  },
  PCIE_ASPM: {
    choices: ['default', 'performance', 'powersave', 'powersupersave'],
    text: 'Active State Power Management for PCIe links. Deeper states save more and occasionally upset a cheap NVMe drive or Wi-Fi card.',
    hint: 'default keeps the firmware setting, which is the safe choice on a ThinkPad.',
    meanings: {
      default: 'keep the firmware setting',
      performance: 'no link power saving',
      powersave: 'save, still widely compatible',
      powersupersave: 'save the most, most likely to misbehave'
    }
  },
  RUNTIME_PM: {
    text: 'Autosuspend for PCIe devices: auto lets idle devices power down, on keeps them awake.',
    choices: ['on', 'auto'],
    meanings: {
      on: 'no autosuspend: fastest to wake',
      auto: 'autosuspend idle devices'
    }
  },
  RUNTIME_PM_DENYLIST: {
    text: 'PCIe devices that must never autosuspend, by address.',
    hint: 'Addresses come from lspci, first column. Space-separated.'
  },
  RUNTIME_PM_DRIVER_DENYLIST: {
    text: 'Devices handled by these kernel drivers are excluded from autosuspend.',
    hint: 'Emptying this list enables autosuspend for everything including the GPU: expect trouble.'
  },
  RUNTIME_PM_ENABLE: {
    text: 'PCIe addresses forced into autosuspend on both power sources. This wins over all the settings above.',
    hint: 'Space-separated, from lspci.'
  },
  RUNTIME_PM_DISABLE: {
    text: 'PCIe addresses kept out of autosuspend on both power sources. This wins over all the settings above.',
    hint: 'Space-separated, from lspci.'
  },
  USB_AUTOSUSPEND: {
    text: 'Put idle USB devices into low-power states. This is where a lot of battery savings come from, and the reason a mouse or hub sometimes needs a second to wake.',
    min: 0,
    max: 1,
    meanings: {
      0: 'never suspend USB devices',
      1: 'suspend idle devices'
    }
  },
  USB_DENYLIST: {
    text: 'USB devices that must never autosuspend, by idVendor:idProduct from lsusb.',
    hint: 'Input devices and supported scanners are already excluded automatically.'
  },
  USB_ALLOWLIST: {
    text: 'With a list here, only these devices may autosuspend at all; everything else stays awake.',
    hint: 'idVendor:idProduct pairs, space-separated.'
  },
  USB_EXCLUDE_AUDIO: {
    text: 'Never autosuspend audio devices. USB headsets and DACs otherwise produce crackle or fail to come back.',
    min: 0,
    max: 1
  },
  USB_EXCLUDE_BTUSB: {
    text: 'Never autosuspend Bluetooth radios.',
    min: 0,
    max: 1
  },
  USB_EXCLUDE_PHONE: {
    text: 'Never autosuspend phones: most need to stay awake to keep charging.',
    min: 0,
    max: 1
  },
  USB_EXCLUDE_PRINTER: {
    text: 'Never autosuspend printers.',
    min: 0,
    max: 1
  },
  USB_EXCLUDE_WWAN: {
    text: 'Never autosuspend mobile broadband modems.',
    min: 0,
    max: 1
  },
  WIFI_PWR: {
    text: 'Wi-Fi power saving: on trades a little latency for battery (an occasional lag spike), off keeps the radio fully awake.',
    choices: ['on', 'off'],
    meanings: {
      on: 'save battery, occasional latency spike',
      off: 'full radio power'
    }
  },
  WOL_DISABLE: {
    text: 'Disable Wake-on-LAN: y stops the wired network card from waking the laptop, which also stops it draining the battery while suspended. n leaves wake-on-LAN enabled.'
  },
  DEVICES_TO_DISABLE: {
    text: 'Radio devices switched off automatically when the charger is unplugged.',
    hint: 'Any of: bluetooth, wifi, wwan. Space-separated. Needs the optional tlp-rdw package.',
    placeholder: 'bluetooth wwan'
  },
  DEVICES_TO_DISABLE_ON_BAT_NOT_IN_USE: {
    text: 'Radios with no active connection are switched off on battery.',
    hint: 'Any of: bluetooth, wifi, wwan.'
  },
  DEVICES_TO_ENABLE: {
    text: 'Radio devices switched back on when the charger is connected.',
    hint: 'Any of: bluetooth, wifi, wwan.'
  },
  DEVICES_TO_DISABLE_ON_STARTUP: {
    text: 'Radio devices switched off when TLP starts, which is at every boot.',
    hint: 'Any of: bluetooth, wifi, wwan.'
  },
  DEVICES_TO_ENABLE_ON_STARTUP: {
    text: 'Radio devices switched on when TLP starts, which is at every boot.',
    hint: 'Any of: bluetooth, wifi, wwan.'
  },
  DEVICES_TO_DISABLE_ON_LAN_CONNECT: {
    text: 'Radio devices switched off as soon as a LAN cable or dock port connects.',
    hint: 'Handy with a dock: wired networking on, Wi-Fi and WWAN off.'
  },
  DEVICES_TO_ENABLE_ON_LAN_DISCONNECT: {
    text: 'Radio devices switched back on when the LAN cable is disconnected.'
  },
  DEVICES_TO_DISABLE_ON_WIFI_CONNECT: {
    text: 'Radio devices switched off when Wi-Fi associates, typically WWAN.',
    hint: 'The wired equivalent is DEVICES_TO_DISABLE_ON_LAN_CONNECT.'
  },
  DEVICES_TO_ENABLE_ON_WIFI_DISCONNECT: {
    text: 'Radio devices switched back on when Wi-Fi disconnects.'
  },
  DEVICES_TO_DISABLE_ON_WWAN_CONNECT: {
    text: 'Radio devices switched off when the mobile modem connects, typically Wi-Fi.'
  },
  DEVICES_TO_ENABLE_ON_WWAN_DISCONNECT: {
    text: 'Radio devices switched back on when the mobile modem disconnects.'
  },
  DEVICES_TO_ENABLE_ON_DOCK: {
    text: 'Radio devices switched on when the machine is docked.',
    hint: 'Not every dock is recognised, especially USB-C ones: if a LAN cable is involved, use the LAN connect settings instead.'
  },
  DEVICES_TO_DISABLE_ON_DOCK: {
    text: 'Radio devices switched off when the machine is docked.'
  },
  DEVICES_TO_ENABLE_ON_UNDOCK: {
    text: 'Radio devices switched on when the machine is undocked.'
  },
  DEVICES_TO_DISABLE_ON_UNDOCK: {
    text: 'Radio devices switched off when the machine is undocked.'
  },
  SOUND_POWER_SAVE: {
    text: 'Seconds of silence before the HDA audio codec is allowed to sleep. 0 disables it, 1 is the usual choice; a few codecs need 10 before their amplifier actually powers down.',
    unit: 's',
    min: 0,
    hint: 'TLP reads this value as a switch, so this editor offers 0 or 1.'
  },
  SOUND_POWER_SAVE_CONTROLLER: {
    text: 'Also power down the codec controller while the value above is active: slightly more saving, occasionally a small pop when playback starts.'
  },
  BAY_POWEROFF: {
    text: 'Power off the optical drive bay, if this ThinkPad has one.',
    min: 0,
    max: 1,
    meanings: {
      0: 'leave the bay powered',
      1: 'power the bay off'
    }
  },
  BAY_DEVICE: {
    text: 'Which device the bay setting above acts on.',
    placeholder: 'sr0'
  },
  RESTORE_DEVICE_STATE_ON_STARTUP: {
    text: 'Restore the USB and PCIe device state that was saved when the machine suspended. TLP marks this deprecated: the kernel handles device restore on resume now.'
  },
  RESTORE_THRESHOLDS: {
    text: 'Reapply the battery charge thresholds while running on battery. Some Lenovo models lose them when the charger is removed, which is exactly what this fixes.',
    min: 0,
    max: 1,
    meanings: {
      0: 'leave thresholds alone on battery',
      1: 'reapply them, the safe default'
    }
  },
  NATACPI_ENABLE: {
    text: 'The older ThinkPad ACPI integration: battery, fan and brightness through the kernel thinkpad_acpi driver. TLP 1.10 replaced it with plugins and keeps this value in its own defaults.',
    hint: 'This TLP version documents nothing here. Leave it at the default unless TLP release notes tell you otherwise.'
  },
  TPSMAPI_ENABLE: {
    text: 'ThinkPad SMAPI, the firmware interface older Lenovo models use for brightness and hotkeys. TLP 1.10 replaced it with plugins and keeps this value in its own defaults.',
    hint: 'This TLP version documents nothing here. Leave it at the default unless TLP release notes tell you otherwise.'
  },
  START_CHARGE_THRESH_BAT0: {
    text: 'Charging starts again once the battery drops below this percentage.',
    number: true,
    unit: '%',
    min: 0,
    max: 100,
    hint: 'A ThinkPad needs both thresholds: set start and stop together, or TLP refuses to apply either.'
  },
  STOP_CHARGE_THRESH_BAT0: {
    text: 'Charging stops at this percentage. On a ThinkPad that lives on a desk, stopping at 80 instead of 100 is the single best thing you can do for the battery.',
    number: true,
    unit: '%',
    min: 0,
    max: 100,
    hint: 'Must be higher than the start threshold, and both must be set.'
  },
  START_CHARGE_THRESH_BAT1: {
    text: 'Charging starts again once the second battery drops below this percentage.',
    number: true,
    unit: '%',
    min: 0,
    max: 100
  },
  STOP_CHARGE_THRESH_BAT1: {
    text: 'Charging stops at this percentage for the second battery.',
    number: true,
    unit: '%',
    min: 0,
    max: 100
  },
  INTEL_GPU_POWER_PROFILE: {
    text: 'Intel GPU power policy: base favours speed, power_saving favours battery.',
    choices: ['base', 'power_saving'],
    meanings: {
      base: 'favour GPU speed',
      power_saving: 'favour battery'
    }
  },
  INTEL_GPU_MIN_FREQ: {
    text: 'Lowest Intel GPU frequency, in MHz. 0 means no limit.',
    number: true,
    unit: 'MHz',
    min: 0
  },
  INTEL_GPU_MAX_FREQ: {
    text: 'Highest Intel GPU frequency, in MHz. A lower ceiling cuts heat and fan noise while gaming or driving an external display; 0 means no limit.',
    number: true,
    unit: 'MHz',
    min: 0
  },
  INTEL_GPU_BOOST_FREQ: {
    text: 'Boost frequency ceiling for the Intel GPU, in MHz. 0 means no limit.',
    number: true,
    unit: 'MHz',
    min: 0
  },
  RADEON_DPM_PERF_LEVEL: {
    text: 'Radeon dynamic power management: auto lets the driver decide, low saves power, high prefers performance.',
    choices: ['auto', 'low', 'high'],
    meanings: {
      auto: 'driver decides',
      low: 'save power',
      high: 'prefer performance'
    }
  },
  AMDGPU_ABM_LEVEL: {
    text: 'Adaptive Backlight Management dims the panel backlight while keeping the picture readable. It is the best screen-power trick on an AMD ThinkPad.',
    min: 0,
    max: 4,
    meanings: {
      0: 'off, full backlight',
      1: 'a little dimming',
      2: 'middle',
      3: 'strong dimming',
      4: 'maximum saving, least punch'
    }
  }
};

/** The editor's whole state. Kept in module scope so switching views keeps edits. */
const editor = {
  ready: false,
  loading: false,
  writing: false,
  tab: 'diagnosis',
  entries: [],
  byKey: new Map(),
  values: new Map(),
  baseline: new Map(),
  collapsed: new Set(),
  filter: { search: '', changed: false, available: true },
  request: 0,
  review: null
};

function rowOf(entry) {
  return entry.row || entry.key;
}

/** 'CPU_SCALING_GOVERNOR_ON_AC' -> 'CPU scaling governor'. */
function humanLabel(row) {
  if (LABEL_OVERRIDES[row]) {
    return LABEL_OVERRIDES[row];
  }
  const words = row.split('_').map((raw) => {
    const word = raw.toLowerCase();
    const battery = /^bat(\d)$/.exec(word);
    if (battery) {
      return 'battery ' + battery[1];
    }
    return WORD_MAP[word] || word;
  });
  const text = words.join(' ');
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function copyOf(item) {
  return COPY[item.row] || {};
}

/**
 * The clock range of this CPU, from the command the performance view already
 * uses. A performance percentage is a ratio of that range and nothing more: the
 * kernel maps its own `max_perf_pct` onto the hardware limits, so the frequency
 * derived from it is an estimate to read, not a promise.
 */
let clockLimits = null;

async function loadClockLimits() {
  if (clockLimits) {
    return;
  }
  try {
    clockLimits = unwrap(await invoke('get_clock_limits'));
  } catch (error) {
    // A backend without that command, or a machine that reports no usable
    // range, leaves the sliders without a frequency to show -- which is fine.
    clockLimits = null;
    console.warn('[Power] The CPU clock range could not be read:', error);
  }
}

/** A frequency in kHz, as the GHz a person reads. */
function gigahertz(khz) {
  return (khz / 1000000).toFixed(2) + ' GHz';
}

/** The clock a performance percentage lands on, or '' when no range is known. */
function clockEstimate(percent) {
  if (!clockLimits || percent === null || percent === '') {
    return '';
  }
  const min = Number(clockLimits.cpu_hw_min_khz);
  const max = Number(clockLimits.cpu_hw_max_khz);
  const ratio = Number(percent) / 100;
  if (!Number.isFinite(min) || !Number.isFinite(max) || max <= min || !Number.isFinite(ratio)) {
    return '';
  }
  return '≈ ' + gigahertz(min + (max - min) * Math.min(1, Math.max(0, ratio)));
}

/**
 * The two rows whose value is one of the CPU's own frequencies.
 *
 * TLP hands the number straight to the kernel's `scaling_min_freq` and
 * `scaling_max_freq` nodes, which take kHz; the kernel then clamps and rounds
 * whatever it is given. Nothing here depends on it rejecting a bad number: the
 * ladder exists so that every value this editor can produce is a clock the CPU
 * actually runs at, which is a different promise from "the hardware will cope".
 *
 * `side` names the end of the range that means "no limit" for that row: a floor
 * at the hardware minimum is no floor, a ceiling at the hardware maximum is no
 * ceiling. An unset control parks there and still reads "not set".
 */
const FREQUENCY_ROWS = {
  CPU_SCALING_MIN_FREQ: { side: 'min' },
  CPU_SCALING_MAX_FREQ: { side: 'max' }
};

/** A frequency in kHz, as the MHz a specification sheet quotes it in. */
function megahertz(khz) {
  const mhz = khz / 1000;
  return (Number.isInteger(mhz) ? String(mhz) : mhz.toFixed(3).replace(/0+$/, '')) + ' MHz';
}

/** Which end of the range means "no limit" on this row, or null for other rows. */
function frequencySideOf(item) {
  const known = FREQUENCY_ROWS[rowOf(item)];
  return known ? known.side : null;
}

/** TLP writes no frequency when the key is unset; its own `0` means no limit. */
function isUnsetFrequency(value) {
  const text = String(value === undefined || value === null ? '' : value).trim();
  return text === '' || text === '0';
}

/**
 * The CPU's frequency ladder as the backend published it, or null when there is
 * none to be had: an older backend, or a machine whose driver reports neither a
 * list nor a range. The list is sorted and deduped here so that the slider's
 * index arithmetic is the only thing about it that has to be right.
 */
function cpuLadder() {
  const published = clockLimits && clockLimits.cpu_frequencies;
  const listed = published && Array.isArray(published.frequencies) ? published.frequencies : [];
  const frequencies = Array.from(
    new Set(listed.map(Number).filter((khz) => Number.isFinite(khz) && khz > 0))
  ).sort((a, b) => a - b);
  if (!frequencies.length) {
    return null;
  }
  const knee = Number(published && published.lowest_nonlinear_khz);
  return {
    frequencies,
    source: published && published.source === 'kernel' ? 'kernel' : 'synthesised',
    lowest_nonlinear_khz: Number.isFinite(knee) && knee > 0 ? knee : null
  };
}

/** The stop closest to a frequency, for showing a value that is not one of them. */
function nearestStopIndex(frequencies, khz) {
  if (!Number.isFinite(khz)) {
    return frequencies.length - 1;
  }
  let best = 0;
  for (let index = 1; index < frequencies.length; index += 1) {
    if (Math.abs(frequencies[index] - khz) < Math.abs(frequencies[best] - khz)) {
      best = index;
    }
  }
  return best;
}

/**
 * Where the efficiency knee sits along the ladder, as a fractional index.
 *
 * The knee is a real frequency and usually not one of the stops -- 1.11 GHz
 * lies between the 1.10 and 1.20 GHz stops of this machine -- so it is placed
 * between the two stops it lies between. Looking it up by a value that has to
 * match a stop would drop the marker on the machines it describes.
 */
function kneePosition(frequencies, knee) {
  if (!knee) {
    return null;
  }
  const last = frequencies.length - 1;
  if (knee <= frequencies[0]) {
    return 0;
  }
  if (knee >= frequencies[last]) {
    return last;
  }
  let upper = 1;
  while (upper < last && frequencies[upper] < knee) {
    upper += 1;
  }
  const lower = upper - 1;
  const span = frequencies[upper] - frequencies[lower];
  return lower + (span > 0 ? (knee - frequencies[lower]) / span : 0);
}

/** The selected frequency in the units the kernel, the spec sheet and an eye read. */
function frequencyTexts(khz) {
  return {
    clock: gigahertz(khz),
    sheet: megahertz(khz),
    written: khz + ' kHz'
  };
}

/**
 * What a pending value will be written as. A frequency is shown with the unit
 * TLP hands the kernel and with the GHz the thumb sits on, so the badge agrees
 * with the control it belongs to.
 */
function pendingText(item, value) {
  const khz = Number(value);
  if (frequencySideOf(item) && Number.isFinite(khz) && khz > 0) {
    return khz + ' kHz (' + gigahertz(khz) + ')';
  }
  return value;
}

/**
 * What the ladder is, said once for the row rather than once per profile.
 *
 * Both frequency settings read from the same ladder, and the same paragraph
 * three times over -- AC, battery, and when either is exhausted -- is noise.
 */
function frequencyLadderNote(row) {
  const ladder = FREQUENCY_ROWS[row] ? cpuLadder() : null;
  if (!ladder) {
    return '';
  }
  const stops = ladder.frequencies;
  const low = stops[0];
  const high = stops[stops.length - 1];
  const parts = [
    ladder.source === 'kernel'
      ? 'The kernel publishes a discrete frequency list for this CPU, and these are its ' +
        stops.length +
        ' entries: ' +
        low +
        ' kHz to ' +
        high +
        ' kHz.'
      : "This CPU's driver publishes no discrete frequency list, so the stops are its own range in " +
        '100 MHz steps: ' +
        stops.length +
        ' frequencies from ' +
        low +
        ' kHz to ' +
        high +
        ' kHz.',
    'That is ' +
      gigahertz(low) +
      ' to ' +
      gigahertz(high) +
      ". The ends are the hardware's own limits: the top stop means no ceiling, the bottom stop means no floor.",
    'TLP passes the number to the kernel in kHz.'
  ];
  if (ladder.lowest_nonlinear_khz) {
    parts.splice(
      2,
      0,
      'Below ' +
        ladder.lowest_nonlinear_khz +
        ' kHz (' +
        gigahertz(ladder.lowest_nonlinear_khz) +
        '), which the tick on the track marks, the CPU runs in its linear region, where a higher clock costs more voltage than it saves.'
    );
  }
  return parts.join(' ');
}

function descriptionOf(item) {
  const copy = copyOf(item);
  if (copy.text) {
    return copy.text;
  }
  const documented = item.doc && item.doc.description ? String(item.doc.description).trim() : '';
  if (documented) {
    return 'TLP documents this one as: ' + documented;
  }
  return 'This TLP version ships no description for this setting, so its name is all there is. Check TLP release notes before changing it.';
}

function hintOf(item) {
  const copy = copyOf(item);
  const parts = [];
  if (copy.hint) {
    parts.push(copy.hint);
  }
  if (item.doc && item.doc.default_note) {
    parts.push('TLP documents the default as ' + item.doc.default_note + '.');
  }
  return parts.join(' ');
}

function kindOf(entry) {
  return entry && entry.kind && entry.kind.kind ? entry.kind.kind : 'text';
}

/**
 * Whether a switch has to be written as 1/0 or as Y/N.
 *
 * TLP is not uniform: WOL_DISABLE and SOUND_POWER_SAVE_CONTROLLER are compared
 * to the letter Y by TLP's own scripts, while everything else takes 1/0. The
 * intrinsic default is the only reliable tell the schema gives us.
 */
function boolFlavor(item) {
  const fallback = String(item.default || '')
    .trim()
    .toUpperCase();
  return fallback === 'Y' || fallback === 'N' ? 'yn' : '01';
}

function isOn(item, value) {
  if (value === undefined || value === null || value === '') {
    return false;
  }
  if (boolFlavor(item) === 'yn') {
    return value === 'Y' || value === 'y' || value === '1';
  }
  return value === '1' || value === 'Y';
}

/**
 * TLP's own taxonomy has no prefix for the charge thresholds, so the backend
 * files them under "Other" — although TLP's template heads them "Battery Care".
 * Showing the one setting a docked ThinkPad owner comes here for under "Other"
 * would be a usability bug, so they are displayed in Battery care.
 */
function displayGroup(entry) {
  const row = rowOf(entry);
  if (/^(START|STOP)_CHARGE_THRESH_BAT(\d+)$/.test(row)) {
    return 'battery';
  }
  if (row === 'MAX_LOST_WORK_SECS') {
    return 'disk';
  }
  return entry.group || 'other';
}

function isOurs(item) {
  return Boolean(item.provenance && item.provenance.path === DROPIN_PATH);
}

/** A file that is read after this app's drop-in beats anything written here. */
function isLoser(item) {
  return Boolean(item.provenance && losesToApp(item.provenance.path));
}

function sourceBadge(item) {
  const provenance = item.provenance;
  if (!provenance) {
    return null;
  }
  let text;
  if (provenance.origin === 'defaults') {
    text = 'TLP default';
  } else if (provenance.origin === 'user') {
    text = '/etc/tlp.conf:' + provenance.line;
  } else if (provenance.origin === 'dropin') {
    text = provenance.path.replace('/etc/tlp.d/', '') + ':' + provenance.line;
  } else {
    text = provenance.path + ':' + provenance.line;
  }
  const badge = element('span', 'tlp-badge is-source' + (isLoser(item) ? ' is-lost' : ''), text);
  if (isLoser(item)) {
    badge.title =
      "Your /etc/tlp.conf is read after this app's drop-in, so this value wins and a change written here will not take effect.";
  } else if (provenance.origin === 'defaults') {
    badge.title = "This is TLP's built-in default; anything written here replaces it.";
  }
  return badge;
}

function entryMatchesSearch(item, words) {
  if (!words.length) {
    return true;
  }
  const haystack = [item.key, item.row, item.label, descriptionOf(item), hintOf(item)]
    .join(' ')
    .toLowerCase();
  return words.every((word) => haystack.indexOf(word) !== -1);
}

function matchesFilters(item, words) {
  const filter = editor.filter;
  if (filter.available && item.deprecated) {
    return false;
  }
  if (filter.changed && !isDirty(item)) {
    return false;
  }
  return entryMatchesSearch(item, words);
}

/**
 * Whether the operator has a pending change on this parameter. A setting this
 * machine cannot use is never a change: nothing in it can be edited, so it must
 * not reach the dirty count, the preview, or the payload that gets written.
 */
function isDirty(item) {
  if (!item.applies) {
    return false;
  }
  return editor.values.get(item.key) !== item.baseline;
}

function currentValue(item) {
  const value = editor.values.get(item.key);
  return value === undefined ? item.baseline : value;
}

function displayValue(item, value) {
  if (value === undefined || value === null || value === '') {
    return 'not set';
  }
  if (item.kind === 'bool') {
    return (isOn(item, value) ? 'on' : 'off') + ' (' + value + ')';
  }
  return value;
}

/** The value a control falls back to when the operator wants no change here. */
function unsetValue(item) {
  if (item.effective !== '' && !isOurs(item)) {
    return { value: item.effective, label: 'Leave it as it is now (' + item.effective + ')' };
  }
  if (item.default !== '') {
    return { value: item.default, label: 'TLP default (' + item.default + ')' };
  }
  return null;
}

function variantMeta(item) {
  const nodes = [];
  if (item.provenance && !isOurs(item)) {
    const badge = sourceBadge(item);
    if (badge) {
      nodes.push(badge);
    }
  }
  nodes.push(element('span', 'tlp-meta', 'now: ' + displayValue(item, item.effective)));
  if (item.default !== '' && item.default !== item.effective) {
    nodes.push(element('span', 'tlp-meta', 'TLP default: ' + displayValue(item, item.default)));
  }
  if (isDirty(item)) {
    const value = currentValue(item);
    nodes.push(
      element(
        'span',
        'tlp-badge is-pending',
        value === '' ? 'will write nothing' : 'will write: ' + pendingText(item, value)
      )
    );
  }
  return nodes;
}

/**
 * The control for one entry, chosen from the kind TLP's own text implies.
 * A free-text box is never offered where the accepted values are known.
 */
/** How a control records what the operator chose. */
function setPending(item, value) {
  if (item.applies) {
    editor.values.set(item.key, value);
  }
}

/** Point a control at a value, without disturbing a caret already in it. */
function setValue(control, value) {
  if (control.value !== value) {
    control.value = value;
  }
}

/**
 * A percentage setting as a slider with a live readout.
 *
 * The frequency beside the percentage is marked as the estimate it is: TLP
 * writes a ratio into the kernel's own `*_perf_pct` nodes, and the kernel maps
 * that ratio onto the hardware's clock range, which is what we read here.
 *
 * "Not set" has no position on a track. The thumb parks at the TLP default when
 * there is one and at the top of the range when there is not, and the readout
 * says "not set" outright, so the position never claims a value nobody chose.
 */
function percentSlider(item, copy, wrap, value, onChange) {
  const min = Number.isFinite(copy.min) ? copy.min : 0;
  const max = Number.isFinite(copy.max) ? copy.max : 100;
  const numeric = value === '' ? null : Number(value);
  const floor = Number.isFinite(numeric) ? Math.min(min, numeric) : min;
  const ceiling = Number.isFinite(numeric) ? Math.max(max, numeric) : max;
  const declared = Number(item.default);
  const parked = item.default !== '' && Number.isFinite(declared) ? declared : ceiling;

  const line = element('div', 'tlp-slider-line');
  const input = document.createElement('input');
  input.type = 'range';
  input.className = 'tlp-slider';
  input.min = String(floor);
  input.max = String(ceiling);
  input.step = '1';
  input.setAttribute('aria-label', item.label + ' — ' + item.sourceLabel);

  const readout = element('span', 'tlp-slider-readout');
  const percent = element('span', 'tlp-slider-percent', '');
  readout.append(percent);
  const clock = copy.clock ? element('span', 'tlp-slider-clock', '') : null;
  if (clock) {
    readout.append(clock);
  }
  line.append(input, readout);

  const sync = () => {
    const current = currentValue(item);
    const unset = current === '';
    setValue(input, unset ? String(parked) : current);
    percent.textContent = unset ? 'not set' : current + '%';
    line.classList.toggle('is-unset', unset);
    const estimate = unset ? '' : clockEstimate(current);
    if (clock) {
      clock.textContent = estimate;
    }
    input.setAttribute(
      'aria-valuetext',
      [unset ? 'not set — move the slider to set a value' : current + ' per cent', estimate]
        .filter(Boolean)
        .join(', ')
    );
  };
  input.addEventListener('input', () => {
    setPending(item, input.value.trim());
    sync();
    onChange();
  });
  sync();
  wrap.append(line);
  return sync;
}

/**
 * The CPU frequency rows: a slider whose stops are the frequencies this CPU
 * actually has, and "not set" as a state of its own.
 *
 * Forty stops is not a track a thumb can be aimed along to the kHz, but it is a
 * track that cannot lie: every position is an index into the ladder the backend
 * read out of the kernel, so no drag, no arrow key and no resurrected value can
 * put a frequency into TLP that the hardware does not have. Zero is not a stop
 * and is unreachable -- the ladder's ends are the hardware's own minimum and
 * maximum, and the control spans indices, never frequencies.
 *
 * "not set" is a checkbox rather than the foot of the track, because no
 * frequency is not the same thing as the lowest one. While it is set the thumb
 * parks at the stop that means "no limit" on this side and the readout says
 * "not set" outright, so the position never claims a value nobody chose.
 *
 * Returns false when the backend published no ladder, leaving the caller to
 * offer the guarded field instead.
 */
function ladderControl(item, wrap, onChange) {
  const ladder = cpuLadder();
  const side = frequencySideOf(item);
  if (!ladder || !side) {
    return false;
  }
  const stops = ladder.frequencies;
  const last = stops.length - 1;
  const parked = side === 'min' ? 0 : last;
  const knee = kneePosition(stops, ladder.lowest_nonlinear_khz);

  const line = element('div', 'tlp-slider-line');
  const track = element('div', 'tlp-ladder-track');
  const input = document.createElement('input');
  input.type = 'range';
  input.className = 'tlp-slider';
  input.min = '0';
  input.max = String(last);
  input.step = '1';
  input.setAttribute('aria-label', item.label + ' — ' + item.sourceLabel);
  track.append(input);

  if (knee !== null && last > 0) {
    // Drawn between the two stops the knee lies between, and it is not a stop:
    // it is a property of the part, not a frequency to pick.
    const marker = element('span', 'tlp-ladder-knee', '');
    marker.setAttribute('aria-hidden', 'true');
    marker.style.left = 'calc(8px + (100% - 16px) * ' + knee / last + ')';
    marker.title =
      ladder.lowest_nonlinear_khz +
      ' kHz (' +
      gigahertz(ladder.lowest_nonlinear_khz) +
      ') is where this CPU leaves its linear voltage region. It is not one of the stops, and it cannot be picked: it is where scaling stops being cheap.';
    track.append(marker);
  }

  const readout = element('span', 'tlp-slider-readout');
  // A range input reports its own value -- the ladder index -- to assistive
  // technology, which is a number that means nothing on its own. The readout
  // names the frequency, so it is bound to the slider as its description and
  // repeated as the slider's value text.
  readout.id = 'ladder-readout-' + item.key;
  input.setAttribute('aria-describedby', readout.id);
  const clock = element('span', 'tlp-slider-percent', '');
  const sheet = element('span', 'tlp-slider-clock', '');
  const below = element('span', 'tlp-ladder-below', 'below the knee');
  below.hidden = true;
  if (ladder.lowest_nonlinear_khz) {
    below.title =
      'Below ' +
      gigahertz(ladder.lowest_nonlinear_khz) +
      ' the CPU runs in its linear region, where raising the clock costs more voltage than it saves.';
  }
  readout.append(clock, sheet, below);
  line.append(track, readout);

  const unset = element('label', 'tlp-ladder-unset');
  const box = document.createElement('input');
  box.type = 'checkbox';
  unset.append(box, element('span', null, 'not set — write no frequency here'));

  const sync = () => {
    const current = currentValue(item);
    const unsetNow = isUnsetFrequency(current);
    const index = unsetNow ? parked : nearestStopIndex(stops, Number(current));
    const khz = stops[index];
    const texts = frequencyTexts(khz);
    setValue(input, String(index));
    box.checked = unsetNow;
    line.classList.toggle('is-unset', unsetNow);
    clock.textContent = unsetNow ? 'not set' : texts.clock;
    sheet.textContent = unsetNow ? '' : texts.sheet;
    below.hidden = unsetNow || !ladder.lowest_nonlinear_khz || khz >= ladder.lowest_nonlinear_khz;
    input.setAttribute(
      'aria-valuetext',
      unsetNow
        ? 'not set — move the slider to pick a frequency this CPU has'
        : texts.clock + ', ' + texts.sheet + ', ' + texts.written
    );
    input.title = unsetNow ? '' : texts.clock + ' — ' + texts.sheet + ' — ' + texts.written;
  };

  input.addEventListener('input', () => {
    const khz = stops[Number(input.value)];
    if (!Number.isFinite(khz)) {
      // Only an index into the ladder is ever recorded, so a value that is not
      // one of its stops is put back rather than written anywhere.
      sync();
      return;
    }
    setPending(item, String(khz));
    sync();
    onChange();
  });
  box.addEventListener('change', () => {
    setPending(item, box.checked ? '' : String(stops[parked]));
    sync();
    onChange();
  });

  sync();
  wrap.append(line, unset);
  wrap.syncValue = sync;
  return true;
}

/**
 * The frequency rows when there is no ladder to offer: an older backend, or a
 * machine whose driver reported neither a list nor a range.
 *
 * The field stays a plain one, but a real one: the accepted range is the
 * hardware's own, and a value outside it -- 0 above all, which is not a
 * frequency -- is refused rather than recorded, let alone written.
 */
function frequencyField(item, wrap, onChange) {
  const low = Number(clockLimits && clockLimits.cpu_hw_min_khz);
  const high = Number(clockLimits && clockLimits.cpu_hw_max_khz);
  const min = Number.isFinite(low) && low > 0 ? low : null;
  const max = Number.isFinite(high) && high > 0 ? high : null;

  const line = element('div', 'tlp-inline-value');
  const input = document.createElement('input');
  input.type = 'number';
  input.className = 'tlp-input tlp-number is-khz';
  input.step = '1000';
  input.min = min === null ? '1' : String(min);
  if (max !== null) {
    input.max = String(max);
  }
  input.placeholder = min !== null && max !== null ? min + '–' + max : 'kHz';
  input.setAttribute('aria-label', item.label + ' — ' + item.sourceLabel);
  line.append(input, element('span', 'tlp-unit', 'kHz'));

  const range =
    min !== null && max !== null
      ? min + ' to ' + max + ' kHz (' + gigahertz(min) + ' to ' + gigahertz(max) + ')'
      : 'the kHz range this CPU reports';
  const error = element('p', 'tlp-note is-warn', '');
  error.hidden = true;

  const sync = () => {
    setValue(input, currentValue(item));
    line.classList.remove('is-invalid');
    error.hidden = true;
    error.textContent = '';
  };

  const accept = () => {
    const text = input.value.trim();
    if (text === '') {
      setPending(item, '');
      return '';
    }
    const khz = Number(text);
    if (!Number.isFinite(khz) || khz <= 0) {
      return 'A frequency in kHz is needed here: 0 is not one, and it is not written.';
    }
    if ((min !== null && khz < min) || (max !== null && khz > max)) {
      return 'This CPU runs between ' + range + ', so nothing outside that can be set here.';
    }
    setPending(item, text);
    return '';
  };

  input.addEventListener('input', () => {
    // Typing a frequency passes through values no CPU has ("1", "18", "180"),
    // so nothing is said while the field has the caret: the value is merely not
    // recorded until it is one the hardware could take.
    accept();
    line.classList.remove('is-invalid');
    error.hidden = true;
    error.textContent = '';
    onChange();
  });
  input.addEventListener('change', () => {
    // Leaving the field or pressing Enter decides it: a value that was refused
    // is named as refused and the last accepted one is put back, so the box
    // never keeps a value that was not recorded.
    const message = accept();
    if (!message) {
      return;
    }
    error.hidden = false;
    error.textContent = message;
    line.classList.add('is-invalid');
    input.value = currentValue(item);
    onChange();
  });

  sync();
  wrap.append(
    line,
    error,
    element(
      'p',
      'tlp-note',
      "This CPU's frequency list could not be read, so the accepted range is " +
        range +
        '. Leave the box empty to write no frequency.'
    )
  );
  wrap.syncValue = sync;
}

function buildControl(item, onChange) {
  const kind = item.kind;
  const copy = copyOf(item);
  const wrap = element('div', 'tlp-variant-control');
  const value = currentValue(item);
  const choices = Array.isArray(copy.choices) ? copy.choices : item.enumValues;

  // TLP's text cannot always tell a switch from a free-text field: a setting
  // with no intrinsic default and a description that spells out 0 and 1 is
  // typed as text. The copy table says which of those are switches.
  if (kind === 'bool' || copy.switch === true) {
    const flavor = boolFlavor(item);
    const on = flavor === 'yn' ? 'Y' : '1';
    const off = flavor === 'yn' ? 'N' : '0';
    const line = element('div', 'tlp-inline-value');
    const toggle = element('label', 'toggle-switch-small');
    const input = document.createElement('input');
    input.type = 'checkbox';
    input.checked = isOn(item, value);
    input.setAttribute('aria-label', item.label + ' — ' + item.sourceLabel);
    const slider = element('span', 'toggle-slider-small');
    const state = element('span', 'tlp-unit', '');
    const sync = () => {
      input.checked = isOn(item, currentValue(item));
      state.textContent = input.checked ? 'on' : 'off';
    };
    input.addEventListener('change', () => {
      setPending(item, input.checked ? on : off);
      sync();
      onChange();
    });
    sync();
    toggle.append(input, slider);
    line.append(toggle, state);
    wrap.append(line);
    wrap.syncValue = sync;
    return wrap;
  }

  if (choices && choices.length) {
    const select = element('select', 'tlp-select');
    select.setAttribute('aria-label', item.label + ' — ' + item.sourceLabel);
    const unset = unsetValue(item);
    if (unset) {
      const option = document.createElement('option');
      option.value = unset.value;
      option.textContent = unset.label;
      select.append(option);
    } else {
      // Nothing is set and TLP ships no default: without this the browser shows
      // the first allowed value while the setting is in fact unset.
      const option = document.createElement('option');
      option.value = '';
      option.textContent = 'not set';
      select.append(option);
    }
    for (const choice of choices) {
      const option = document.createElement('option');
      option.value = choice;
      option.textContent = choice;
      select.append(option);
    }
    const known = choices.indexOf(value) !== -1 || (unset && unset.value === value);
    if (!known && value !== '') {
      const option = document.createElement('option');
      option.value = value;
      option.textContent = value + ' (current, not in TLP docs)';
      select.append(option);
    }
    const note = element('p', 'tlp-note', '');
    const definitions = copy.meanings || {};
    const sync = () => {
      setValue(select, currentValue(item));
      note.textContent = definitions[select.value] || '';
    };
    select.addEventListener('change', () => {
      setPending(item, select.value);
      sync();
      onChange();
    });
    sync();
    wrap.append(select, note);
    wrap.syncValue = sync;
    return wrap;
  }

  // A percentage is a position on a range, not a number to type blind. The
  // slider shows where the setting sits, and the performance percentages also
  // show the clock that position lands on.
  if (copy.unit === '%') {
    wrap.syncValue = percentSlider(item, copy, wrap, value, onChange);
    return wrap;
  }

  // The CPU's own frequencies, as a ladder of the stops it actually has. A
  // backend that cannot publish one leaves the guarded field behind it, so the
  // row still works without ever accepting a frequency the hardware has not.
  if (copy.ladder === true) {
    if (ladderControl(item, wrap, onChange)) {
      return wrap;
    }
    frequencyField(item, wrap, onChange);
    return wrap;
  }

  if (kind === 'int' || copy.number === true) {
    const line = element('div', 'tlp-inline-value');
    const input = document.createElement('input');
    input.type = 'number';
    input.className = 'tlp-input tlp-number';
    input.step = '1';
    setValue(input, value);
    input.setAttribute('aria-label', item.label + ' — ' + item.sourceLabel);
    if (copy.min !== undefined) {
      input.min = String(copy.min);
    }
    if (copy.max !== undefined) {
      input.max = String(copy.max);
    }
    input.addEventListener('input', () => {
      setPending(item, input.value.trim());
      onChange();
    });
    line.append(input);
    if (copy.unit) {
      line.append(element('span', 'tlp-unit', copy.unit));
    }
    const meanings = copy.meanings || {};
    const note = element('p', 'tlp-note', '');
    const sync = () => {
      setValue(input, currentValue(item));
      const meaning = meanings[input.value.trim()];
      const range =
        copy.min !== undefined || copy.max !== undefined
          ? ' range ' +
            (copy.min === undefined ? '0' : copy.min) +
            ' to ' +
            (copy.max === undefined ? 'any' : copy.max) +
            '.'
          : '';
      note.textContent = (meaning ? meaning + '.' : '') + range;
    };
    input.addEventListener('input', sync);
    sync();
    wrap.append(line, note);
    wrap.syncValue = sync;
    return wrap;
  }

  const input = document.createElement('input');
  input.type = 'text';
  input.className = 'tlp-input is-text';
  setValue(input, value);
  input.spellcheck = false;
  input.placeholder = copy.placeholder || '';
  input.setAttribute('aria-label', item.label + ' — ' + item.sourceLabel);
  input.addEventListener('input', () => {
    setPending(item, input.value.trim());
    onChange();
  });
  wrap.syncValue = () => setValue(input, currentValue(item));
  wrap.append(input);
  if (value === '' && item.baseline !== '') {
    wrap.append(
      element(
        'p',
        'tlp-note is-warn',
        'Empty values are not written; this app keeps an empty field as an empty field.'
      )
    );
  }
  return wrap;
}

/** An inert setting stays on the page, visibly out of reach. */
function disableControl(control) {
  for (const node of control.querySelectorAll('input, select, button')) {
    node.disabled = true;
  }
}

/**
 * The reason every profile of a setting is unreachable, when they share one.
 *
 * TLP gates a whole run of variants on the same hardware, so the same paragraph
 * would otherwise be printed once per profile.
 */
function sharedInertReason(items) {
  if (!items.length || items.some((item) => item.applies)) {
    return '';
  }
  const reason = items[0].unavailable;
  return reason && items.every((item) => item.unavailable === reason) ? reason : '';
}

function buildVariant(item, onValueChange, sharedReason) {
  const row = element('div', 'tlp-variant');
  if (!item.applies) {
    row.classList.add('is-inert');
  }
  row.append(element('span', 'tlp-variant-source', item.sourceLabel));
  const meta = element('div', 'tlp-variant-meta');
  const refresh = () => {
    meta.replaceChildren(...variantMeta(item));
    updateFooter();
    if (onValueChange) {
      onValueChange();
    }
  };
  const control = buildControl(item, refresh);
  if (!item.applies) {
    disableControl(control);
    if (!sharedReason) {
      control.append(
        element(
          'p',
          'tlp-note is-inert',
          item.unavailable ||
            'This setting has nothing to act on here, so TLP ignores it on this machine.'
        )
      );
    }
  }
  row.append(control, meta);
  refresh();
  // A row can be put back to its default from the setting head, so each row
  // knows how to repaint itself from the editor's values.
  row.refreshValue = () => {
    if (control.syncValue) {
      control.syncValue();
    }
    refresh();
  };
  return row;
}

/**
 * Put one setting back to what TLP ships.
 *
 * TLP has a default for some keys and none for others, so "back" is the default
 * when there is one and "not set" when there is not. It only moves the editor's
 * pending values: the operator still presses Preview and Write to make it real.
 */
function buildRevert(items, rows) {
  const button = element('button', 'tlp-revert', '');
  button.type = 'button';
  const live = items.filter((item) => item.applies);
  const named = items.some((item) => item.default !== '');
  const sync = () => {
    button.textContent = named ? 'TLP default' : 'Not set';
    button.title = named
      ? 'Put every profile here back to the value TLP ships by default. Anything TLP ships no default for goes back to not set.'
      : 'Clear this setting. TLP ships no default for it, so TLP then does what it does when the key is absent.';
    button.disabled = !live.length || live.every((item) => currentValue(item) === item.default);
  };
  // Every control change re-runs this, so the button is live exactly when there
  // is something left to put back.
  button.syncState = sync;
  button.addEventListener('click', () => {
    for (const item of live) {
      setPending(item, item.default);
    }
    for (const row of rows) {
      row.refreshValue();
    }
    sync();
    updateFooter();
  });
  sync();
  return button;
}

function buildSetting(chunk, items) {
  const setting = element('article', 'tlp-setting');
  const first = items[0];
  if (items.every((item) => !item.applies)) {
    setting.classList.add('is-inert');
  }
  const head = element('div', 'tlp-setting-head');
  head.append(element('h4', 'tlp-setting-label', chunk.label));
  head.append(element('code', 'tlp-setting-key', chunk.row));
  if (first.deprecated) {
    const badge = element('span', 'tlp-badge is-deprecated', 'removed or renamed');
    badge.title = first.deprecated;
    head.append(badge);
  }
  if (items.some((item) => item.provenance && item.provenance.path === '/etc/tlp.conf')) {
    const badge = element('span', 'tlp-badge is-lost', 'set in /etc/tlp.conf');
    badge.title =
      "Your /etc/tlp.conf is read after this app's drop-in, so those values win and a change written here will not take effect.";
    head.append(badge);
  }
  if (first.doc && first.doc.requires) {
    const badge = element('span', 'tlp-badge', 'needs specific hardware');
    badge.title = 'TLP: ' + first.doc.requires;
    head.append(badge);
  }
  const variants = element('div', 'tlp-variants');
  const rows = [];
  const revert = buildRevert(items, rows);
  const sharedReason = sharedInertReason(items);
  for (const item of items) {
    rows.push(buildVariant(item, () => revert.syncState(), sharedReason));
  }
  variants.append(...rows);
  head.append(revert);
  setting.append(head);
  setting.append(element('p', 'tlp-setting-text', descriptionOf(first)));
  const hint = hintOf(first);
  if (hint) {
    setting.append(element('p', 'tlp-setting-hint', hint));
  }
  // The frequency ladder is one ladder for one row, so what it is gets said
  // once here rather than under each of the three profiles.
  const ladderNote = frequencyLadderNote(rowOf(first));
  if (ladderNote) {
    setting.append(element('p', 'tlp-note is-ladder', ladderNote));
  }
  if (sharedReason) {
    setting.append(element('p', 'tlp-note is-inert', sharedReason));
  }
  setting.append(variants);
  return setting;
}

function filterChunk(chunk) {
  const filter = editor.filter;
  const words = filter.search ? filter.search.toLowerCase().split(/\s+/).filter(Boolean) : [];
  const kept = chunk.items.filter((item) => matchesFilters(item, words));
  if (kept.length) {
    return kept;
  }
  if (
    words.length &&
    entryMatchesSearch(
      { key: chunk.row, row: chunk.row, label: chunk.label, doc: chunk.items[0].doc },
      words
    )
  ) {
    // The search hit the setting as a whole (its name or description), so every
    // variant of it is relevant, not just the one whose key matched.
    return chunk.items.filter((item) => {
      if (filter.available && item.deprecated) {
        return false;
      }
      return !filter.changed || isDirty(item);
    });
  }
  return [];
}

function renderEditor() {
  const groupsHost = document.getElementById('tlp-groups');
  if (!groupsHost) {
    return;
  }
  groupsHost.replaceChildren();
  const chunks = new Map();
  for (const item of editor.entries) {
    let chunk = chunks.get(item.row);
    if (!chunk) {
      chunk = { row: item.row, label: item.label, items: [] };
      chunks.set(item.row, chunk);
    }
    chunk.items.push(item);
  }

  let shownSettings = 0;
  let shownEntries = 0;
  for (const id of GROUP_ORDER) {
    const groupChunks = [];
    for (const chunk of chunks.values()) {
      if (chunk.items[0].group !== id) {
        continue;
      }
      const kept = filterChunk(chunk);
      if (kept.length) {
        groupChunks.push({ chunk, kept });
      }
    }
    if (!groupChunks.length) {
      continue;
    }
    const details = element('details', 'tlp-group');
    details.open = editor.filter.search ? true : !editor.collapsed.has(id);
    const summary = element('summary', 'tlp-group-head');
    summary.append(element('h3', 'tlp-group-title', GROUP_TITLES[id] || id));
    summary.append(
      element(
        'span',
        'tlp-group-count',
        groupChunks.length === 1 ? '1 setting' : groupChunks.length + ' settings'
      )
    );
    details.append(summary);
    if (GROUP_BLURB[id]) {
      details.append(element('p', 'tlp-group-blurb', GROUP_BLURB[id]));
    }
    const body = element('div', 'tlp-settings');
    for (const { chunk, kept } of groupChunks) {
      body.append(buildSetting(chunk, kept));
    }
    details.append(body);
    details.addEventListener('toggle', () => {
      if (details.open) {
        editor.collapsed.delete(id);
      } else {
        editor.collapsed.add(id);
      }
    });
    groupsHost.append(details);
    shownSettings += groupChunks.length;
    shownEntries += groupChunks.reduce((sum, entry) => sum + entry.kept.length, 0);
  }

  if (!shownSettings) {
    groupsHost.append(
      element(
        'p',
        'tlp-empty',
        editor.filter.search
          ? 'Nothing matches "' +
              editor.filter.search +
              '". Try a shorter word, or clear the filters.'
          : 'Nothing to show with these filters.'
      )
    );
  }
  updateStatus(shownSettings, shownEntries);
}

function updateStatus(shownSettings, shownEntries) {
  const status = document.getElementById('tlp-status');
  if (!status) {
    return;
  }
  const parameters = editor.entries.length;
  const settings = new Set(editor.entries.map((item) => item.row)).size;
  const parts = [
    'showing ' +
      shownSettings +
      ' of ' +
      settings +
      ' settings (' +
      shownEntries +
      ' of ' +
      parameters +
      ' TLP parameters)'
  ];
  const own = editor.entries.filter(isOurs).length;
  const yours = editor.entries.filter(
    (item) => item.provenance && item.provenance.path === '/etc/tlp.conf'
  ).length;
  const gone = editor.entries.filter((item) => item.deprecated).length;
  parts.push(
    own
      ? own + (own === 1 ? ' parameter comes' : ' parameters come') + ' from this app drop-in'
      : 'this app drop-in is empty'
  );
  if (yours) {
    parts.push(yours + ' already set in /etc/tlp.conf, which wins over this app');
  }
  if (gone) {
    parts.push(gone + ' removed or renamed by this TLP version');
  }
  const inert = editor.entries.filter((item) => !item.applies).length;
  if (inert) {
    parts.push(inert + ' do not apply on this machine, so TLP ignores them');
  }
  status.textContent = parts.join(' · ') + '.';
}

function changeList() {
  return editor.entries
    .filter((item) => isDirty(item) && currentValue(item) !== '')
    .map((item) => ({ key: item.key, value: currentValue(item) }));
}

function updateFooter() {
  const footer = document.getElementById('tlp-footer');
  const count = document.getElementById('tlp-footer-count');
  if (!footer) {
    return;
  }
  const changes = changeList().length;
  const empty = editor.entries.filter((item) => isDirty(item) && currentValue(item) === '').length;
  footer.hidden = editor.tab !== 'settings' || (changes === 0 && empty === 0);
  if (count) {
    const text = changes === 1 ? '1 change' : changes + ' changes';
    count.textContent = empty
      ? text + ' (' + empty + ' with an empty value, which is not written)'
      : text;
  }
  markPreviewStale();
}
/** A path that TLP reads after this app's drop-in, and therefore beats it. */
function losesToApp(path) {
  if (!path) {
    return false;
  }
  if (path === '/etc/tlp.conf') {
    return true;
  }
  if (path.indexOf('/etc/tlp.d/') === 0) {
    return path.slice('/etc/tlp.d/'.length) > DROPIN_NAME;
  }
  return false;
}

function augmentEntries(schema, config) {
  const values = (config && config.values) || {};
  const provenance = (config && config.provenance) || {};
  const entries = schema && Array.isArray(schema.entries) ? schema.entries : [];
  return entries.map((entry) => {
    const source = provenance[entry.key] || null;
    const assigned = Object.prototype.hasOwnProperty.call(values, entry.key);
    const effective =
      assigned && values[entry.key] !== null && values[entry.key] !== undefined
        ? String(values[entry.key])
        : '';
    const kind = entry.kind || { kind: 'text' };
    const row = rowOf(entry);
    return {
      entry,
      key: entry.key,
      row,
      group: displayGroup(entry),
      label: humanLabel(row),
      sourceLabel: SOURCE_LABELS[entry.source] || 'All profiles',
      kind: kindOf(entry),
      enumValues: Array.isArray(kind.values) ? kind.values : null,
      default:
        entry.default_value === null || entry.default_value === undefined
          ? ''
          : String(entry.default_value),
      doc: entry.doc || null,
      deprecated: entry.deprecated || null,
      // Absent fields mean an older backend that knew nothing about
      // applicability: everything applies until it says otherwise.
      applies: entry.applies !== false,
      unavailable:
        typeof entry.unavailable === 'string' && entry.unavailable.trim()
          ? entry.unavailable.trim()
          : '',
      requires: entry.doc && entry.doc.requires ? entry.doc.requires : null,
      effective,
      provenance: source,
      baseline: effective
    };
  });
}

/**
 * Read the schema and the effective configuration, and reseed the editor.
 *
 * `keepEdits` preserves what the operator has typed for the rows they touched,
 * so a diagnosis re-check does not throw away an unfinished edit.
 */
async function loadEditorData(options) {
  const keepEdits = !options || options.keepEdits !== false;
  const previousValues = editor.values;
  const previousBaseline = editor.baseline;
  const wasEdited = (key) =>
    previousValues.has(key) && previousValues.get(key) !== previousBaseline.get(key);

  const token = ++editor.request;
  const answers = await Promise.all([invoke('tlp_schema'), invoke('tlp_config')]);
  if (token !== editor.request) {
    return;
  }
  const schema = unwrap(answers[0]);
  const config = unwrap(answers[1]);

  editor.entries = augmentEntries(schema, config);
  editor.byKey = new Map(editor.entries.map((item) => [item.key, item]));

  const values = new Map();
  const baseline = new Map();
  for (const item of editor.entries) {
    baseline.set(item.key, item.baseline);
    values.set(
      item.key,
      keepEdits && wasEdited(item.key) ? previousValues.get(item.key) : item.baseline
    );
  }
  editor.values = values;
  editor.baseline = baseline;
  editor.ready = true;

  const panel = document.getElementById('tlp-editor-panel');
  const toolbar = document.getElementById('tlp-toolbar');
  const warnings = document.getElementById('tlp-warnings');
  const status = document.getElementById('tlp-status');
  if (panel) {
    panel.setAttribute('aria-busy', 'false');
  }

  if (schema && schema.available === false) {
    if (toolbar) {
      toolbar.hidden = true;
    }
    const host = document.getElementById('tlp-groups');
    if (host) {
      host.replaceChildren();
    }
    if (warnings) {
      warnings.hidden = true;
      warnings.replaceChildren();
    }
    if (status) {
      status.hidden = false;
      status.classList.add('tlp-unavailable');
      status.textContent =
        schema.reason ||
        'TLP is not installed on this machine, so there is nothing to configure here. The diagnosis above says what is managing power instead.';
    }
    updateFooter();
    return;
  }

  if (status) {
    status.hidden = false;
    status.classList.remove('tlp-unavailable');
  }
  if (toolbar) {
    toolbar.hidden = false;
  }

  const notices = [];
  if (schema && schema.reason) {
    notices.push(schema.reason + '.');
  }
  for (const warning of (config && config.warnings) || []) {
    notices.push(
      warning.key +
        ' in ' +
        warning.path +
        ':' +
        warning.line +
        ' — ' +
        warning.message +
        (warning.replacement ? ' Use ' + warning.replacement + ' instead.' : '')
    );
  }
  if (warnings) {
    warnings.hidden = notices.length === 0;
    warnings.replaceChildren(...notices.map((text) => element('li', '', text)));
  }

  renderEditor();
  updateFooter();
}

/** Fetch the schema and configuration once, or again on demand. */
async function loadEditor(force) {
  if (editor.loading) {
    return;
  }
  if (editor.ready && !force) {
    return;
  }
  const status = document.getElementById('tlp-status');
  const panel = document.getElementById('tlp-editor-panel');
  editor.loading = true;
  if (panel) {
    panel.setAttribute('aria-busy', 'true');
  }
  if (status) {
    status.hidden = false;
    status.classList.remove('tlp-unavailable');
    status.textContent = editor.entries.length
      ? 'Reloading what the installed TLP ships…'
      : 'Reading the settings the installed TLP ships…';
  }
  try {
    // The percentage sliders need the hardware's clock range, which the
    // performance view reads with the same command. Without it they still work;
    // they just have no frequency to show.
    await loadClockLimits();
    await loadEditorData({ keepEdits: true });
  } catch (error) {
    if (status) {
      status.classList.add('tlp-unavailable');
      status.textContent = 'The TLP settings could not be read: ' + errorText(error);
    }
    if (panel) {
      panel.setAttribute('aria-busy', 'false');
    }
  } finally {
    editor.loading = false;
  }
}

// --- Review, apply, roll back ------------------------------------------------

function reviewRefs() {
  const panel = document.getElementById('tlp-review-panel');
  const actions = document.getElementById('tlp-review-actions');
  if (!panel || !actions) {
    return null;
  }
  return {
    panel,
    actions,
    title: document.getElementById('tlp-review-title'),
    note: document.getElementById('tlp-review-note'),
    list: document.getElementById('tlp-change-list'),
    fileWrap: document.getElementById('tlp-review-file-wrap'),
    file: document.getElementById('tlp-review-file')
  };
}

function changeItem(options) {
  const item = element('li', 'tlp-change' + (options.className ? ' ' + options.className : ''));
  item.append(element('span', 'tlp-change-label', options.label));
  item.append(element('code', 'tlp-change-key', options.key));
  item.append(element('span', 'tlp-change-value', options.value));
  if (options.reason) {
    item.append(element('p', 'tlp-change-reason', options.reason));
  }
  return item;
}

/** What the backend says about a key another file already owns, in plain words. */
function overrideNote(override) {
  const path = override.path;
  if (path === '/etc/tlp.conf') {
    return (
      'Your /etc/tlp.conf sets ' +
      override.key +
      ' on line ' +
      override.line +
      ', and TLP reads that file after this app drop-in: your value there wins. Comment that line out to let this app decide.'
    );
  }
  if (path.indexOf('/etc/tlp.d/') === 0) {
    const name = path.slice('/etc/tlp.d/'.length);
    return losesToApp(path)
      ? name + ' is read after this app file and wins over it.'
      : name + ' is read before this app file, so the value written here wins.';
  }
  if (/defaults\.conf$/.test(path)) {
    return 'TLP ships this one as a default; this app file is read after it, so the value written here replaces it.';
  }
  return 'Currently set in ' + path + '.';
}

function setReviewActions(refs, actions) {
  refs.actions.replaceChildren(
    ...actions.map((action) => {
      const button = element(
        'button',
        'power-action-btn' + (action.primary ? ' tlp-btn-primary' : ''),
        action.label
      );
      button.type = 'button';
      if (action.disabled) {
        button.disabled = true;
        button.title = action.disabledReason || '';
      }
      button.addEventListener('click', () => {
        action.run(button);
      });
      return button;
    })
  );
}

function setReviewBusy(busy) {
  const refs = reviewRefs();
  if (!refs) {
    return;
  }
  refs.panel.setAttribute('aria-busy', busy ? 'true' : 'false');
  for (const button of refs.actions.querySelectorAll('button')) {
    button.disabled = busy;
  }
}

function fingerprint(changes) {
  return changes
    .map((change) => change.key + '=' + change.value)
    .sort()
    .join(';');
}

function hideReview() {
  const refs = reviewRefs();
  editor.review = null;
  if (!refs) {
    return;
  }
  refs.panel.hidden = true;
  refs.note.classList.remove('is-warn');
  refs.list.replaceChildren();
}

function setReviewNote(text, warning) {
  const refs = reviewRefs();
  if (!refs) {
    return;
  }
  refs.note.textContent = text;
  refs.note.classList.toggle('is-warn', Boolean(warning));
}

function isPromptDismissed(text) {
  return /dismiss|not authorized|cancel|dibatalkan/i.test(text || '');
}

function renderPreview(preview, requested) {
  const refs = reviewRefs();
  if (!refs) {
    return;
  }
  editor.review = { kind: 'preview', changes: requested, print: fingerprint(requested) };
  refs.panel.hidden = false;
  refs.title.textContent = 'Review before writing';
  refs.list.replaceChildren();
  refs.fileWrap.hidden = true;

  if (!preview.available) {
    setReviewNote(
      preview.reason || 'These values cannot be checked against the installed TLP.',
      true
    );
    setReviewActions(refs, [{ label: 'Keep editing', run: hideReview }]);
    return;
  }

  const overrideByKey = new Map((preview.overridden || []).map((row) => [row.key, row]));
  for (const change of preview.accepted) {
    const item = editor.byKey.get(change.key);
    const override = overrideByKey.get(change.key);
    const lost = override ? losesToApp(override.path) : false;
    refs.list.append(
      changeItem({
        label: item ? item.label + ' — ' + item.sourceLabel : change.key,
        key: change.key,
        value: (item ? displayValue(item, item.effective) : 'not set') + '  →  ' + change.value,
        reason: override ? overrideNote(override) : '',
        className: lost ? 'is-lost' : ''
      })
    );
  }
  for (const rejection of preview.rejected) {
    const item = editor.byKey.get(rejection.key);
    refs.list.append(
      changeItem({
        label: item ? item.label + ' — ' + item.sourceLabel : rejection.key,
        key: rejection.key,
        value: rejection.value === '' ? '(empty)' : rejection.value,
        reason: 'TLP refused this value: ' + rejection.reason + '.',
        className: 'is-rejected'
      })
    );
  }

  const parts = [];
  if (preview.accepted.length) {
    parts.push(
      preview.accepted.length +
        ' of ' +
        requested.length +
        ' settings will be written to ' +
        preview.path +
        '. TLP restarts afterwards, which is what makes them take effect.'
    );
  } else {
    parts.push('Nothing can be written here.');
  }
  if (preview.rejected.length) {
    parts.push(
      preview.rejected.length === 1
        ? 'TLP refused 1 of the values.'
        : 'TLP refused ' + preview.rejected.length + ' of the values.'
    );
  }
  if (preview.overridden && preview.overridden.length) {
    parts.push(
      preview.overridden.length +
        ' of them belong to another configuration file: those rows say which.'
    );
  }
  setReviewNote(parts.join(' '), preview.accepted.length === 0);

  if (preview.content) {
    refs.file.textContent = preview.content;
    refs.fileWrap.hidden = false;
  }

  setReviewActions(refs, [
    { label: 'Keep editing', run: hideReview },
    {
      label: 'Write these settings',
      primary: true,
      disabled: preview.accepted.length === 0,
      disabledReason: 'Nothing was accepted, so there is nothing to write.',
      run: (button) => {
        void applyChanges(button, preview.accepted);
      }
    }
  ]);
}

function renderApplied(applied) {
  const refs = reviewRefs();
  if (!refs) {
    return;
  }
  editor.review = { kind: 'applied', changes: applied.applied };
  refs.panel.hidden = false;
  refs.title.textContent = 'Written to TLP';
  refs.list.replaceChildren();
  refs.fileWrap.hidden = true;

  const parts = [
    applied.applied.length +
      ' settings written to ' +
      applied.path +
      '.' +
      (applied.tlp_restarted
        ? ' TLP was restarted, so they are in effect now.'
        : ' TLP is not running, so they take effect the next time it starts.')
  ];
  if (applied.backup) {
    parts.push('The previous file is kept as ' + applied.backup + '.');
  }
  if (applied.note) {
    parts.push(applied.note);
  }
  setReviewNote(parts.join(' '), false);

  const overrideByKey = new Map((applied.overridden || []).map((row) => [row.key, row]));
  for (const change of applied.applied) {
    const item = editor.byKey.get(change.key);
    const override = overrideByKey.get(change.key);
    const lost = override ? losesToApp(override.path) : false;
    refs.list.append(
      changeItem({
        label: item ? item.label + ' — ' + item.sourceLabel : change.key,
        key: change.key,
        value: change.value,
        reason: override ? overrideNote(override) : '',
        className: lost ? 'is-lost' : ''
      })
    );
  }
  for (const rejection of applied.rejected) {
    const item = editor.byKey.get(rejection.key);
    refs.list.append(
      changeItem({
        label: item ? item.label + ' — ' + item.sourceLabel : rejection.key,
        key: rejection.key,
        value: rejection.value === '' ? '(empty)' : rejection.value,
        reason: 'TLP refused this value: ' + rejection.reason + '.',
        className: 'is-rejected'
      })
    );
  }

  setReviewActions(refs, [
    {
      label: 'Roll back these settings',
      run: (button) => {
        void rollbackDropin(button);
      }
    },
    { label: 'Done', primary: true, run: hideReview }
  ]);
}

async function previewChanges(trigger) {
  const changes = changeList();
  if (!changes.length) {
    toast('Nothing to preview', {
      kind: 'info',
      detail: 'Change a setting first, or press Revert to go back to what is in effect.'
    });
    return;
  }
  try {
    await runWithState(
      trigger,
      async () => {
        const preview = unwrap(await invoke('power_preview_changes', { changes }));
        renderPreview(preview, changes);
      },
      { success: '', title: 'The preview could not be prepared' }
    );
  } catch {
    /* reported by runWithState */
  }
}

/**
 * Write the changes. The privileged step raises a PolicyKit password prompt, so
 * a dismissed prompt is reported as "nothing was written" rather than as a
 * failure, and success is only claimed once the backend says so.
 */
async function applyChanges(trigger, changes) {
  if (!changes.length) {
    return;
  }
  setReviewNote(
    'Writing ' +
      changes.length +
      ' settings. PolicyKit asks for your password first, then TLP is restarted.',
    false
  );
  setReviewBusy(true);
  editor.writing = true;
  try {
    const applied = unwrap(await invoke('power_apply_changes', { changes }));
    renderApplied(applied);
    await loadEditorData({ keepEdits: false });
    await refreshPower();
    toast('Settings applied', {
      kind: 'ok',
      detail: applied.applied.length + ' written to ' + applied.path
    });
  } catch (error) {
    const text = errorText(error);
    setReviewNote(
      isPromptDismissed(text)
        ? 'Nothing was written: the password prompt was dismissed. Press Write these settings again when you are ready.'
        : 'Nothing was written: ' + text,
      true
    );
  } finally {
    editor.writing = false;
    setReviewBusy(false);
  }
}

async function rollbackDropin(trigger) {
  const previous = trigger ? trigger.textContent : '';
  setReviewBusy(true);
  editor.writing = true;
  try {
    if (trigger) {
      trigger.textContent = 'Rolling back…';
    }
    const message = unwrap(await invoke('power_rollback_dropin'));
    await loadEditorData({ keepEdits: false });
    await refreshPower();
    hideReview();
    toast('Rolled back', { kind: 'ok', detail: message });
  } catch (error) {
    const text = errorText(error);
    setReviewNote(
      isPromptDismissed(text)
        ? 'Nothing was changed: the password prompt was dismissed.'
        : 'The rollback did not run: ' + text,
      true
    );
    if (trigger) {
      trigger.textContent = previous;
    }
  } finally {
    editor.writing = false;
    setReviewBusy(false);
  }
}

function revertAll() {
  for (const item of editor.entries) {
    editor.values.set(item.key, item.baseline);
  }
  hideReview();
  renderEditor();
  updateFooter();
}

// --- Wiring ------------------------------------------------------------------

function selectPowerTab(id) {
  editor.tab = id;
  for (const button of document.querySelectorAll('.power-tab[data-power-tab]')) {
    const active = button.dataset.powerTab === id;
    button.classList.toggle('is-active', active);
    button.setAttribute('aria-selected', active ? 'true' : 'false');
    button.tabIndex = active ? 0 : -1;
  }
  const diagnosis = document.getElementById('power-tab-diagnosis');
  const settings = document.getElementById('power-tab-settings');
  if (diagnosis) {
    diagnosis.hidden = id !== 'diagnosis';
  }
  if (settings) {
    settings.hidden = id !== 'settings';
  }
  updateFooter();
  if (id === 'settings') {
    void loadEditor(false);
  }
}

function bindPowerTabs() {
  const tabs = Array.from(document.querySelectorAll('.power-tab[data-power-tab]'));
  for (const tab of tabs) {
    if (tab.dataset.bound) {
      continue;
    }
    tab.dataset.bound = 'true';
    tab.addEventListener('click', () => {
      selectPowerTab(tab.dataset.powerTab);
    });
    tab.addEventListener('keydown', (event) => {
      if (event.key !== 'ArrowRight' && event.key !== 'ArrowLeft') {
        return;
      }
      event.preventDefault();
      const other = tabs.find((candidate) => candidate.dataset.powerTab !== editor.tab);
      if (other) {
        selectPowerTab(other.dataset.powerTab);
        other.focus();
      }
    });
  }
}

function bindEditorControls() {
  let searchTimer = null;
  const search = document.getElementById('tlp-search');
  if (search && !search.dataset.bound) {
    search.dataset.bound = 'true';
    search.addEventListener('input', () => {
      const value = search.value.trim();
      // Rebuilding the list on every keystroke is visible on a slow machine;
      // one frame and a bit of breathing room is imperceptible.
      if (searchTimer) {
        clearTimeout(searchTimer);
      }
      searchTimer = setTimeout(() => {
        editor.filter.search = value;
        renderEditor();
      }, 120);
    });
  }
  const changed = document.getElementById('tlp-filter-changed');
  if (changed && !changed.dataset.bound) {
    changed.dataset.bound = 'true';
    changed.addEventListener('change', () => {
      editor.filter.changed = changed.checked;
      renderEditor();
    });
  }
  const available = document.getElementById('tlp-filter-available');
  if (available && !available.dataset.bound) {
    available.dataset.bound = 'true';
    available.addEventListener('change', () => {
      editor.filter.available = available.checked;
      renderEditor();
    });
  }
  const reload = document.getElementById('btn-tlp-reload');
  if (reload && !reload.dataset.bound) {
    reload.dataset.bound = 'true';
    reload.addEventListener('click', async () => {
      try {
        await runWithState(reload, () => loadEditor(true), {
          success: 'Settings reloaded',
          title: 'Could not reload the TLP settings'
        });
      } catch {
        /* reported by runWithState */
      }
    });
  }
  const preview = document.getElementById('btn-tlp-preview');
  if (preview && !preview.dataset.bound) {
    preview.dataset.bound = 'true';
    preview.addEventListener('click', () => {
      void previewChanges(preview);
    });
  }
  const revert = document.getElementById('btn-tlp-revert');
  if (revert && !revert.dataset.bound) {
    revert.dataset.bound = 'true';
    revert.addEventListener('click', revertAll);
  }
  const cancel = document.getElementById('btn-tlp-cancel');
  if (cancel && !cancel.dataset.bound) {
    cancel.dataset.bound = 'true';
    cancel.addEventListener('click', hideReview);
  }
}

/** Called once per view activation; the editor itself is loaded on demand. */
function setupTlpEditor() {
  bindPowerTabs();
  bindEditorControls();
  selectPowerTab(editor.tab);
}

function markPreviewStale() {
  if (!editor.review || editor.review.kind !== 'preview') {
    return;
  }
  if (editor.review.print !== fingerprint(changeList())) {
    setReviewNote(
      'The settings have changed since this preview. Press Preview again to check the new values.',
      true
    );
  }
}
export async function loadPowerView() {
  setupTlpEditor();

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
