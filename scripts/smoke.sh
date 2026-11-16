#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT/src-tauri"
cargo test
cargo clippy -- -D warnings
RESULTS=$(mktemp -d "${XDG_RUNTIME_DIR:?}/quickask/smoke.XXXXXX")
BIN="$HOME/.local/bin/quickask"
timeout 30s "$BIN" --ask 'Responde solo: OK' > "$RESULTS/simple.jsonl"
python3 - "$RESULTS/simple.jsonl" <<'PY'
import json,sys
es=[json.loads(l) for l in open(sys.argv[1])]
assert es[-1]['kind']=='done' and es[-1]['outcome']=='succeeded'
assert 'OK' in ''.join(e.get('delta',e.get('text','')) for e in es if e['kind'] in ['text','textFinal'])
PY
SECONDS=0
timeout 15s "$BIN" --ask '¿Qué aplicaciones se ven en esta pantalla? Una frase.' --screen > "$RESULTS/screen.jsonl"
SCREEN_SECONDS=$SECONDS
python3 - "$RESULTS/screen.jsonl" "$SCREEN_SECONDS" <<'PY'
import json,sys
es=[json.loads(l) for l in open(sys.argv[1])]
assert es[-1]['kind']=='done' and es[-1]['outcome']=='succeeded'
assert any(e['kind']=='screen' and e['status']=='attached' for e in es)
assert any(e['kind']=='textFinal' and e['text'].strip() for e in es)
assert not any(e['kind']=='tool' and 'fusion-visual' in e.get('title','') for e in es)
assert int(sys.argv[2])<15, 'La visión superó 15 segundos'
PY
python3 - "$BIN" <<'PY'
import os,socket,json,subprocess,sys,time
p=os.environ['XDG_RUNTIME_DIR']+'/quickask/ctl.sock'
s=socket.socket(socket.AF_UNIX);s.connect(p);s.sendall(b'{"cmd":"hide"}\n');s.close();time.sleep(.2)
log=os.path.expanduser('~/.local/state/quickask/quickask.log')
start=os.path.getsize(log)
begin=time.monotonic();subprocess.run([sys.argv[1],'--toggle'],check=True)
assert (time.monotonic()-begin)<.15, 'Cliente toggle demasiado lento'
time.sleep(.5)
with open(log) as f: f.seek(start); text=f.read()
assert 'toggle visible' in text and 'focused=true' in text, text
import re
match=re.search(r'toggle visible ms=(\d+)',text)
assert match and int(match.group(1))<150, text
subprocess.run([sys.argv[1],'--toggle'],check=True)
PY
printf 'Smoke OK. Resultados: %s\n' "$RESULTS"
