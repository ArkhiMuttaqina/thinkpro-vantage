# Power

Power is the page that explains why a setting will or will not take effect, and where
you change it. It is built on two backends: an arbiter that reports who currently owns
each knob, and a diagnosis that reports one finding per thing in the way.

## Who owns what

Several daemons can claim the same hardware: TLP, power-profiles-daemon, `lactd` and
this application itself. When two of them own the same knob, nothing written by either
sticks for long. The first panel lists every claim and marks the conflicts, ordered by
how much they matter:

| Severity | Meaning |
| --- | --- |
| Blocking | Two owners for one knob; writes from this app will not survive |
| Warning | Values will drift, or a feature is unavailable |
| Info | Worth knowing |

## Diagnosis

Each finding names one problem and carries the remedy that applies to it. Remedies come
in two kinds, and the difference is deliberate:

- **Action** — something this application may do through the privileged helper, such as
  installing it or writing the `fan_control=1` module parameter. Pressing the button runs
  it.
- **Command** — something that belongs to the administrator, such as installing TLP or
  restarting a service. The app shows the command, and pressing the button opens your
  terminal with it already typed, so the sudo prompt is yours and the output stays on
  screen. Clicking the command itself copies it instead. Nothing is ever run silently.

The distribution is read from `/etc/os-release`, following the `ID_LIKE` chain, so a
derivative such as CachyOS resolves through `arch` to pacman. The detected name is shown
next to the capability, and the install command is built for that package manager —
`pacman`, `apt`, `dnf` or `zypper` — with the package names each one uses.

A finding that the firmware simply cannot support, like charge thresholds on a pack that
 does not expose them, is a notice rather than a problem.

## Capability

| Capability | What is available |
| --- | --- |
| TLP in charge | The full settings editor |
| power-profiles-daemon only | Three profiles and its own actions; TLP settings cannot be expressed |
| direct control only | Individual settings through the helper, but no per-source configuration |

## TLP settings editor

The second tab is the editor: every parameter the installed TLP ships, grouped by TLP's
own sections — general, CPU, platform, graphics, disks, PCIe, USB, radios, audio, battery
care. Each group carries one line saying what it is for; each setting carries what it
changes in plain language, what it currently is, and where that value comes from.

- The control follows the setting: a switch for a yes/no parameter, a list for one with
  fixed values, a number with its unit and range for a percentage, a frequency or a
  timeout, and a text field only where TLP really accepts free text — a device list, a
  path, a set of colors.
- A badge names the file a value comes from whenever that is not the app's own drop-in:
  `TLP default`, `00-template.conf:3`, `/etc/tlp.conf:258`.
- Settings that already come from `/etc/tlp.conf` are marked, because that file is read
  after the drop-in: the app would write the value and TLP would not use it. The row says
  so before you change anything, and the preview repeats it with the file and line.
- Search matches the key, the name and the description. **Only changed** narrows the list
  to what you have touched. **Only settings this TLP accepts** hides parameters the
  installed TLP has removed or renamed, which it would refuse to write anyway.

Nothing is written until you press **Preview** and then confirm. The preview returns what
would be written, what TLP refused and why, and which of the changes another file owns —
per change, not as one summary. Applying raises a PolicyKit prompt and restarts TLP;
dismissing that prompt is reported as "nothing was written" rather than as a failure, and
the previous drop-in stays available for **Roll back these settings**.

Two parameters are compared by TLP against the letters `y` and `n` instead of `0` and
`1`: `WOL_DISABLE` and `SOUND_POWER_SAVE_CONTROLLER`. The writer accepts only `0`/`1` for
switches, so the editor refuses a change to those two and says why on the row. Set them in
`/etc/tlp.conf`.

## Writing settings

The editor writes exactly one file, `/etc/tlp.d/99-thinkpro-vantage.conf`. It never
touches `/etc/tlp.conf`, which belongs to you, and deleting that one file — or pressing
Roll back — undoes everything the application ever set.

Pressing Preview runs three checks against the installed TLP, and the result is shown per
change before you confirm:

1. The key exists in the schema of the TLP you actually have installed; renamed and
   deprecated keys are refused with the reason.
2. The value matches the type the key declares: a switch takes `0` or `1`, a number takes
   a number, a choice takes one of its listed values.
3. The value uses only letters, digits, spaces and `_ , . + : % - / ( )`. A newline, a
   quote, a backtick or a `#` cannot appear, so a value can never become a second setting.

Preview shows **the whole file** that is about to be installed, not just the settings you
just changed. A write merges: the keys the file already holds are kept, and only the ones
you name are overwritten. A key the application is not told about is never deleted, so
changing one setting cannot lose the others.

Leaving a key out and clearing it are different things. A key you leave out keeps the
value it has; a key sent with an empty value is removed from the file, which puts TLP's
own default back in charge of it.

## TLP reads your file last

TLP reads `/usr/share/tlp/defaults.conf`, then `/etc/tlp.d/*.conf`, then
`/etc/tlp.conf`. A key you set in `/etc/tlp.conf` therefore wins over the same key in the
application's drop-in. The app checks this rather than assuming: after writing, it
re-reads the merged configuration and reports every key that another file still owns,
with the file and line number.

## Rolling back

Roll back restores the previous version of the drop-in, or removes it when the app had
never written one, then restarts TLP if it is running. The previous version is kept
beside the file as `99-thinkpro-vantage.conf.bak`, which TLP does not read.

Merging does not make that backup redundant: a write still replaces every value it names,
and Roll back is the only way back to the values it replaced.
