# crownpositor

A tiling Wayland compositor for [CrownOS](https://github.com/Crown-OS), built on
[Smithay](https://github.com/Smithay/smithay).

`crownpositor` is not only a window manager — it *is* the CrownOS session. It
creates the Wayland socket, exports `WAYLAND_DISPLAY`, and spawns the rest of the
desktop itself.

**Status: Early.** It builds, runs and is actively developed, but the API and
internals move, and several features are stubbed. See
[Known limitations](#known-limitations).

## What it does

- **Three layouts** — master-stack (dwm/xmonad), scrolling columns (niri/PaperWM)
  and floating, switchable per workspace
- **Two backends** — DRM/KMS on a bare TTY, and winit for a nested development
  session
- **wlr-layer-shell**, so `crownbar`, `crowndock`, `crownotify` and
  `crowndictator` can attach
- **XWayland** for X11 clients
- **Trackpad gestures** that resolve to the same action vocabulary as keyboard
  chords
- **Spring animations**, critically damped, integrated at a fixed 1/240 s substep
- **Live configuration** — a rebind takes effect without a restart

## Layout

Cargo workspace with two members:

| Crate | Role |
|---|---|
| `compositor` | Everything. `main.rs` is three lines calling `compositor::run()`. |
| `config` | The compositor's *compiled* configuration — regexes, chords, geometry |

## Prerequisites

`crownos-config` is an ordinary crates.io dependency
(`{ version = "0.2", default-features = false }`), but **0.2 is not published** —
nothing in the CrownOS organization is on crates.io except `crownshell` 0.1.0 and
0.2.0. A fresh clone fails at `cargo metadata` until Cargo is pointed at a local
checkout.

`crownos-setup`'s `./bootstrap.sh --dev` clones the repos side by side and writes
a `[patch.crates-io]` overlay into a `.cargo/config.toml` one directory **above**
them:

```
~/crownos/
├── .cargo/config.toml   # [patch.crates-io] crownos-config = { path = "crownos-config" }
├── crownpositor/
└── crownos-config/
```

Cargo walks up from the working directory to find that file, and the paths in it
are relative to the file's own directory. No particular directory layout is
required — the repository itself contains no `[patch]` section.

Native dependencies (Arch):

```bash
sudo pacman -S --needed base-devel pkgconf \
  wayland wayland-protocols libxkbcommon \
  libdrm libinput seatd systemd-libs pixman \
  vulkan-icd-loader vulkan-headers mesa libglvnd \
  fontconfig dbus xorg-xwayland
```

Debian/Ubuntu equivalents and the full list:
[Prerequisites](https://github.com/Crown-OS/crownos-documentations/blob/main/docs/10-getting-started/prerequisites.md).

Rust **1.88+** (set by the dependency graph, not the edition; pinned in `rust-toolchain.toml`).

> `smithay-drm-extras` is deliberately declared `default-features = false`
> because the `display-info` sys crate does not accept the version of
> `libdisplay-info` on current systems. Do not re-enable it.

## Build and run

```bash
cargo build
cargo test                        # 183 unit tests

# Nested inside your existing session — what you want for development
CROWN_BACKEND=winit cargo run

# On real hardware, from a bare TTY, with seatd running
CROWN_BACKEND=kms cargo run --release
```

`Super+Return` spawns `foot`. `Super+Shift+E` quits. Have a second TTY available
before running on hardware.

To attach a client to a nested session, use the socket name it logs:

```bash
WAYLAND_DISPLAY=wayland-2 cargo run     # in crownbar, for example
```

### Environment

| Variable | Values |
|---|---|
| `CROWN_BACKEND` | `winit`, or `kms`/`drm`/`udev`. Unset autodetects. |
| `CROWN_RENDER_API` | `egl`/`gles`/`gles3` (default), or `vulkan`/`vk` |
| `XCURSOR_THEME`, `XCURSOR_SIZE` | Cursor theme and size |
| `CROWN_CONFIG_DIR` | Overrides `~/.config/crownos` |

Unknown values for the first two log a warning and fall back rather than failing.

## Configuration

`~/.config/crownos/compositor.ron`, watched live. Also reads `appearance.ron` and
`display.ron`.

```ron
(
    layout: ScrollingColumns,
    focus_follows_mouse: true,
    keybinds: [
        (keys: "Super+Return", action: "spawn foot"),
    ],
    window_rules: [
        (app_id: "Nautilus", floating: true),
    ],
    outputs: [
        (name: "eDP-1", scale: 2.0, position: (0, 0)),
    ],
)
```

An empty `keybinds` list means "use the built-in defaults", not "nothing bound".

Full schema and the 32 default bindings:
[Configuration schema](https://github.com/Crown-OS/crownos-documentations/blob/main/docs/50-reference/config-schema.md)
·
[Keybindings](https://github.com/Crown-OS/crownos-documentations/blob/main/docs/50-reference/keybindings.md)

## Architecture

Two boundaries worth respecting when adding code:

- **`layout/` is surface-blind.** Nothing there can see a `WlSurface`, a
  `Window`, an `Output` or the `Shell`. That is why the tiling algorithms are
  unit-testable.
- **One action vocabulary.** Chords and gestures both produce the same `Action`
  enum and go down one dispatch path — deliberately, so that "swipe left" and
  `Super+Tab` cannot drift apart.

`backend/mod.rs` carries a four-step guide to adding a backend.

## Known limitations

- **Blur is not implemented.** `handlers/background_effect.rs` is entirely
  commented out, so `ext-background-effect-v1` is never advertised and every
  CrownOS surface requesting blur degrades silently.
- **The workspace overview does not exist.** `shell/windows_view/` and
  `shell/workspaces_view/` are zero-byte files; `OpenWorkspaceView` and
  `CloseWorkspaceView` log "not implemented yet" — and the four-finger gestures
  are bound to them.
- `shm_formats` is empty and the dmabuf global is never created.
- `privileged_client_filter` returns `true` for every client.
- Fractional scale sends the wrong scale; CSD is not honoured; popups are not
  unconstrained; session lock confirms early.
- No pinch, hold, touch, tablet or output hotplug handling.

## Contributing

See the organization-wide
[contribution guide](https://github.com/Crown-OS/crownos-documentations/blob/main/CONTRIBUTING.md).
Default branch here is **`main`**.

## License

Licensed under the [MIT License](LICENSE).
