#!/usr/bin/env python3
"""Manage only quickask's GNOME binding, preserving all unrelated entries."""
import ast
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib

SCHEMA = 'org.gnome.settings-daemon.plugins.media-keys'
CUSTOM = SCHEMA + '.custom-keybinding'
ROOT = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/'

def get(schema, key):
    return subprocess.check_output(['gsettings', 'get', schema, key], text=True).strip()

def put(schema, key, value):
    subprocess.run(['gsettings', 'set', schema, key, repr(value)], check=True)

def main():
    mode = sys.argv[1]
    runtime = Path(os.environ['XDG_RUNTIME_DIR']) / 'quickask'
    runtime.mkdir(mode=0o700, parents=True, exist_ok=True)
    raw = get(SCHEMA, 'custom-keybindings')
    paths = ast.literal_eval(raw.removeprefix('@as '))
    # Snapshot every touched key, including existing quickask entries, before writing.
    (runtime / 'keybindings.backup').write_text(raw + '\n')
    values = {p: {key: get(CUSTOM + ':' + p, key)
                  for key in ['name', 'command', 'binding']} for p in paths}
    (runtime / 'keybindings-values.backup.json').write_text(json.dumps(values, indent=2))
    ours = [p for p in paths if ast.literal_eval(values[p]['name']) == 'quickask']
    if mode == 'uninstall':
        put(SCHEMA, 'custom-keybindings', [p for p in paths if p not in ours])
        for p in ours:
            subprocess.run(['gsettings', 'reset-recursively', CUSTOM + ':' + p], check=True)
        return
    if mode != 'install':
        raise ValueError('Modo inválido')
    if ours:
        path = ours[0]
    else:
        i = 0
        while ROOT + f'custom{i}/' in paths:
            i += 1
        path = ROOT + f'custom{i}/'
        # Also back up orphaned values at the newly selected path.
        values[path] = {key: get(CUSTOM + ':' + path, key) for key in ['name', 'command', 'binding']}
        (runtime / 'keybindings-values.backup.json').write_text(json.dumps(values, indent=2))
        paths.append(path)
    conf = Path.home() / '.config/quickask/config.toml'
    shortcut = tomllib.loads(conf.read_text()).get('shortcut', '<Control><Alt>space') if conf.exists() else '<Control><Alt>space'
    put(CUSTOM + ':' + path, 'name', 'quickask')
    put(CUSTOM + ':' + path, 'command', str(Path.home() / '.local/bin/quickask') + ' --toggle')
    put(CUSTOM + ':' + path, 'binding', shortcut)
    put(SCHEMA, 'custom-keybindings', paths)

if __name__ == '__main__':
    main()
