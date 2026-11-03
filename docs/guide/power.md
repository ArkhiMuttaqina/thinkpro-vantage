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

## Writing settings

The editor writes exactly one file, `/etc/tlp.d/99-thinkpro-vantage.conf`. It never
touches `/etc/tlp.conf`, which belongs to you, and deleting that one file — or pressing
Roll back — undoes everything the application ever set.

Three checks run before anything is written:

1. The key exists in the schema of the TLP you actually have installed; renamed and
   deprecated keys are refused with the reason.
2. The value matches the type the key declares: a switch takes `0` or `1`, a number takes
   a number, a choice takes one of its listed values.
3. The value uses only letters, digits, spaces and `_ , . + : % - / ( )`. A newline, a
   quote, a backtick or a `#` cannot appear, so a value can never become a second setting.

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
