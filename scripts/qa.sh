#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRATCH="${TMPDIR:-/tmp}/dupelens-qa-$$"
trap 'rm -rf "$SCRATCH"' EXIT
mkdir -p "$SCRATCH/primary" "$SCRATCH/backup" "$SCRATCH/trash"

python3 - <<PY
from pathlib import Path
b=Path('$SCRATCH')
(b/'primary'/'keep.bin').write_bytes(b'A'*4096)
(b/'backup'/'copy1.bin').write_bytes(b'A'*4096)
(b/'backup'/'copy2.bin').write_bytes(b'A'*4096)
(b/'primary'/'primary-only-1.bin').write_bytes(b'B'*4096)
(b/'primary'/'primary-only-2.bin').write_bytes(b'B'*4096)
(b/'backup'/'backup-only-1.bin').write_bytes(b'C'*4096)
(b/'backup'/'backup-only-2.bin').write_bytes(b'C'*4096)
(b/'primary'/'same-size.bin').write_bytes(b'D'*4096)
(b/'backup'/'same-size.bin').write_bytes(b'E'*4096)
PY

printf '== Python compile + compare plan ==\n'
cd "$ROOT/prototype-python"
python3 -m py_compile dupelens.py
python3 dupelens.py --compare -r --mode deep --quiet --plan "$SCRATCH/python-plan.json" "$SCRATCH/primary" "$SCRATCH/backup" > "$SCRATCH/python-plan.out"
python3 - <<PY
import json
from pathlib import Path
b=Path('$SCRATCH')
p=json.loads((b/'python-plan.json').read_text())
assert p['total_reclaimable_bytes']==8192, p
assert len(p['groups'])==1, p
assert p['groups'][0]['keep'].endswith('/primary/keep.bin'), p
assert all('/backup/' in x for x in p['groups'][0]['move_to_trash']), p
print('python_compare_plan_ok')
PY
DUPLENS_TRASH_DIR="$SCRATCH/trash" python3 dupelens.py --apply-plan "$SCRATCH/python-plan.json" --confirm "MOVE TO TRASH" > "$SCRATCH/python-apply.out"
test -f "$SCRATCH/primary/keep.bin"
test ! -e "$SCRATCH/backup/copy1.bin"
test ! -e "$SCRATCH/backup/copy2.bin"

printf '== Rust build + compare plan ==\n'
rm -rf "$SCRATCH/primary" "$SCRATCH/backup" "$SCRATCH/trash"
mkdir -p "$SCRATCH/primary" "$SCRATCH/backup" "$SCRATCH/trash"
python3 - <<PY
from pathlib import Path
b=Path('$SCRATCH')
(b/'primary'/'keep.bin').write_bytes(b'R'*4096)
(b/'primary'/'single-dup-1.bin').write_bytes(b'T'*4096)
(b/'primary'/'single-dup-2.bin').write_bytes(b'T'*4096)
(b/'backup'/'copy1.bin').write_bytes(b'R'*4096)
(b/'backup'/'copy2.bin').write_bytes(b'R'*4096)
(b/'backup'/'backup-only-1.bin').write_bytes(b'S'*4096)
(b/'backup'/'backup-only-2.bin').write_bytes(b'S'*4096)
PY
cd "$ROOT/core"
cargo test
cargo build --release
./target/release/dupelens-core --compare -r --mode deep --quiet --plan "$SCRATCH/rust-plan.json" "$SCRATCH/primary" "$SCRATCH/backup" > "$SCRATCH/rust-plan.out"
./target/release/dupelens-core -r --mode deep --quiet --ui-json "$SCRATCH/primary" > "$SCRATCH/rust-single-ui.json"
./target/release/dupelens-core --compare -r --mode deep --quiet --ui-json "$SCRATCH/primary" "$SCRATCH/backup" > "$SCRATCH/rust-ui.json"
./target/release/dupelens-core --compare -r --mode deep --json-events --ui-json "$SCRATCH/primary" "$SCRATCH/backup" > "$SCRATCH/rust-events-ui.json" 2> "$SCRATCH/rust-events.ndjson"
python3 - <<PY
import json
from pathlib import Path
b=Path('$SCRATCH')
p=json.loads((b/'rust-plan.json').read_text())
assert p['total_reclaimable_bytes']==8192, p
assert len(p['groups'])==1, p
assert p['groups'][0]['keep'].endswith('/primary/keep.bin'), p
assert all('/backup/' in x for x in p['groups'][0]['move_to_trash']), p
ui=json.loads((b/'rust-ui.json').read_text())
assert ui['schema']=='dupelens-scan-v1', ui
assert ui['mode']=='compare', ui
assert ui['summary']['reclaimable_bytes']==8192, ui
assert ui['groups'][0]['keep'].endswith('/primary/keep.bin'), ui
assert all('/backup/' in x for x in ui['groups'][0]['trash_candidates']), ui
events=[json.loads(line) for line in (b/'rust-events.ndjson').read_text().splitlines() if line.strip()]
assert any(e.get('event')=='progress' for e in events), events
assert events[-1].get('event')=='done', events[-1]
single=json.loads((b/'rust-single-ui.json').read_text())
assert single['schema']=='dupelens-scan-v1', single
assert single['mode']=='scan', single
assert single['summary']['groups']==1, single
print('rust_compare_plan_ok')
print('rust_ui_json_events_ok')
print('rust_single_folder_ui_ok')
PY

printf '== Local web UI syntax ==\n'
cd "$ROOT"
python3 -m py_compile app/server.py
test -f app/index.html
test -f app/app.js
test -f app/styles.css

printf '== Done ==\n'
