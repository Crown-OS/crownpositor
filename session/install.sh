#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Install the CrownOS session so a display manager can offer it, and drop an
# example config if there is none.
#
#   ./install.sh            # into ~/.local  -- no root, this user only
#   sudo ./install.sh --system   # into /usr/local -- every user
#   ./install.sh --uninstall
#
# What this installs and why it is separate from `cargo install`: cargo puts a
# binary on your PATH and stops. A *session* additionally needs an entry under
# wayland-sessions for the greeter to list, and a launcher that sets
# XDG_CURRENT_DESKTOP and a session bus before the compositor starts.

set -euo pipefail

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SYSTEM=0
UNINSTALL=0

for a in "$@"; do
  case "$a" in
    --system)    SYSTEM=1 ;;
    --uninstall) UNINSTALL=1 ;;
    -h|--help)   sed -n '3,14p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'; exit 0 ;;
    *) echo "unknown argument: $a" >&2; exit 2 ;;
  esac
done

if [ "$SYSTEM" -eq 1 ]; then
  PREFIX=/usr/local
  SESSIONS="$PREFIX/share/wayland-sessions"
  BIN="$PREFIX/bin"
  [ "$(id -u)" -eq 0 ] || { echo "--system needs root" >&2; exit 1; }
else
  PREFIX="$HOME/.local"
  # Greeters read the user directory too, but not all of them do; --system is
  # the reliable one for a login screen.
  SESSIONS="$PREFIX/share/wayland-sessions"
  BIN="$PREFIX/bin"
fi

CONFIG="${CROWN_CONFIG_DIR:-$HOME/.config/crownos}"

if [ "$UNINSTALL" -eq 1 ]; then
  rm -fv "$SESSIONS/crownos.desktop" "$BIN/crownos-session"
  echo "Left $CONFIG alone -- your settings are yours to delete."
  exit 0
fi

install -Dm755 "$HERE/crownos-session" "$BIN/crownos-session"
install -Dm644 "$HERE/crownos.desktop" "$SESSIONS/crownos.desktop"
echo "installed $BIN/crownos-session"
echo "installed $SESSIONS/crownos.desktop"

if [ ! -e "$CONFIG/compositor.ron" ]; then
  install -Dm644 "$HERE/compositor.example.ron" "$CONFIG/compositor.ron"
  echo "installed $CONFIG/compositor.ron (example -- edit it)"
else
  echo "kept $CONFIG/compositor.ron (already exists)"
  echo "  compare against $HERE/compositor.example.ron for the startup list"
fi

case ":${PATH}:" in
  *":$BIN:"*) ;;
  *) echo; echo "warning: $BIN is not on your PATH. Add:"
     echo "    export PATH=\"$BIN:\$PATH\"" ;;
esac

echo
echo "Log out and pick CrownOS at your display manager, or from a TTY run:"
echo "    crownos-session"
echo "To try it inside your current session instead:"
echo "    CROWN_BACKEND=winit crownos-session"
