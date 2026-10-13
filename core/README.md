# DupeLens Core (Rust MVP)

Fast exact duplicate scanner with a safe cleanup-plan flow.

## Build

```bash
cargo build --release
```

Binary:

```bash
./target/release/dupelens-core
```

## Scan modes

Default mode is `balanced`.

```text
quick     10MB minimum, fastest, largest savings first
balanced  1MB minimum, default practical cleanup mode
deep       1 byte minimum, slow/noisy exhaustive scan
```

`--min-size` overrides the mode threshold.

## Interactive setup

Use this when you do not want to remember flags:

```bash
./target/release/dupelens-core --interactive ~/Downloads
```

or:

```bash
./target/release/dupelens-core -i ~/Downloads
```

It asks:

- recurse into subfolders or not;
- quick / balanced / deep scan;
- show progress or hide it;
- summary only, text report, JSON report, terminal preview, or cleanup plan;
- confirmation before starting.

## Simple clean wizard

This is the easiest flow:

```bash
./target/release/dupelens-core --clean ~/Downloads
```

It scans, shows a top duplicate preview, then asks whether to:

1. move recommended duplicate copies to Trash now;
2. save a cleanup plan JSON only;
3. exit without changes.

Moving files still requires typing `MOVE TO TRASH`.

## Compare roots

Use this when one folder is the source of truth and another folder is an import,
backup, external drive, or old Mac dump. DupeLens keeps the first path untouched
and only reports duplicate copies found in later paths:

```bash
./target/release/dupelens-core --compare -r ~/Downloads "/Volumes/Backup/Old Downloads" --quiet --summary
./target/release/dupelens-core --compare -r ~/Downloads "/Volumes/Backup/Old Downloads" --quiet --plan cleanup-plan.json
```

In compare mode, cleanup plans mark only non-primary-root copies as
`move_to_trash` candidates.

## Examples

Balanced summary, default:

```bash
./target/release/dupelens-core -r ~/Downloads --quiet --summary
```

Quick cleanup summary:

```bash
./target/release/dupelens-core -r ~/Downloads --quiet --summary --mode quick
```

Deep scan, equivalent to the old default:

```bash
./target/release/dupelens-core -r ~/Downloads --quiet --summary --mode deep
```

Full text report:

```bash
./target/release/dupelens-core -r ~/Downloads --quiet --mode balanced -o duplicates.txt
```

JSON report:

```bash
./target/release/dupelens-core -r ~/Downloads --quiet --mode balanced --json -o duplicates.json
```

Safe cleanup plan, no deletion:

```bash
./target/release/dupelens-core -r ~/Downloads --quiet --mode balanced --plan cleanup-plan.json
```

Compare a primary folder against a backup/import folder:

```bash
./target/release/dupelens-core --compare -r ~/Downloads "/Volumes/Backup/Old Downloads" --quiet --summary
```

The cleanup plan keeps the first sorted path in each group and marks the rest as `move_to_trash` candidates for review.

Apply a reviewed plan by moving candidates to Trash:

```bash
./target/release/dupelens-core --apply-plan cleanup-plan.json --confirm "MOVE TO TRASH"
```

This does not use `rm`. It moves files to `~/.Trash` on macOS and `~/.local/share/Trash/files` on Linux. For tests, set `DUPLENS_TRASH_DIR=/path/to/test-trash`.

Override minimum size manually:

```bash
./target/release/dupelens-core -r ~/Downloads --quiet --min-size 50MB -o duplicates-50mb.txt
```

Use more/less hashing workers:

```bash
./target/release/dupelens-core -r ~/Downloads --threads 8 --summary
```

## Notes

- Scanning and `--plan` are read-only.
- `--apply-plan` is the only state-changing command and requires exact confirmation.
- Exact duplicate detection only.
- Pipeline: scan -> group by size -> partial hash -> full hash -> byte verify.
- Uses a fast built-in streaming FNV-1a hash for candidate grouping, then byte-verifies matches by default.
