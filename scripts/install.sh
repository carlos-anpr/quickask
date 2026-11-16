#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
[[ -d "$ROOT/ui/node_modules" ]] || npm --prefix "$ROOT/ui" ci
cd "$ROOT/src-tauri"
cargo tauri build --no-bundle
mkdir -p "$HOME/.local/bin" "$HOME/.config/autostart"
if [[ -x "$HOME/.local/bin/quickask" ]]; then
    "$HOME/.local/bin/quickask" --quit
    for _ in {1..50}; do
        if ! python3 - <<'PY'
import os,socket,sys
s=socket.socket(socket.AF_UNIX)
try: s.connect(os.environ['XDG_RUNTIME_DIR']+'/quickask/ctl.sock')
except OSError: sys.exit(1)
PY
        then break; fi
        sleep .1
    done
fi
# Atomic replacement also works while an older executable finishes shutting down.
install -m 755 target/release/quickask "$HOME/.local/bin/quickask.new"
mv -f "$HOME/.local/bin/quickask.new" "$HOME/.local/bin/quickask"
cat > "$HOME/.config/autostart/quickask.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=quickask
Exec="$HOME/.local/bin/quickask" --background
X-GNOME-Autostart-enabled=true
Terminal=false
DESKTOP
python3 "$ROOT/scripts/keybindings.py" install
setsid -f "$HOME/.local/bin/quickask" --background </dev/null >/dev/null 2>&1
printf '%s\n' 'quickask instalado y arrancado.'
