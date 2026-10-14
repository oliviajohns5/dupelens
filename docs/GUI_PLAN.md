# DupeLens GUI Plan

## Goal

Build a visual interface for duplicate cleanup, focused on compare-mode safety:

> Keep my primary folder untouched. Find duplicate copies in backup/import folders and let me review before moving them to Trash.

## Non-goals

- Do not build a general Mac cleaner.
- Do not scan the whole disk by default.
- Do not clean caches, app leftovers, logs, Time Machine snapshots, or system folders.
- Do not add fuzzy image/audio/video matching before exact duplicate cleanup is polished.

## Recommended implementation path

### Phase 1 — stabilize Rust core for UI

Implemented in the Rust core via `--ui-json`. The stable UI-facing JSON schema:

```json
{
  "schema": "dupelens-scan-v1",
  "mode": "compare",
  "scan_mode": "balanced",
  "min_size_bytes": 1048576,
  "recursive": true,
  "primary_root": "/Users/me/Downloads",
  "roots": ["/Users/me/Downloads", "/Volumes/Backup/Old Downloads"],
  "summary": {
    "groups": 12,
    "files": 39,
    "reclaimable_bytes": 123456789,
    "elapsed_seconds": 1.234
  },
  "groups": [
    {
      "id": "...",
      "size": 123,
      "reclaimable_bytes": 246,
      "hash": "...",
      "keep": "...",
      "trash_candidates": ["..."],
      "files": [
        {"id":"...", "path":"...", "size":123, "mtime":0, "role":"keep"},
        {"id":"...", "path":"...", "size":123, "mtime":0, "role":"trash_candidate"}
      ]
    }
  ]
}
```

Implemented in the Rust core via `--json-events`. Events are newline-delimited
JSON on stderr while the final report remains on stdout:

```bash
./target/release/dupelens-core --compare -r PRIMARY BACKUP --ui-json --json-events > scan.json 2> events.ndjson
```

Progress event examples:

```json
{"event":"progress","phase":"Partial hashing","done":120,"total":500,"detail":"file.bin","elapsed_seconds":0.420}
{"event":"phase_done","message":"Full hashing complete: 12 duplicate groups","elapsed_seconds":1.100}
{"event":"done","groups":12,"reclaimable_bytes":123456789,"elapsed_seconds":1.234}
```

Stable IDs are included:

- `group_id = hash + size`
- `file_id = hash(path + size + mtime)`

### Phase 2 — local web UI prototype

Implemented under `app/` as a local-only Python stdlib server.

Run:

```bash
cd dupelens
cd core && cargo build --release && cd ..
python3 app/server.py
```

The local server intentionally stays running while the browser UI is open.
Stop it with `Ctrl+C` in the Terminal window. Request logging is suppressed so
browser polling does not fill the terminal during scans.

Open `http://127.0.0.1:8787`.

Files:

```text
app/
  server.py
  index.html
  styles.css
  app.js
```

UI flow:

1. Choose mode: `Scan one folder` or `Compare with backup`.
2. Pick paths with `Choose folder` / `Choose disk`; manual path input remains as fallback.
3. Start scan.
4. Show progress.
5. Show summary: groups, files, reclaimable space.
6. Show groups with `KEEP` and `TRASH` labels.
7. Save cleanup plan.
8. Apply selected cleanup only after confirmation.

### Phase 3 — desktop app

Options:

- Tauri: Rust backend + web UI, fastest cross-platform desktop path.
- SwiftUI: best native Mac app path, but requires macOS/Xcode for full build/test/signing.

Recommended: prototype UI locally first, then decide between Tauri and SwiftUI once the product flow is proven.

## Safety requirements for GUI

- Folder selection must be explicit.
- Native picker is allowed because the server runs only on `127.0.0.1`; it must not accept remote URLs.
- Primary root must be visually marked as protected.
- Every destructive action requires review.
- Move to Trash only; no permanent delete in v1.
- Never delete the last copy in a group.
- Show why each file is recommended for KEEP/TRASH.
- Plans must be editable before apply.
- Apply only selected groups/files.
- After apply, show moved/skipped/error counts.

## First visual MVP acceptance criteria

- User can compare two folders.
- UI shows progress while scanning.
- UI shows duplicate groups sorted by reclaimable bytes.
- UI marks one `KEEP` file under primary root.
- UI marks only secondary-root copies as `TRASH` candidates.
- User can save a cleanup plan JSON.
- User can apply a selected plan with `MOVE TO TRASH` confirmation.
- Synthetic compare fixture passes end-to-end.
