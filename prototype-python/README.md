# DupeLens CLI MVP

Duplicate-file scanner prototype with a safe cleanup-plan flow.

Algorithm:

1. Enumerate files.
2. Group by exact size.
3. Hash first + last 1 MiB for same-size candidates.
4. Full SHA-256 hash only remaining candidates.
5. Optional byte-by-byte verification.
6. Report duplicate groups and reclaimable bytes.

## Scan modes

Default mode is `balanced`.

```text
quick     10MB minimum, fastest, largest savings first
balanced  1MB minimum, default practical cleanup mode
deep       1 byte minimum, slow/noisy exhaustive scan
```

`--min-size` overrides the mode threshold. Python accepts bytes or values like `1MB`, `10MB`, `1GB`.

## Interactive setup

Use this when you do not want to remember flags:

```bash
python3 dupelens.py --interactive ~/Downloads
```

or:

```bash
python3 dupelens.py -i ~/Downloads
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
python3 dupelens.py --clean ~/Downloads
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
python3 dupelens.py --compare -r ~/Downloads "/Volumes/Backup/Old Downloads" --quiet --summary
python3 dupelens.py --compare -r ~/Downloads "/Volumes/Backup/Old Downloads" --quiet --plan cleanup-plan.json
```

In compare mode, cleanup plans mark only non-primary-root copies as
`move_to_trash` candidates.

## Usage

```bash
python3 dupelens.py --clean ~/Downloads
python3 dupelens.py -r ~/Downloads --quiet --summary
python3 dupelens.py -r ~/Downloads --quiet --summary --mode quick
python3 dupelens.py -r ~/Downloads --quiet --summary --mode deep
python3 dupelens.py -r ~/Downloads --quiet --mode balanced -o duplicates.txt
python3 dupelens.py -r ~/Downloads --quiet --mode balanced --json -o duplicates.json
python3 dupelens.py -r ~/Downloads --quiet --mode balanced --plan cleanup-plan.json
python3 dupelens.py --compare -r ~/Downloads "/Volumes/Backup/Old Downloads" --quiet --summary
python3 dupelens.py -r ~/Downloads --quiet --min-size 50MB -o duplicates-50mb.txt
```

Apply a reviewed cleanup plan by moving candidates to Trash:

```bash
python3 dupelens.py --apply-plan cleanup-plan.json --confirm "MOVE TO TRASH"
```

This does not use `rm`. It moves files to `~/.Trash` on macOS and `~/.local/share/Trash/files` on Linux. For tests, set `DUPLENS_TRASH_DIR=/path/to/test-trash`.

The cleanup plan keeps the first sorted path in each group and marks the rest as `move_to_trash` candidates for review.

Scanning and `--plan` are read-only. `--apply-plan` is the only state-changing command and requires exact confirmation.
