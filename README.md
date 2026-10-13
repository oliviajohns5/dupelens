# DupeLens

DupeLens is a safe duplicate-file cleanup tool for messy folders, imports, backups, external drives, and old Mac migrations.

It is **not** a general Mac cleaner. The first product wedge is simple:

> Choose a primary folder and a backup/import folder. DupeLens finds exact duplicates and protects the primary folder.

## Status

Early MVP / prototype.

Current repo contains:

- `core/` — Rust CLI core, the product backend going forward.
- `prototype-python/` — Python reference prototype used to validate behavior and UX.
- `docs/` — product notes and GUI transition plan.

## Why DupeLens exists

Most Mac cleaner apps hide duplicate cleanup inside a broader cleaner workflow. DupeLens focuses on exact duplicate cleanup where safety matters:

- old `Downloads` folders;
- external drives;
- backup/import folders;
- migration dumps from old Macs;
- developer/project archives;
- NAS or mounted storage;
- “keep this folder untouched, clean duplicates from that folder”.

## Safety principles

- Explicit user-selected roots only.
- Trash folders are always excluded from scans.
- Exact duplicates only: size → partial hash → full hash → byte verification.
- Cleanup is plan-based and reviewable.
- `--apply-plan` requires the exact confirmation phrase `MOVE TO TRASH`.
- Files are moved to Trash, not permanently deleted by default.
- At least one verified copy is kept per group.
- Compare mode keeps the first/primary root untouched.

## Recommended path: Rust core

Build:

```bash
cd core
cargo build --release
```

Run a normal scan:

```bash
./target/release/dupelens-core -r ~/Downloads --quiet --summary
```

Compare a primary folder with an old backup/import folder:

```bash
./target/release/dupelens-core --compare -r ~/Downloads "/Volumes/Backup/Old Downloads" --quiet --summary
```

Create a safe cleanup plan:

```bash
./target/release/dupelens-core --compare -r ~/Downloads "/Volumes/Backup/Old Downloads" --quiet --plan cleanup-plan.json
```

Emit GUI-ready scan JSON:

```bash
./target/release/dupelens-core --compare -r ~/Downloads "/Volumes/Backup/Old Downloads" --quiet --ui-json > scan.json
```

Emit progress events for a visual interface:

```bash
./target/release/dupelens-core --compare -r ~/Downloads "/Volumes/Backup/Old Downloads" --ui-json --json-events > scan.json 2> events.ndjson
```

Apply a reviewed plan:

```bash
./target/release/dupelens-core --apply-plan cleanup-plan.json --confirm "MOVE TO TRASH"
```

## Scan modes

```text
quick     files >= 10MB, fastest, largest savings first
balanced  files >= 1MB, default practical cleanup mode
deep      files >= 1 byte, exhaustive but noisy
```

Examples:

```bash
./target/release/dupelens-core -r ~/Downloads --mode quick --summary
./target/release/dupelens-core -r ~/Downloads --mode balanced --summary
./target/release/dupelens-core -r ~/Downloads --mode deep --summary
./target/release/dupelens-core -r ~/Downloads --min-size 50MB -o duplicates.txt
```

## Python prototype

The Python version mirrors the core behavior and is useful for readable reference/testing:

```bash
cd prototype-python
python3 dupelens.py --compare -r ~/Downloads "/Volumes/Backup/Old Downloads" --quiet --summary
```

## GUI direction

DupeLens should move toward a visual interface around the Rust core, not a broad cleaner.

Planned first GUI flow:

1. Choose `Primary folder`.
2. Choose `Backup/import folder`.
3. Scan exact duplicates.
4. Show `KEEP` vs `TRASH` recommendations.
5. Save cleanup plan or move selected duplicates to Trash.

See [`docs/GUI_PLAN.md`](docs/GUI_PLAN.md).

## Development

Run checks:

```bash
./scripts/qa.sh
```

Or manually:

```bash
cd core
cargo test
cargo build --release

cd ../prototype-python
python3 -m py_compile dupelens.py
```

## License

MIT. See [`LICENSE`](LICENSE).
