# System Overview

Home answers two questions: what this machine is, and what it is doing right now.
No settings live here — those are on the pages named after them.

## Identity

The model name is the largest text in the application, with the machine type code
and the BIOS version under it. The serial number is shown next to a Copy button.

The serial is mode 0400 root-only in sysfs, so the app does not read it at startup.
It is fetched the first time you press Copy, through the privileged helper.

## Each card

| Card | What it shows |
| --- | --- |
| Processor | Governor, current clock against the maximum, core and thread count, package temperature |
| Graphics | Driver, clock against the maximum, busy percentage, temperature, power, VRAM, and the driver's performance level when it exposes one |
| Memory | Used and available, swap, the usage bar, and the module speed and part numbers when the memory table can be read |
| Storage | Capacity, firmware, temperature, read and write rates, and SMART wear, spare, written bytes and error counts |
| Power | The active profile, charger state, charging or discharging watts, current, voltage, energy, runtime or time to full, and CPU package power |

## Devices are found, not assumed

The graphics card is located by looking for a card whose driver is `amdgpu`, `i915`
or `xe` — on this class of machine it is not necessarily `card0` — and its name comes
from the PCI database, preferring the marketing name when the database keeps one in
brackets.

## What needs the privileged helper

Most of the page is read straight from sysfs, `/proc` and the hwmon devices, with no
privileges at all. Four readings are mode 0400 root-only and go through the helper:

| Reading | Why it is restricted |
| --- | --- |
| Machine serial | DMI identity files are root-only |
| CPU package power | The RAPL energy counter is root-only |
| Memory speed and part numbers | `dmidecode -t 17` needs root |
| Storage health | `smartctl` needs root |

The helper answers these with fixed commands and takes no argument from the caller
beyond its exact-match whitelist. When it is not installed, the affected rows say so
instead of showing a zero, and the Power page explains what to do about it.

## Rates need two samples

Read and write rates, and CPU package power, are differences between two counters.
The first sample only establishes a baseline, so those rows stay empty for one poll
and fill in on the next.

## Polling

The overview is read every two seconds while the page is visible and stops when you
navigate away. CPU package power is sampled every fifteen seconds because each sample
costs a privileged call.
