# The CrownOS session

`crownpositor` is the session — it owns the Wayland socket and spawns the rest
of the desktop. But a compositor binary on `$PATH` is not yet something a login
screen can offer, and until this directory existed there was no way to *log in
to* CrownOS at all: no `wayland-sessions` entry, no launcher, and a
`compositor.startup` list that defaults to empty, so a fresh install came up as
a bare compositor with no bar, no dock and no notifications.

| File | What it is |
|---|---|
| `crownos.desktop` | The `wayland-sessions` entry. GDM, SDDM, greetd, LightDM and ly all read this directory; it is how CrownOS appears in the list. |
| `crownos-session` | What the entry runs. Sets `XDG_CURRENT_DESKTOP` and the toolkit variables, makes sure there is a session bus, then execs `crownpositor`. |
| `compositor.example.ron` | A `compositor.ron` with a populated `startup` list — the thing that turns a compositor into a desktop. |
| `install.sh` | Puts the three where they belong. |

## Install

```bash
./install.sh              # ~/.local, this user, no root
sudo ./install.sh --system   # /usr/local, every user -- more reliable for greeters
./install.sh --uninstall
```

It installs `compositor.ron` only if you do not already have one; your settings
are never overwritten.

## Why a launcher script and not `Exec=crownpositor`

Three things have to happen before the compositor starts, and none of them are
its job:

1. **`XDG_CURRENT_DESKTOP=CrownOS`** — xdg-desktop-portal picks its backend from
   this. Without it, file pickers and screen sharing resolve to whatever is
   installed first.
2. **A session bus.** `crownotify` registers `org.freedesktop.Notifications`.
   Some greeters hand you a session with no bus at all, so the launcher runs
   `dbus-run-session` when `DBUS_SESSION_BUS_ADDRESS` is unset.
3. **Toolkit variables** — `MOZ_ENABLE_WAYLAND`, `QT_QPA_PLATFORM`,
   `SDL_VIDEODRIVER`, `_JAVA_AWT_WM_NONREPARENTING`. Firefox, Qt apps, SDL games
   and Java UIs each default to X11 under an unrecognised compositor.

It also checks for a seat before a KMS start, because the failure without one is
a DRM error that never mentions seats.

## Running it nested instead

You do not have to log out to try it:

```bash
CROWN_BACKEND=winit crownos-session
```

That opens the whole session in a window inside your current desktop, spawns
everything in `startup`, and needs no seat and no TTY.

## What is deliberately not here

**No systemd user units.** `compositor.startup` already starts the desktop, is
watched live, and is the mechanism the compositor is built around. A second
mechanism that starts the same four programs is how you get two crownbars.

**No greeter.** CrownOS does not ship one. Any of GDM, SDDM, greetd or ly will
list this session once the `.desktop` file is installed.
