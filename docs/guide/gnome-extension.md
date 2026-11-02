# GNOME Shell Extension

Live CPU temperature, fan speed and battery state in the top bar, plus quick fan
controls, without keeping the window open.

## What it shows

The indicator sits on the right-hand side of the top bar:

- CPU temperature, read from `/proc/acpi/ibm/thermal`, falling back to
  `/sys/class/thermal/thermal_zone0/temp` on machines without the ThinkPad ACPI
  interface
- Fan speed and mode, read from `/proc/acpi/ibm/fan`
- Battery percentage and state, read from `/sys/class/power_supply/`

None of that needs privileges. Opening the menu adds the same readings plus the
quick fan controls — Auto, Level 1 (quiet), Level 4 (medium), Level 7 (high) and
Full speed — and entries that open or refresh the application.

## Supported GNOME Shell versions

**45 through 51.**

GNOME refuses to load an extension whose `metadata.json` does not list the
running shell version, and the refusal is quiet:
`gnome-extensions info thinkprovantage@arkhimuttaqina.my.id` reports
`State: OUT OF DATE` and the indicator simply never appears.

The advertised range lives in exactly one place,
`gnome-extension/thinkprovantage@arkhimuttaqina.my.id/metadata.json`. The
application embeds that same file when it installs the extension, so the
in-app path and the script path cannot drift apart.

Extending the range is a deliberate change, not a version bump: the API surface
the extension relies on — the `gi://` and `resource://` module imports,
`PanelMenu.Button`, `St` layout, `PopupMenu` items — has to be checked against
the new shell first. A test pins the range so it cannot be widened by accident.

## Installing

From the application: **Settings → GNOME Shell Extension → Install**.

From a source checkout:

```bash
./gnome-extension/install.sh
```

Either way the files land in
`~/.local/share/gnome-shell/extensions/thinkprovantage@arkhimuttaqina.my.id/`.

Then **log out and back in**. A running GNOME Shell on Wayland cannot reload an
extension in place, so the indicator only appears in a new session. To confirm it
loaded:

```bash
gnome-extensions info thinkprovantage@arkhimuttaqina.my.id
```

`State: ENABLED` or `State: ACTIVE` means it is running; `OUT OF DATE` means the
metadata does not list your shell version.

## Fan control needs the privileged helper

Reading sensors is unprivileged; changing fan speed is not, so the menu goes
through the same helper the application uses:

```bash
pkexec /usr/lib/thinkpro-vantage/thinkpro-ctl "level 4"
pkexec /usr/lib/thinkpro-vantage/thinkpro-ctl "watchdog 30"
```

The helper accepts an exact whitelist — `level auto`, `level full-speed`,
`level 0` through `level 7`, and `watchdog 30` — and nothing else, so no
arbitrary command can be smuggled through it. While a manual level is held the
extension re-asserts it and refreshes the firmware watchdog, so an embedded
controller reset cannot silently hand control back to automatic.

## Uninstalling

Use the Remove button in the application, or from a source checkout:

```bash
./gnome-extension/install.sh --uninstall
```

Log out and back in to finish.
