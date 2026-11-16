#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
if [[ -x "$HOME/.local/bin/quickask" ]]; then "$HOME/.local/bin/quickask" --quit; fi
python3 "$ROOT/scripts/keybindings.py" uninstall
rm -f "$HOME/.local/bin/quickask" "$HOME/.config/autostart/quickask.desktop"
printf '%s\n' 'quickask desinstalado. Configuración y conversaciones conservadas.'
