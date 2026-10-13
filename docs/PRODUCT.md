# Product Direction

## Positioning

DupeLens is a duplicate-first cleanup tool, not a Mac cleaner.

Primary promise:

> Clean duplicate files from old backups and imports without risking your main folder.

## Target users

- People with messy `Downloads` folders.
- People migrating from old Macs.
- People with external drives full of copied folders.
- Developers with project archives and repeated assets.
- Power users who want a reviewable cleanup plan before deleting anything.

## Differentiation

Compared with broad cleaner apps:

- Duplicate-first workflow.
- Compare roots: protect one folder, clean another.
- Transparent `KEEP` / `TRASH` recommendations.
- JSON cleanup plans for review and automation.
- CLI-first core that can later power GUI/TUI/native app.

Compared with generic duplicate finders:

- Safer cleanup plan by default.
- Strong backup/import workflow.
- Explicit primary-root protection.
- Practical summaries, not huge terminal dumps.

## Current MVP features

- Exact duplicate detection.
- Recursive scanning.
- Scan modes: quick, balanced, deep.
- Progress output.
- Text and JSON reports.
- Safe cleanup plan generation.
- Apply plan to Trash with exact confirmation.
- Clean wizard.
- Compare mode.
- Trash folder exclusion.
- Python prototype and Rust core.

## Near-term backlog

1. Unified JSON schema for GUI.
2. JSON progress events.
3. Stable group/file IDs.
4. Selective apply-plan support.
5. Local web UI prototype.
6. HTML/CSV reports.
7. Hash cache.
8. Hard-link/APFS clone awareness.
9. macOS native/Tauri packaging decision.
