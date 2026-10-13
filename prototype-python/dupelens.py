#!/usr/bin/env python3
"""DupeLens CLI - safe duplicate file scanner MVP with progress."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import BinaryIO, Iterable

CHUNK = 1024 * 1024
PROGRESS_INTERVAL = 0.35


@dataclass(frozen=True)
class FileInfo:
    path: str
    size: int
    mtime: float
    inode: int | None
    device: int | None


@dataclass(frozen=True)
class DuplicateGroup:
    size: int
    wasted_bytes: int
    full_hash: str
    files: list[FileInfo]


class Progress:
    def __init__(self, *, quiet: bool = False) -> None:
        self.quiet = quiet
        self.last = 0.0
        self.started = time.monotonic()
        self.last_line_len = 0

    def update(self, phase: str, current: int = 0, total: int | None = None, detail: str = "", *, force: bool = False) -> None:
        if self.quiet:
            return
        now = time.monotonic()
        if not force and now - self.last < PROGRESS_INTERVAL:
            return
        self.last = now
        elapsed = now - self.started
        if total:
            pct = min(100.0, (current / total) * 100)
            line = f"{phase}: {current}/{total} ({pct:5.1f}%)"
        elif current:
            line = f"{phase}: {current}"
        else:
            line = phase
        if detail:
            line += f" | {detail}"
        line += f" | {elapsed:,.1f}s"
        padded = line + " " * max(0, self.last_line_len - len(line))
        self.last_line_len = len(line)
        print("\r" + padded, end="", file=sys.stderr, flush=True)

    def done(self, line: str) -> None:
        if self.quiet:
            return
        self.update(line, force=True)
        print(file=sys.stderr, flush=True)
        self.last_line_len = 0


def format_bytes(n: int) -> str:
    units = ["B", "KB", "MB", "GB", "TB"]
    value = float(n)
    for unit in units:
        if value < 1024 or unit == units[-1]:
            return f"{value:.1f} {unit}" if unit != "B" else f"{int(value)} B"
        value /= 1024


def format_duration(seconds: float) -> str:
    total = int(seconds)
    millis = int((seconds - total) * 1000)
    hours, remainder = divmod(total, 3600)
    minutes, secs = divmod(remainder, 60)
    if hours:
        return f"{hours}h {minutes}m {secs}s"
    if minutes:
        return f"{minutes}m {secs}s"
    return f"{secs}.{millis:03d}s"


def parse_size(value: str) -> int:
    text = value.strip().lower()
    multipliers = {
        "k": 1024, "kb": 1024,
        "m": 1024 ** 2, "mb": 1024 ** 2,
        "g": 1024 ** 3, "gb": 1024 ** 3,
    }
    for suffix, multiplier in sorted(multipliers.items(), key=lambda item: -len(item[0])):
        if text.endswith(suffix):
            return int(text[:-len(suffix)].strip()) * multiplier
    return int(text)


def mode_min_size(mode: str) -> int:
    return {"quick": 10 * 1024 * 1024, "balanced": 1024 * 1024, "deep": 1}[mode]


def is_trash_path(path: Path) -> bool:
    parts = path.parts
    if ".Trash" in parts or ".Trashes" in parts:
        return True
    return any(parts[i:i + 4] == (".local", "share", "Trash", "files") for i in range(max(0, len(parts) - 3)))


def iter_files(paths: list[Path], *, recursive: bool, include_hidden: bool, follow_symlinks: bool, min_size: int, progress: Progress) -> Iterable[FileInfo]:
    scanned = 0
    kept = 0
    for root in paths:
        if is_trash_path(root):
            continue
        progress.update("Scanning files", scanned, detail=str(root), force=True)
        if root.is_file():
            for item in maybe_file(root, include_hidden=include_hidden, follow_symlinks=follow_symlinks, min_size=min_size):
                kept += 1
                yield item
            continue
        if not root.is_dir():
            continue
        walker = os.walk(root, followlinks=follow_symlinks) if recursive else [(str(root), [], os.listdir(root))]
        for dirpath, dirnames, filenames in walker:
            dirnames[:] = [d for d in dirnames if not is_trash_path(Path(dirpath) / d)]
            if not include_hidden:
                dirnames[:] = [d for d in dirnames if not d.startswith('.')]
                filenames = [f for f in filenames if not f.startswith('.')]
            filenames = [f for f in filenames if not is_trash_path(Path(dirpath) / f)]
            for name in filenames:
                scanned += 1
                for item in maybe_file(Path(dirpath) / name, include_hidden=include_hidden, follow_symlinks=follow_symlinks, min_size=min_size):
                    kept += 1
                    yield item
                progress.update("Scanning files", scanned, detail=f"kept {kept}; {dirpath}")
    progress.done(f"Scanning files complete: scanned {scanned}, kept {kept}")


def maybe_file(path: Path, *, include_hidden: bool, follow_symlinks: bool, min_size: int) -> Iterable[FileInfo]:
    try:
        if is_trash_path(path):
            return
        if not include_hidden and path.name.startswith('.'):
            return
        st = path.stat() if follow_symlinks else path.lstat()
        if not follow_symlinks and path.is_symlink():
            return
        if not path.is_file():
            return
        if st.st_size < min_size:
            return
        yield FileInfo(str(path), st.st_size, st.st_mtime, getattr(st, "st_ino", None), getattr(st, "st_dev", None))
    except OSError:
        return


def hash_file(path: str, *, limit: int | None = None) -> str | None:
    h = hashlib.sha256()
    try:
        with open(path, "rb") as f:
            if limit is None:
                while data := f.read(CHUNK):
                    h.update(data)
            else:
                h.update(read_partial(f, limit))
        return h.hexdigest()
    except OSError:
        return None


def read_partial(f: BinaryIO, limit: int) -> bytes:
    first = f.read(limit)
    try:
        f.seek(0, os.SEEK_END)
        size = f.tell()
        if size > limit:
            f.seek(max(0, size - limit))
            last = f.read(limit)
        else:
            last = b""
    except OSError:
        last = b""
    return first + b"\0DUPLENS\0" + last


def same_bytes(paths: list[str]) -> bool:
    if len(paths) < 2:
        return False
    try:
        handles = [open(p, "rb") for p in paths]
    except OSError:
        return False
    try:
        while True:
            chunks = [h.read(CHUNK) for h in handles]
            if not chunks[0]:
                return all(not c for c in chunks)
            if any(c != chunks[0] for c in chunks[1:]):
                return False
    finally:
        for h in handles:
            h.close()


def group_by(items: Iterable[FileInfo], key):
    buckets = {}
    for item in items:
        buckets.setdefault(key(item), []).append(item)
    return {k: v for k, v in buckets.items() if len(v) > 1}


def find_duplicates(paths: list[Path], *, recursive: bool, include_hidden: bool, follow_symlinks: bool, min_size: int, verify_bytes: bool, progress: Progress) -> list[DuplicateGroup]:
    files = list(iter_files(paths, recursive=recursive, include_hidden=include_hidden, follow_symlinks=follow_symlinks, min_size=min_size, progress=progress))

    progress.update("Grouping by size", force=True)
    by_size = group_by(files, lambda f: f.size)
    size_candidate_count = sum(len(v) for v in by_size.values())
    progress.done(f"Grouping by size complete: {len(by_size)} candidate size groups, {size_candidate_count} files")

    partial_candidates: list[FileInfo] = []
    partial_total = size_candidate_count
    partial_done = 0
    for same_size in by_size.values():
        partial_buckets = {}
        for info in same_size:
            partial_done += 1
            progress.update("Partial hashing", partial_done, partial_total, Path(info.path).name)
            digest = hash_file(info.path, limit=CHUNK)
            if digest:
                partial_buckets.setdefault(digest, []).append(info)
        for bucket in partial_buckets.values():
            if len(bucket) > 1:
                partial_candidates.extend(bucket)
    progress.done(f"Partial hashing complete: {len(partial_candidates)} files still candidates")

    groups: list[DuplicateGroup] = []
    full_by_size = group_by(partial_candidates, lambda f: f.size)
    full_total = sum(len(v) for v in full_by_size.values())
    full_done = 0
    for same_size in full_by_size.values():
        full_buckets = {}
        for info in same_size:
            full_done += 1
            progress.update("Full hashing", full_done, full_total, Path(info.path).name)
            digest = hash_file(info.path)
            if digest:
                full_buckets.setdefault(digest, []).append(info)
        for digest, bucket in full_buckets.items():
            if len(bucket) <= 1:
                continue
            if verify_bytes:
                progress.update("Byte verifying", len(groups) + 1, detail=Path(bucket[0].path).name)
                if not same_bytes([b.path for b in bucket]):
                    continue
            size = bucket[0].size
            groups.append(DuplicateGroup(size=size, wasted_bytes=size * (len(bucket) - 1), full_hash=digest, files=sorted(bucket, key=lambda f: f.path)))
    progress.done(f"Full hashing complete: {len(groups)} duplicate groups")
    return sorted(groups, key=lambda g: (-g.wasted_bytes, g.full_hash))


def path_under(path: str, root: Path) -> bool:
    try:
        Path(path).resolve(strict=False).relative_to(root.resolve(strict=False))
        return True
    except ValueError:
        return False


def filter_compare_groups(groups: list[DuplicateGroup], primary_root: Path) -> list[DuplicateGroup]:
    compare_groups: list[DuplicateGroup] = []
    for group in groups:
        primary_files = [f for f in group.files if path_under(f.path, primary_root)]
        other_files = [f for f in group.files if not path_under(f.path, primary_root)]
        if not primary_files or not other_files:
            continue
        files = [primary_files[0]] + sorted(other_files, key=lambda f: f.path)
        compare_groups.append(DuplicateGroup(
            size=group.size,
            wasted_bytes=group.size * len(other_files),
            full_hash=group.full_hash,
            files=files,
        ))
    return sorted(compare_groups, key=lambda g: (-g.wasted_bytes, g.full_hash))


def render_text(groups: list[DuplicateGroup], *, max_groups: int | None = None) -> str:
    if not groups:
        return "No exact duplicates found.\n"
    total_wasted = sum(g.wasted_bytes for g in groups)
    shown = groups if max_groups is None else groups[:max_groups]
    lines = [
        f"Found {len(groups)} duplicate groups; reclaimable if keeping one per group: {total_wasted} bytes ({format_bytes(total_wasted)})",
        f"Showing {len(shown)} of {len(groups)} groups",
    ]
    for index, group in enumerate(shown, 1):
        lines.append("")
        lines.append(f"Group {index}: {len(group.files)} files, size={group.size} ({format_bytes(group.size)}), wasted={group.wasted_bytes} ({format_bytes(group.wasted_bytes)}), sha256={group.full_hash}")
        for file in group.files:
            lines.append(f"  {file.path}")
    if max_groups is not None and len(groups) > max_groups:
        lines.append("")
        lines.append(f"... {len(groups) - max_groups} more groups omitted. Use --max-groups 0 or --output duplicates.txt for the full report.")
    return "\n".join(lines) + "\n"



def render_cleanup_plan_json(groups: list[DuplicateGroup]) -> str:
    plan = {
        "schema": "dupelens-cleanup-plan-v1",
        "dry_run": True,
        "action": "review_then_move_to_trash",
        "total_reclaimable_bytes": sum(g.wasted_bytes for g in groups),
        "groups": [],
    }
    for group in groups:
        keep = group.files[0]
        plan["groups"].append({
            "size": group.size,
            "reclaimable_bytes": group.wasted_bytes,
            "hash": group.full_hash,
            "keep": keep.path,
            "move_to_trash": [f.path for f in group.files[1:]],
        })
    return json.dumps(plan, indent=2) + "\n"


def render_json(groups: list[DuplicateGroup]) -> str:
    return json.dumps({"groups": [asdict(g) for g in groups], "total_wasted_bytes": sum(g.wasted_bytes for g in groups)}, indent=2) + "\n"


def trash_dir() -> Path:
    override = os.environ.get("DUPLENS_TRASH_DIR")
    if override:
        return Path(override)
    home = Path.home()
    if sys.platform == "darwin":
        return home / ".Trash"
    return home / ".local" / "share" / "Trash" / "files"


def unique_trash_path(trash: Path, src: Path) -> Path:
    candidate = trash / src.name
    if not candidate.exists():
        return candidate
    stem = src.stem or "dupelens-file"
    suffix = src.suffix
    for i in range(1, 10_000):
        candidate = trash / f"{stem}-dupelens-{i}{suffix}"
        if not candidate.exists():
            return candidate
    return trash / f"dupelens-file-{int(time.time() * 1000)}"


def apply_cleanup_plan(plan_path: Path, confirm: str | None) -> int:
    if confirm != "MOVE TO TRASH":
        raise SystemExit('refusing to move files: pass --confirm "MOVE TO TRASH"')
    plan = json.loads(plan_path.read_text(encoding="utf-8"))
    if plan.get("schema") != "dupelens-cleanup-plan-v1" or plan.get("dry_run") is not True:
        raise SystemExit("not a DupeLens dry-run cleanup plan")
    candidates: list[Path] = []
    for group in plan.get("groups", []):
        candidates.extend(Path(p) for p in group.get("move_to_trash", []))
    if not candidates:
        print("No move_to_trash candidates found. Nothing moved.")
        return 0
    trash = trash_dir()
    trash.mkdir(parents=True, exist_ok=True)
    moved = skipped = missing = directories = errors = 0
    skipped_examples: list[str] = []
    error_examples: list[str] = []
    for src in candidates:
        if not src.exists():
            skipped += 1
            missing += 1
            if len(skipped_examples) < 10:
                skipped_examples.append(f"missing: {src}")
            continue
        if src.is_dir():
            skipped += 1
            directories += 1
            if len(skipped_examples) < 10:
                skipped_examples.append(f"directory: {src}")
            continue
        dest = unique_trash_path(trash, src)
        try:
            src.rename(dest)
        except OSError as exc:
            errors += 1
            if len(error_examples) < 10:
                error_examples.append(f"{src}: {exc}")
            continue
        moved += 1
        if moved <= 10:
            print(f"moved to trash: {src} -> {dest}")
    if moved > 10:
        print(f"... {moved - 10} more files moved")
    if skipped_examples:
        print("Skipped examples:", file=sys.stderr)
        for item in skipped_examples:
            print(f"  {item}", file=sys.stderr)
        if skipped > len(skipped_examples):
            print(f"  ... {skipped - len(skipped_examples)} more skipped", file=sys.stderr)
    if error_examples:
        print("Error examples:", file=sys.stderr)
        for item in error_examples:
            print(f"  {item}", file=sys.stderr)
        if errors > len(error_examples):
            print(f"  ... {errors - len(error_examples)} more errors", file=sys.stderr)
    print(f"Done. moved={moved}, skipped={skipped}, missing={missing}, directories={directories}, errors={errors}, trash={trash}")
    if moved == 0 and missing > 0 and errors == 0:
        print("Nothing moved. This usually means the cleanup plan was already applied or the files were moved elsewhere. Run --clean again to rescan current files.")
    return 1 if errors else 0




def print_clean_preview(groups: list[DuplicateGroup], limit: int = 20) -> None:
    for idx, group in enumerate(groups[:limit], 1):
        keep = group.files[0]
        print(f"\n{idx}. {len(group.files)} files, reclaimable {group.wasted_bytes} bytes ({format_bytes(group.wasted_bytes)})")
        print(f"   KEEP: {keep.path}")
        for file in group.files[1:6]:
            print(f"   TRASH: {file.path}")
        if len(group.files) > 6:
            print(f"   ... {len(group.files) - 6} more trash candidates")
    if len(groups) > limit:
        print(f"\n... {len(groups) - limit} more groups not shown in preview")


def run_clean_setup(args) -> None:
    print("DupeLens clean wizard")
    print("Scanning: " + ", ".join(str(p) for p in args.paths))
    print("This flow scans, shows the largest duplicate groups, then can move recommended copies to Trash.")
    print()
    args.recursive = ask_yes_no("Recurse into subfolders?", True)
    if args.min_size is None:
        print("\nScan size mode:")
        print("  1) Quick     - files >= 10MB, fastest, largest cleanup wins")
        print("  2) Balanced  - files >= 1MB, recommended default")
        print("  3) Deep      - files >= 1 byte, slow/noisy exhaustive scan")
        choice = ask_choice("Choose mode", "2").strip().lower()
        if choice in {"1", "quick", "q"}:
            args.mode = "quick"
        elif choice in {"3", "deep", "d"}:
            args.mode = "deep"
        else:
            args.mode = "balanced"
    args.quiet = not ask_yes_no("Show live progress?", True)
    print("\nPlan:")
    min_size = args.min_size if args.min_size is not None else mode_min_size(args.mode)
    print(f"  recursive: {args.recursive}")
    print(f"  mode: {args.mode}; min-size: {format_bytes(min_size)}")
    print(f"  progress: {'hidden' if args.quiet else 'shown'}")
    print()
    if not ask_yes_no("Start scan now?", True):
        raise SystemExit("clean cancelled")


def run_clean_flow(args, groups: list[DuplicateGroup], elapsed_text: str, total_wasted: int, min_size: int) -> int:
    print("\nScan complete")
    print(f"Found {len(groups)} duplicate groups; reclaimable: {total_wasted} bytes ({format_bytes(total_wasted)})")
    print(f"Scan mode: {args.mode}; min-size: {format_bytes(min_size)}")
    print(f"Scan time: {elapsed_text}")
    if not groups:
        return 0
    print("\nTop recommended cleanup groups:")
    print_clean_preview(groups, 20)
    print("\nNext step:")
    print("  1) Move all recommended duplicate copies to Trash now")
    print("  2) Save cleanup plan JSON only")
    print("  3) Exit without changes")
    choice = ask_choice("Choose action", "2").strip().lower()
    if choice in {"1", "move", "trash"}:
        print("\nSafety rule: DupeLens keeps one file per group and moves only duplicate copies.")
        confirm = ask_choice("Type MOVE TO TRASH to continue", "")
        if confirm != "MOVE TO TRASH":
            print("Cancelled. No files were moved.")
            return 0
        tmp_plan = Path(os.environ.get("TMPDIR", "/tmp")) / f"dupelens-clean-plan-{os.getpid()}.json"
        tmp_plan.write_text(render_cleanup_plan_json(groups), encoding="utf-8")
        try:
            return apply_cleanup_plan(tmp_plan, "MOVE TO TRASH")
        finally:
            try:
                tmp_plan.unlink()
            except OSError:
                pass
    if choice in {"3", "exit", "q", "quit"}:
        print("No files were moved.")
        return 0
    path = Path(ask_choice("Cleanup plan JSON file", "cleanup-plan.json"))
    path.write_text(render_cleanup_plan_json(groups), encoding="utf-8")
    print(f"Cleanup plan written to: {path}")
    print("No files were moved.")
    return 0

def ask_choice(prompt: str, default: str) -> str:
    value = input(f"{prompt} [{default}]: ").strip()
    return value or default


def ask_yes_no(prompt: str, default: bool) -> bool:
    suffix = "Y/n" if default else "y/N"
    answer = ask_choice(f"{prompt} ({suffix})", "y" if default else "n").strip().lower()
    return answer in {"y", "yes", "д", "да"}


def run_wizard(args) -> None:
    print("DupeLens setup")
    print("Scanning: " + ", ".join(str(p) for p in args.paths))
    print()

    args.recursive = ask_yes_no("Recurse into subfolders?", args.recursive)

    if args.min_size is None:
        print("\nScan size mode:")
        print("  1) Quick     - files >= 10MB, fastest, largest cleanup wins")
        print("  2) Balanced  - files >= 1MB, recommended default")
        print("  3) Deep      - files >= 1 byte, slow/noisy exhaustive scan")
        choice = ask_choice("Choose mode", "2").strip().lower()
        if choice in {"1", "quick", "q"}:
            args.mode = "quick"
        elif choice in {"3", "deep", "d"}:
            args.mode = "deep"
        else:
            args.mode = "balanced"

    show_progress = ask_yes_no("Show live progress?", not args.quiet)
    args.quiet = not show_progress

    print("\nOutput:")
    print("  1) Summary only")
    print("  2) Save full text report to file")
    print("  3) Save full JSON report to file")
    print("  4) Print capped preview in terminal")
    print("  5) Create safe cleanup plan JSON")
    output_choice = ask_choice("Choose output", "1").strip().lower()
    if output_choice in {"2", "text", "txt"}:
        args.summary = False
        args.json = False
        args.output = Path(ask_choice("Report file", "duplicates.txt"))
    elif output_choice in {"3", "json"}:
        args.summary = False
        args.json = True
        args.output = Path(ask_choice("JSON file", "duplicates.json"))
    elif output_choice in {"4", "preview"}:
        args.summary = False
        args.json = False
        raw = ask_choice("Max groups to print, 0 = all", "50")
        args.max_groups = int(raw) if raw.isdigit() else 50
    elif output_choice in {"5", "plan"}:
        args.summary = False
        args.json = False
        args.plan = Path(ask_choice("Cleanup plan JSON file", "cleanup-plan.json"))
    else:
        args.summary = True
        args.output = None
        args.json = False

    min_size = args.min_size if args.min_size is not None else mode_min_size(args.mode)
    print("\nPlan:")
    print(f"  recursive: {args.recursive}")
    print(f"  mode: {args.mode}; min-size: {format_bytes(min_size)}")
    print(f"  progress: {'hidden' if args.quiet else 'shown'}")
    print(f"  output: {'cleanup plan: ' + str(args.plan) if args.plan else (args.output if args.output else ('summary only' if args.summary else 'terminal preview'))}")
    print()
    if not ask_yes_no("Start scan now?", True):
        raise SystemExit("scan cancelled")


def main() -> int:
    parser = argparse.ArgumentParser(description="Find exact duplicate files safely (read-only).")
    parser.add_argument("paths", nargs="*", type=Path)
    parser.add_argument("-r", "--recursive", action="store_true", help="recurse into subdirectories")
    parser.add_argument("-i", "--interactive", "--wizard", action="store_true", help="ask scan options interactively")
    parser.add_argument("--clean", action="store_true", help="scan + preview + optional Move to Trash flow")
    parser.add_argument("--compare", action="store_true", help="compare roots: keep files from the first path, only report duplicates in later paths")
    parser.add_argument("--include-hidden", action="store_true", help="include dotfiles and hidden dirs")
    parser.add_argument("--follow-symlinks", action="store_true", help="follow symlinks")
    parser.add_argument("--mode", choices=("quick", "balanced", "deep"), default="balanced", help="scan mode: quick=10MB, balanced=1MB, deep=1 byte; default: balanced")
    parser.add_argument("--min-size", type=parse_size, default=None, help="override minimum file size: bytes or 10KB/10MB/1GB")
    parser.add_argument("--no-byte-verify", action="store_true", help="skip final byte-by-byte verification")
    parser.add_argument("--json", action="store_true", help="print JSON")
    parser.add_argument("-o", "--output", type=Path, help="write full report to file instead of terminal")
    parser.add_argument("--plan", type=Path, help="write safe cleanup plan JSON; does not delete files")
    parser.add_argument("--apply-plan", type=Path, help="move plan candidates to Trash; requires --confirm \"MOVE TO TRASH\"")
    parser.add_argument("--confirm", help="required confirmation for --apply-plan")
    parser.add_argument("--max-groups", type=int, default=50, help="max groups printed to terminal; 0 means all; default: 50")
    parser.add_argument("--summary", action="store_true", help="print only summary to terminal")
    parser.add_argument("--quiet", action="store_true", help="hide progress output")
    args = parser.parse_args()
    if args.apply_plan:
        return apply_cleanup_plan(args.apply_plan, args.confirm)
    if not args.paths:
        parser.error("missing path")
    if args.compare and len(args.paths) < 2:
        parser.error("--compare requires at least two paths: PRIMARY SECONDARY [MORE...]" )
    if args.clean:
        args.summary = False
        args.output = None
        args.plan = None
        args.json = False
        run_clean_setup(args)
    elif args.interactive:
        run_wizard(args)

    started = time.monotonic()
    min_size = args.min_size if args.min_size is not None else mode_min_size(args.mode)
    progress = Progress(quiet=args.quiet)
    groups = find_duplicates(args.paths, recursive=args.recursive, include_hidden=args.include_hidden, follow_symlinks=args.follow_symlinks, min_size=min_size, verify_bytes=not args.no_byte_verify, progress=progress)
    if args.compare:
        groups = filter_compare_groups(groups, args.paths[0])
    elapsed_text = format_duration(time.monotonic() - started)
    total_wasted = sum(g.wasted_bytes for g in groups)
    if args.clean:
        return run_clean_flow(args, groups, elapsed_text, total_wasted, min_size)
    if args.plan:
        args.plan.write_text(render_cleanup_plan_json(groups), encoding="utf-8")
        print(f"Found {len(groups)} duplicate groups; reclaimable: {total_wasted} bytes ({format_bytes(total_wasted)})")
        print(f"Scan mode: {args.mode}; min-size: {format_bytes(min_size)}")
        if args.compare:
            print(f"Compare mode: keep primary root untouched: {args.paths[0]}")
        print(f"Scan time: {elapsed_text}")
        print(f"Cleanup plan written to: {args.plan}")
        print("No files were deleted. Review the plan before applying it.")
        return 0

    if args.output:
        report = render_json(groups) if args.json else render_text(groups, max_groups=None)
        args.output.write_text(report, encoding="utf-8")
        print(f"Found {len(groups)} duplicate groups; reclaimable: {total_wasted} bytes ({format_bytes(total_wasted)})")
        print(f"Scan mode: {args.mode}; min-size: {format_bytes(min_size)}")
        if args.compare:
            print(f"Compare mode: keep primary root untouched: {args.paths[0]}")
        print(f"Scan time: {elapsed_text}")
        print(f"Full report written to: {args.output}")
        return 0

    if args.summary:
        print(f"Found {len(groups)} duplicate groups; reclaimable: {total_wasted} bytes ({format_bytes(total_wasted)})")
        print(f"Scan mode: {args.mode}; min-size: {format_bytes(min_size)}")
        if args.compare:
            print(f"Compare mode: keep primary root untouched: {args.paths[0]}")
        print(f"Scan time: {elapsed_text}")
        print("Run again with -o duplicates.txt for the full report.")
        return 0

    if args.json:
        print(render_json(groups), end="")
    else:
        max_groups = None if args.max_groups == 0 else max(0, args.max_groups)
        print(render_text(groups, max_groups=max_groups), end="")
        print(f"Scan mode: {args.mode}; min-size: {format_bytes(min_size)}")
        if args.compare:
            print(f"Compare mode: keep primary root untouched: {args.paths[0]}")
        print(f"Scan time: {elapsed_text}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
