use std::collections::{BTreeMap, HashMap, VecDeque};
use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const CHUNK: usize = 1024 * 1024;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Clone, Debug)]
struct FileInfo {
    path: PathBuf,
    size: u64,
    mtime: u64,
}

#[derive(Clone, Debug)]
struct HashedFile {
    info: FileInfo,
    hash: u64,
}

#[derive(Clone, Debug)]
struct DuplicateGroup {
    size: u64,
    wasted_bytes: u64,
    hash: u64,
    files: Vec<FileInfo>,
}

#[derive(Clone, Debug)]
struct Config {
    paths: Vec<PathBuf>,
    recursive: bool,
    include_hidden: bool,
    follow_symlinks: bool,
    min_size: u64,
    scan_mode: String,
    no_byte_verify: bool,
    json: bool,
    ui_json: bool,
    json_events: bool,
    output: Option<PathBuf>,
    cleanup_plan: Option<PathBuf>,
    apply_plan: Option<PathBuf>,
    clean: bool,
    compare: bool,
    confirm: Option<String>,
    max_groups: Option<usize>,
    summary: bool,
    quiet: bool,
    threads: usize,
    interactive: bool,
}

struct Progress {
    quiet: bool,
    json_events: bool,
    started: Instant,
    last: Mutex<Instant>,
}

impl Progress {
    fn new(quiet: bool, json_events: bool) -> Self {
        Self { quiet, json_events, started: Instant::now(), last: Mutex::new(Instant::now() - Duration::from_secs(10)) }
    }

    fn update(&self, phase: &str, current: usize, total: Option<usize>, detail: &str, force: bool) {
        if self.quiet && !self.json_events {
            return;
        }
        let mut last = self.last.lock().unwrap();
        let now = Instant::now();
        if !force && now.duration_since(*last) < Duration::from_millis(350) {
            return;
        }
        *last = now;
        let elapsed = now.duration_since(self.started).as_secs_f32();
        if self.json_events {
            let total_json = total.map(|n| n.to_string()).unwrap_or_else(|| "null".to_string());
            eprintln!(
                "{{\"event\":\"progress\",\"phase\":\"{}\",\"done\":{},\"total\":{},\"detail\":\"{}\",\"elapsed_seconds\":{:.3}}}",
                json_escape(phase),
                current,
                total_json,
                json_escape(detail),
                elapsed
            );
            return;
        }
        if let Some(total) = total {
            let pct = if total == 0 { 100.0 } else { (current as f64 / total as f64) * 100.0 };
            eprint!("\r{}: {}/{} ({:5.1}%) | {} | {:.1}s", phase, current, total, pct, detail, elapsed);
        } else if current > 0 {
            eprint!("\r{}: {} | {} | {:.1}s", phase, current, detail, elapsed);
        } else {
            eprint!("\r{} | {} | {:.1}s", phase, detail, elapsed);
        }
        let _ = io::stderr().flush();
    }

    fn done(&self, msg: &str) {
        if self.json_events {
            let elapsed = self.started.elapsed().as_secs_f32();
            eprintln!("{{\"event\":\"phase_done\",\"message\":\"{}\",\"elapsed_seconds\":{:.3}}}", json_escape(msg), elapsed);
        } else if !self.quiet {
            eprintln!("\r{}", msg);
        }
    }
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(err) => {
            eprintln!("error: {}", err);
            std::process::exit(1);
        }
    }
}

fn run() -> io::Result<()> {
    let cfg = parse_args()?;
    if let Some(plan) = &cfg.apply_plan {
        return apply_cleanup_plan(plan, cfg.confirm.as_deref());
    }
    let total_started = Instant::now();
    let progress = Arc::new(Progress::new(cfg.quiet, cfg.json_events));
    let mut groups = find_duplicates(&cfg, progress)?;
    if cfg.compare {
        groups = filter_compare_groups(groups, &cfg.paths[0]);
    }
    let elapsed = total_started.elapsed();
    let elapsed_text = format_duration(elapsed);
    let total_wasted: u64 = groups.iter().map(|g| g.wasted_bytes).sum();
    if cfg.json_events {
        eprintln!("{{\"event\":\"done\",\"groups\":{},\"reclaimable_bytes\":{},\"elapsed_seconds\":{:.3}}}", groups.len(), total_wasted, elapsed.as_secs_f64());
    }

    if cfg.clean {
        return run_clean_flow(&cfg, &groups, elapsed_text, total_wasted);
    }

    if let Some(path) = &cfg.cleanup_plan {
        fs::write(path, render_cleanup_plan_json(&groups))?;
        println!("Found {} duplicate groups; reclaimable: {} ({})", groups.len(), total_wasted, format_bytes(total_wasted));
        println!("Scan mode: {}; min-size: {}", cfg.scan_mode, format_bytes(cfg.min_size));
        if cfg.compare {
            println!("Compare mode: keep primary root untouched: {}", cfg.paths[0].display());
        }
        println!("Scan time: {}", elapsed_text);
        println!("Cleanup plan written to: {}", path.display());
        println!("No files were deleted. Review the plan before applying it.");
        return Ok(());
    }

    if let Some(path) = &cfg.output {
        let report = if cfg.ui_json { render_ui_json(&cfg, &groups, elapsed) } else if cfg.json { render_json(&groups) } else { render_text(&groups, None) };
        fs::write(path, report)?;
        println!("Found {} duplicate groups; reclaimable: {} ({})", groups.len(), total_wasted, format_bytes(total_wasted));
        println!("Scan mode: {}; min-size: {}", cfg.scan_mode, format_bytes(cfg.min_size));
        if cfg.compare {
            println!("Compare mode: keep primary root untouched: {}", cfg.paths[0].display());
        }
        println!("Scan time: {}", elapsed_text);
        println!("Full report written to: {}", path.display());
        return Ok(());
    }

    if cfg.summary {
        println!("Found {} duplicate groups; reclaimable: {} ({})", groups.len(), total_wasted, format_bytes(total_wasted));
        println!("Scan mode: {}; min-size: {}", cfg.scan_mode, format_bytes(cfg.min_size));
        if cfg.compare {
            println!("Compare mode: keep primary root untouched: {}", cfg.paths[0].display());
        }
        println!("Scan time: {}", elapsed_text);
        println!("Run again with -o duplicates.txt for the full report.");
        return Ok(());
    }

    if cfg.ui_json {
        print!("{}", render_ui_json(&cfg, &groups, elapsed));
    } else if cfg.json {
        print!("{}", render_json(&groups));
    } else {
        print!("{}", render_text(&groups, cfg.max_groups));
        println!("Scan mode: {}; min-size: {}", cfg.scan_mode, format_bytes(cfg.min_size));
        if cfg.compare {
            println!("Compare mode: keep primary root untouched: {}", cfg.paths[0].display());
        }
        println!("Scan time: {}", elapsed_text);
    }
    Ok(())
}

fn parse_args() -> io::Result<Config> {
    let mut args = env::args().skip(1).peekable();
    let mut min_size_set = false;
    let mut cfg = Config {
        paths: Vec::new(),
        recursive: false,
        include_hidden: false,
        follow_symlinks: false,
        min_size: 1024 * 1024,
        scan_mode: "balanced".to_string(),
        no_byte_verify: false,
        json: false,
        ui_json: false,
        json_events: false,
        output: None,
        cleanup_plan: None,
        apply_plan: None,
        clean: false,
        compare: false,
        confirm: None,
        max_groups: Some(50),
        summary: false,
        quiet: false,
        threads: thread::available_parallelism().map(|n| n.get()).unwrap_or(4),
        interactive: false,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-r" | "--recursive" => cfg.recursive = true,
            "-i" | "--interactive" | "--wizard" => cfg.interactive = true,
            "--clean" | "clean" => cfg.clean = true,
            "--compare" => cfg.compare = true,
            "--include-hidden" => cfg.include_hidden = true,
            "--follow-symlinks" => cfg.follow_symlinks = true,
            "--no-byte-verify" => cfg.no_byte_verify = true,
            "--json" => cfg.json = true,
            "--ui-json" => { cfg.ui_json = true; cfg.json = true; }
            "--json-events" => cfg.json_events = true,
            "--summary" => cfg.summary = true,
            "--quiet" => cfg.quiet = true,
            "--min-size" => {
                cfg.min_size = parse_size(next_value(&mut args, "--min-size")?)?;
                min_size_set = true;
            }
            "--mode" => {
                let mode = next_value(&mut args, "--mode")?.to_lowercase();
                cfg.scan_mode = mode.clone();
                if !min_size_set {
                    cfg.min_size = match mode.as_str() {
                        "quick" => 10 * 1024 * 1024,
                        "balanced" => 1024 * 1024,
                        "deep" => 1,
                        _ => return Err(io_err("invalid --mode; use quick, balanced, or deep")),
                    };
                } else if !matches!(mode.as_str(), "quick" | "balanced" | "deep") {
                    return Err(io_err("invalid --mode; use quick, balanced, or deep"));
                }
            }
            "-o" | "--output" => cfg.output = Some(PathBuf::from(next_value(&mut args, "--output")?)),
            "--plan" => cfg.cleanup_plan = Some(PathBuf::from(next_value(&mut args, "--plan")?)),
            "--apply-plan" => cfg.apply_plan = Some(PathBuf::from(next_value(&mut args, "--apply-plan")?)),
            "--confirm" => cfg.confirm = Some(next_value(&mut args, "--confirm")?),
            "--max-groups" => {
                let n: usize = next_value(&mut args, "--max-groups")?.parse().map_err(|_| io_err("invalid --max-groups"))?;
                cfg.max_groups = if n == 0 { None } else { Some(n) };
            }
            "--threads" => {
                cfg.threads = next_value(&mut args, "--threads")?.parse().map_err(|_| io_err("invalid --threads"))?;
                cfg.threads = cfg.threads.max(1);
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            _ if arg.starts_with('-') => return Err(io_err(&format!("unknown option: {}", arg))),
            _ => cfg.paths.push(PathBuf::from(arg)),
        }
    }

    if cfg.apply_plan.is_none() && cfg.paths.is_empty() {
        print_help();
        return Err(io_err("missing path"));
    }
    if cfg.compare && cfg.paths.len() < 2 {
        return Err(io_err("--compare requires at least two paths: PRIMARY SECONDARY [MORE...]"));
    }
    if cfg.clean {
        cfg.recursive = true;
        cfg.quiet = false;
        cfg.summary = false;
        cfg.output = None;
        cfg.cleanup_plan = None;
        cfg.json = false;
        cfg.ui_json = false;
        run_clean_setup(&mut cfg, min_size_set)?;
    } else if cfg.interactive {
        run_wizard(&mut cfg, min_size_set)?;
    }
    Ok(cfg)
}



fn run_clean_setup(cfg: &mut Config, min_size_set: bool) -> io::Result<()> {
    println!("DupeLens clean wizard");
    println!("Scanning: {}", cfg.paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
    println!("This flow scans, shows the largest duplicate groups, then can move recommended copies to Trash.");
    println!();

    cfg.recursive = ask_yes_no("Recurse into subfolders?", true)?;
    if !min_size_set {
        println!("\nScan size mode:");
        println!("  1) Quick     - files >= 10MB, fastest, largest cleanup wins");
        println!("  2) Balanced  - files >= 1MB, recommended default");
        println!("  3) Deep      - files >= 1 byte, slow/noisy exhaustive scan");
        let choice = ask_choice("Choose mode", "2")?;
        match choice.trim().to_lowercase().as_str() {
            "1" | "quick" | "q" => { cfg.scan_mode = "quick".to_string(); cfg.min_size = 10 * 1024 * 1024; }
            "3" | "deep" | "d" => { cfg.scan_mode = "deep".to_string(); cfg.min_size = 1; }
            _ => { cfg.scan_mode = "balanced".to_string(); cfg.min_size = 1024 * 1024; }
        }
    }
    cfg.quiet = !ask_yes_no("Show live progress?", true)?;
    println!("\nPlan:");
    println!("  recursive: {}", cfg.recursive);
    println!("  mode: {}; min-size: {}", cfg.scan_mode, format_bytes(cfg.min_size));
    println!("  progress: {}", if cfg.quiet { "hidden" } else { "shown" });
    println!();
    if !ask_yes_no("Start scan now?", true)? {
        return Err(io_err("clean cancelled"));
    }
    Ok(())
}

fn run_clean_flow(cfg: &Config, groups: &[DuplicateGroup], elapsed_text: String, total_wasted: u64) -> io::Result<()> {
    println!("\nScan complete");
    println!("Found {} duplicate groups; reclaimable: {} ({})", groups.len(), total_wasted, format_bytes(total_wasted));
    println!("Scan mode: {}; min-size: {}", cfg.scan_mode, format_bytes(cfg.min_size));
    println!("Scan time: {}", elapsed_text);
    if groups.is_empty() {
        return Ok(());
    }
    println!("\nTop recommended cleanup groups:");
    print_clean_preview(groups, 20);
    println!("\nNext step:");
    println!("  1) Move all recommended duplicate copies to Trash now");
    println!("  2) Save cleanup plan JSON only");
    println!("  3) Exit without changes");
    let choice = ask_choice("Choose action", "2")?;
    match choice.trim().to_lowercase().as_str() {
        "1" | "move" | "trash" => {
            println!("\nSafety rule: DupeLens keeps one file per group and moves only duplicate copies.");
            let confirm = ask_choice("Type MOVE TO TRASH to continue", "")?;
            if confirm != "MOVE TO TRASH" {
                println!("Cancelled. No files were moved.");
                return Ok(());
            }
            let tmp_plan = env::temp_dir().join(format!("dupelens-clean-plan-{}.json", std::process::id()));
            fs::write(&tmp_plan, render_cleanup_plan_json(groups))?;
            let result = apply_cleanup_plan(&tmp_plan, Some("MOVE TO TRASH"));
            let _ = fs::remove_file(&tmp_plan);
            result
        }
        "3" | "exit" | "q" | "quit" => {
            println!("No files were moved.");
            Ok(())
        }
        _ => {
            let path = PathBuf::from(ask_choice("Cleanup plan JSON file", "cleanup-plan.json")?);
            fs::write(&path, render_cleanup_plan_json(groups))?;
            println!("Cleanup plan written to: {}", path.display());
            println!("No files were moved.");
            Ok(())
        }
    }
}

fn print_clean_preview(groups: &[DuplicateGroup], limit: usize) {
    for (idx, group) in groups.iter().take(limit).enumerate() {
        let keep = &group.files[0];
        println!("\n{}. {} files, reclaimable {} ({})", idx + 1, group.files.len(), group.wasted_bytes, format_bytes(group.wasted_bytes));
        println!("   KEEP: {}", keep.path.display());
        for file in group.files.iter().skip(1).take(5) {
            println!("   TRASH: {}", file.path.display());
        }
        if group.files.len() > 6 {
            println!("   ... {} more trash candidates", group.files.len() - 6);
        }
    }
    if groups.len() > limit {
        println!("\n... {} more groups not shown in preview", groups.len() - limit);
    }
}

fn run_wizard(cfg: &mut Config, min_size_set: bool) -> io::Result<()> {
    println!("DupeLens setup");
    println!("Scanning: {}", cfg.paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
    println!();

    if ask_yes_no("Recurse into subfolders?", cfg.recursive)? {
        cfg.recursive = true;
    } else {
        cfg.recursive = false;
    }

    if !min_size_set {
        println!("\nScan size mode:");
        println!("  1) Quick     - files >= 10MB, fastest, largest cleanup wins");
        println!("  2) Balanced  - files >= 1MB, recommended default");
        println!("  3) Deep      - files >= 1 byte, slow/noisy exhaustive scan");
        let choice = ask_choice("Choose mode", "2")?;
        match choice.trim() {
            "1" | "quick" | "q" => { cfg.scan_mode = "quick".to_string(); cfg.min_size = 10 * 1024 * 1024; }
            "3" | "deep" | "d" => { cfg.scan_mode = "deep".to_string(); cfg.min_size = 1; }
            _ => { cfg.scan_mode = "balanced".to_string(); cfg.min_size = 1024 * 1024; }
        }
    }

    cfg.quiet = !ask_yes_no("Show live progress?", !cfg.quiet)?;

    println!("\nOutput:");
    println!("  1) Summary only");
    println!("  2) Save full text report to file");
    println!("  3) Save full JSON report to file");
    println!("  4) Print capped preview in terminal");
    println!("  5) Create safe cleanup plan JSON");
    let output_choice = ask_choice("Choose output", "1")?;
    match output_choice.trim() {
        "2" | "text" | "txt" => {
            cfg.summary = false;
            cfg.json = false;
            cfg.output = Some(PathBuf::from(ask_choice("Report file", "duplicates.txt")?));
        }
        "3" | "json" => {
            cfg.summary = false;
            cfg.json = true;
            cfg.output = Some(PathBuf::from(ask_choice("JSON file", "duplicates.json")?));
        }
        "4" | "preview" => {
            cfg.summary = false;
            cfg.json = false;
            let n = ask_choice("Max groups to print, 0 = all", "50")?;
            let parsed = n.trim().parse::<usize>().unwrap_or(50);
            cfg.max_groups = if parsed == 0 { None } else { Some(parsed) };
        }
        "5" | "plan" => {
            cfg.summary = false;
            cfg.json = false;
            cfg.cleanup_plan = Some(PathBuf::from(ask_choice("Cleanup plan JSON file", "cleanup-plan.json")?));
        }
        _ => {
            cfg.summary = true;
            cfg.output = None;
            cfg.json = false;
        }
    }

    println!("\nPlan:");
    println!("  recursive: {}", cfg.recursive);
    println!("  mode: {}; min-size: {}", cfg.scan_mode, format_bytes(cfg.min_size));
    println!("  progress: {}", if cfg.quiet { "hidden" } else { "shown" });
    println!("  output: {}", if let Some(path) = &cfg.cleanup_plan { format!("cleanup plan: {}", path.display()) } else if let Some(path) = &cfg.output { path.display().to_string() } else if cfg.summary { "summary only".to_string() } else { "terminal preview".to_string() });
    println!();
    if !ask_yes_no("Start scan now?", true)? {
        return Err(io_err("scan cancelled"));
    }
    Ok(())
}

fn ask_choice(prompt: &str, default: &str) -> io::Result<String> {
    print!("{} [{}]: ", prompt, default);
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let value = line.trim();
    if value.is_empty() { Ok(default.to_string()) } else { Ok(value.to_string()) }
}

fn ask_yes_no(prompt: &str, default: bool) -> io::Result<bool> {
    let suffix = if default { "Y/n" } else { "y/N" };
    let answer = ask_choice(&format!("{} ({})", prompt, suffix), if default { "y" } else { "n" })?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes" | "д" | "да"))
}

fn next_value<I: Iterator<Item = String>>(args: &mut std::iter::Peekable<I>, name: &str) -> io::Result<String> {
    args.next().ok_or_else(|| io_err(&format!("missing value for {}", name)))
}

fn print_help() {
    println!("DupeLens Core - fast exact duplicate scanner\n\nUsage: dupelens-core [options] PATH...\n\nOptions:\n  -r, --recursive          Recurse into subdirectories\n  -i, --interactive        Ask scan options interactively\n  --clean                  Scan + preview + optional Move to Trash flow\n  --compare                Compare roots: keep files from first path, report duplicates in later paths\n  --include-hidden         Include dotfiles and hidden dirs\n  --follow-symlinks        Follow symlinks\n  --mode MODE              Scan mode: quick=10MB, balanced=1MB, deep=1 byte; default: balanced\n  --min-size SIZE          Override minimum file size: bytes or 10KB/10MB/1GB\n  --no-byte-verify         Skip final byte verification\n  --json                   Output legacy duplicate report JSON\n  --ui-json                Output GUI-ready dupelens-scan-v1 JSON\n  --json-events            Emit newline-delimited progress JSON to stderr\n  -o, --output FILE        Write full report to file\n  --plan FILE              Write safe cleanup plan JSON; does not delete files\n  --apply-plan FILE        Move plan candidates to Trash; requires --confirm \"MOVE TO TRASH\"\n  --confirm TEXT           Required confirmation for --apply-plan\n  --max-groups N           Max groups printed to terminal; 0 means all\n  --summary                Print only summary\n  --quiet                  Hide progress\n  --threads N              Hash worker threads\n  -h, --help               Show help");
}

fn parse_size(s: String) -> io::Result<u64> {
    let upper = s.trim().to_uppercase();
    let (num, mult) = if let Some(n) = upper.strip_suffix("KB") { (n, 1024) }
    else if let Some(n) = upper.strip_suffix("K") { (n, 1024) }
    else if let Some(n) = upper.strip_suffix("MB") { (n, 1024 * 1024) }
    else if let Some(n) = upper.strip_suffix("M") { (n, 1024 * 1024) }
    else if let Some(n) = upper.strip_suffix("GB") { (n, 1024 * 1024 * 1024) }
    else if let Some(n) = upper.strip_suffix("G") { (n, 1024 * 1024 * 1024) }
    else { (upper.as_str(), 1) };
    let value: u64 = num.trim().parse().map_err(|_| io_err("invalid size"))?;
    Ok(value.saturating_mul(mult))
}

fn find_duplicates(cfg: &Config, progress: Arc<Progress>) -> io::Result<Vec<DuplicateGroup>> {
    let files = scan_files(cfg, &progress)?;
    progress.done(&format!("Scanning files complete: kept {}", files.len()));

    let by_size = group_by_size(files);
    let size_candidates: usize = by_size.values().map(|v| v.len()).sum();
    progress.done(&format!("Grouping by size complete: {} candidate size groups, {} files", by_size.len(), size_candidates));

    let mut partial_input = Vec::with_capacity(size_candidates);
    for group in by_size.values() {
        partial_input.extend(group.iter().cloned());
    }
    let partial_hashed = parallel_hash(partial_input, true, cfg.threads, progress.clone(), "Partial hashing");
    let partial_groups = group_by_size_hash(partial_hashed);
    let mut full_input = Vec::new();
    for group in partial_groups.values() {
        if group.len() > 1 {
            full_input.extend(group.iter().map(|h| h.info.clone()));
        }
    }
    progress.done(&format!("Partial hashing complete: {} files still candidates", full_input.len()));

    let full_hashed = parallel_hash(full_input, false, cfg.threads, progress.clone(), "Full hashing");
    let full_groups = group_by_size_hash(full_hashed);
    let mut groups = Vec::new();
    for ((size, hash), hashed) in full_groups {
        if hashed.len() <= 1 {
            continue;
        }
        let mut files: Vec<FileInfo> = hashed.into_iter().map(|h| h.info).collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        if !cfg.no_byte_verify && !same_bytes(&files) {
            continue;
        }
        groups.push(DuplicateGroup { size, wasted_bytes: size.saturating_mul(files.len() as u64 - 1), hash, files });
    }
    groups.sort_by(|a, b| b.wasted_bytes.cmp(&a.wasted_bytes).then_with(|| a.hash.cmp(&b.hash)));
    progress.done(&format!("Full hashing complete: {} duplicate groups", groups.len()));
    Ok(groups)
}

fn scan_files(cfg: &Config, progress: &Progress) -> io::Result<Vec<FileInfo>> {
    let mut files = Vec::new();
    let mut scanned = 0usize;
    for root in &cfg.paths {
        scan_one(root, cfg, &mut scanned, &mut files, progress);
    }
    Ok(files)
}

fn scan_one(path: &Path, cfg: &Config, scanned: &mut usize, files: &mut Vec<FileInfo>, progress: &Progress) {
    if is_trash_path(path) {
        return;
    }
    progress.update("Scanning files", *scanned, None, &path.display().to_string(), false);
    let meta = if cfg.follow_symlinks { fs::metadata(path) } else { fs::symlink_metadata(path) };
    let Ok(meta) = meta else { return; };
    if meta.is_file() {
        *scanned += 1;
        maybe_push_file(path, meta, cfg, files);
        return;
    }
    if !meta.is_dir() {
        return;
    }
    let Ok(read_dir) = fs::read_dir(path) else { return; };
    for entry in read_dir.flatten() {
        let p = entry.path();
        if is_trash_path(&p) {
            continue;
        }
        if !cfg.include_hidden && is_hidden(&p) {
            continue;
        }
        let meta = if cfg.follow_symlinks { fs::metadata(&p) } else { fs::symlink_metadata(&p) };
        let Ok(meta) = meta else { continue; };
        if meta.is_file() {
            *scanned += 1;
            maybe_push_file(&p, meta, cfg, files);
            progress.update("Scanning files", *scanned, None, &format!("kept {}; {}", files.len(), p.display()), false);
        } else if meta.is_dir() && cfg.recursive {
            scan_one(&p, cfg, scanned, files, progress);
        }
    }
}

fn maybe_push_file(path: &Path, meta: fs::Metadata, cfg: &Config, files: &mut Vec<FileInfo>) {
    if is_trash_path(path) {
        return;
    }
    if meta.len() < cfg.min_size {
        return;
    }
    if !cfg.follow_symlinks && meta.file_type().is_symlink() {
        return;
    }
    files.push(FileInfo { path: path.to_path_buf(), size: meta.len(), mtime: mtime_secs(&meta) });
}

fn mtime_secs(meta: &fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn is_hidden(path: &Path) -> bool {
    path.file_name().and_then(|s| s.to_str()).map(|s| s.starts_with('.')).unwrap_or(false)
}

fn is_trash_path(path: &Path) -> bool {
    let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    if parts.iter().any(|part| part == ".Trash" || part == ".Trashes") {
        return true;
    }
    parts.windows(4).any(|w| w[0] == ".local" && w[1] == "share" && w[2] == "Trash" && w[3] == "files")
}

fn group_by_size(files: Vec<FileInfo>) -> BTreeMap<u64, Vec<FileInfo>> {
    let mut map: BTreeMap<u64, Vec<FileInfo>> = BTreeMap::new();
    for file in files {
        map.entry(file.size).or_default().push(file);
    }
    map.retain(|_, v| v.len() > 1);
    map
}

fn group_by_size_hash(files: Vec<HashedFile>) -> HashMap<(u64, u64), Vec<HashedFile>> {
    let mut map: HashMap<(u64, u64), Vec<HashedFile>> = HashMap::new();
    for file in files {
        map.entry((file.info.size, file.hash)).or_default().push(file);
    }
    map.retain(|_, v| v.len() > 1);
    map
}

fn parallel_hash(files: Vec<FileInfo>, partial: bool, threads: usize, progress: Arc<Progress>, phase: &'static str) -> Vec<HashedFile> {
    let total = files.len();
    if total == 0 {
        return Vec::new();
    }
    let queue = Arc::new(Mutex::new(VecDeque::from(files)));
    let (tx, rx) = mpsc::channel();
    let done = Arc::new(Mutex::new(0usize));
    let workers = threads.min(total).max(1);
    for _ in 0..workers {
        let queue = queue.clone();
        let tx = tx.clone();
        let progress = progress.clone();
        let done = done.clone();
        thread::spawn(move || loop {
            let item = { queue.lock().unwrap().pop_front() };
            let Some(info) = item else { break; };
            let hash = if partial { hash_partial(&info.path) } else { hash_full(&info.path) };
            let mut d = done.lock().unwrap();
            *d += 1;
            progress.update(phase, *d, Some(total), &info.path.display().to_string(), false);
            drop(d);
            if let Ok(hash) = hash {
                let _ = tx.send(HashedFile { info, hash });
            }
        });
    }
    drop(tx);
    rx.into_iter().collect()
}

fn hash_partial(path: &Path) -> io::Result<u64> {
    let mut f = File::open(path)?;
    let mut h = Fnv64::new();
    let mut buf = vec![0u8; CHUNK];
    let n = f.read(&mut buf)?;
    h.update(&buf[..n]);
    let size = f.metadata()?.len();
    if size > CHUNK as u64 {
        f.seek(SeekFrom::End(-(CHUNK as i64)))?;
        let n = f.read(&mut buf)?;
        h.update(b"DUPLENS-PARTIAL");
        h.update(&buf[..n]);
    }
    Ok(h.finish())
}

fn hash_full(path: &Path) -> io::Result<u64> {
    let mut f = File::open(path)?;
    let mut h = Fnv64::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Ok(h.finish())
}

struct Fnv64(u64);
impl Fnv64 {
    fn new() -> Self { Self(FNV_OFFSET) }
    fn update(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }
    fn finish(&self) -> u64 { self.0 }
}

fn same_bytes(files: &[FileInfo]) -> bool {
    if files.len() < 2 { return false; }
    let mut handles = Vec::new();
    for f in files {
        let Ok(handle) = File::open(&f.path) else { return false; };
        handles.push(handle);
    }
    let mut chunks = vec![vec![0u8; CHUNK]; handles.len()];
    loop {
        let mut sizes = Vec::with_capacity(handles.len());
        for (i, handle) in handles.iter_mut().enumerate() {
            let Ok(n) = handle.read(&mut chunks[i]) else { return false; };
            sizes.push(n);
        }
        if sizes[0] == 0 {
            return sizes.iter().all(|&n| n == 0);
        }
        if sizes.iter().any(|&n| n != sizes[0]) {
            return false;
        }
        for i in 1..chunks.len() {
            if chunks[i][..sizes[i]] != chunks[0][..sizes[0]] {
                return false;
            }
        }
    }
}

fn path_under(path: &Path, root: &Path) -> bool {
    let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let abs_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    abs_path.starts_with(abs_root)
}

fn filter_compare_groups(groups: Vec<DuplicateGroup>, primary_root: &Path) -> Vec<DuplicateGroup> {
    let mut out = Vec::new();
    for group in groups {
        let mut primary_files: Vec<FileInfo> = group.files.iter().filter(|f| path_under(&f.path, primary_root)).cloned().collect();
        let mut other_files: Vec<FileInfo> = group.files.iter().filter(|f| !path_under(&f.path, primary_root)).cloned().collect();
        if primary_files.is_empty() || other_files.is_empty() {
            continue;
        }
        primary_files.sort_by(|a, b| a.path.cmp(&b.path));
        other_files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut files = Vec::with_capacity(1 + other_files.len());
        files.push(primary_files[0].clone());
        files.extend(other_files.iter().cloned());
        out.push(DuplicateGroup {
            size: group.size,
            wasted_bytes: group.size.saturating_mul(other_files.len() as u64),
            hash: group.hash,
            files,
        });
    }
    out.sort_by(|a, b| b.wasted_bytes.cmp(&a.wasted_bytes).then_with(|| a.hash.cmp(&b.hash)));
    out
}

fn render_text(groups: &[DuplicateGroup], max_groups: Option<usize>) -> String {
    if groups.is_empty() {
        return "No exact duplicates found.\n".to_string();
    }
    let total_wasted: u64 = groups.iter().map(|g| g.wasted_bytes).sum();
    let shown_count = max_groups.unwrap_or(groups.len()).min(groups.len());
    let mut out = String::new();
    out.push_str(&format!("Found {} duplicate groups; reclaimable if keeping one per group: {} ({})\n", groups.len(), total_wasted, format_bytes(total_wasted)));
    out.push_str(&format!("Showing {} of {} groups\n", shown_count, groups.len()));
    for (idx, group) in groups.iter().take(shown_count).enumerate() {
        out.push_str(&format!("\nGroup {}: {} files, size={} ({}), wasted={} ({}), hash={:016x}\n", idx + 1, group.files.len(), group.size, format_bytes(group.size), group.wasted_bytes, format_bytes(group.wasted_bytes), group.hash));
        for f in &group.files {
            out.push_str(&format!("  {}\n", f.path.display()));
        }
    }
    if shown_count < groups.len() {
        out.push_str(&format!("\n... {} more groups omitted. Use --max-groups 0 or --output duplicates.txt for the full report.\n", groups.len() - shown_count));
    }
    out
}



fn apply_cleanup_plan(plan_path: &Path, confirm: Option<&str>) -> io::Result<()> {
    if confirm != Some("MOVE TO TRASH") {
        return Err(io_err("refusing to move files: pass --confirm \"MOVE TO TRASH\""));
    }
    let source = fs::read_to_string(plan_path)?;
    if !source.contains("\"schema\": \"dupelens-cleanup-plan-v1\"") || !source.contains("\"dry_run\": true") {
        return Err(io_err("not a DupeLens dry-run cleanup plan"));
    }
    let candidates = extract_move_to_trash_paths(&source)?;
    if candidates.is_empty() {
        println!("No move_to_trash candidates found. Nothing moved.");
        return Ok(());
    }
    let trash = trash_dir()?;
    fs::create_dir_all(&trash)?;
    let mut moved = 0usize;
    let mut skipped = 0usize;
    let mut missing = 0usize;
    let mut directories = 0usize;
    let mut errors = 0usize;
    let mut skipped_examples: Vec<String> = Vec::new();
    let mut error_examples: Vec<String> = Vec::new();
    for path_text in candidates {
        let src = PathBuf::from(&path_text);
        if !src.exists() {
            skipped += 1;
            missing += 1;
            if skipped_examples.len() < 10 {
                skipped_examples.push(format!("missing: {}", src.display()));
            }
            continue;
        }
        if src.is_dir() {
            skipped += 1;
            directories += 1;
            if skipped_examples.len() < 10 {
                skipped_examples.push(format!("directory: {}", src.display()));
            }
            continue;
        }
        let dest = unique_trash_path(&trash, &src);
        match fs::rename(&src, &dest) {
            Ok(()) => {
                moved += 1;
                if moved <= 10 {
                    println!("moved to trash: {} -> {}", src.display(), dest.display());
                }
            }
            Err(err) => {
                errors += 1;
                if error_examples.len() < 10 {
                    error_examples.push(format!("{}: {}", src.display(), err));
                }
            }
        }
    }
    if moved > 10 {
        println!("... {} more files moved", moved - 10);
    }
    if !skipped_examples.is_empty() {
        eprintln!("Skipped examples:");
        for item in &skipped_examples {
            eprintln!("  {}", item);
        }
        if skipped > skipped_examples.len() {
            eprintln!("  ... {} more skipped", skipped - skipped_examples.len());
        }
    }
    if !error_examples.is_empty() {
        eprintln!("Error examples:");
        for item in &error_examples {
            eprintln!("  {}", item);
        }
        if errors > error_examples.len() {
            eprintln!("  ... {} more errors", errors - error_examples.len());
        }
    }
    println!("Done. moved={}, skipped={}, missing={}, directories={}, errors={}, trash={}", moved, skipped, missing, directories, errors, trash.display());
    if moved == 0 && missing > 0 && errors == 0 {
        println!("Nothing moved. This usually means the cleanup plan was already applied or the files were moved elsewhere. Run --clean again to rescan current files.");
    }
    if errors > 0 {
        return Err(io_err("some files could not be moved"));
    }
    Ok(())
}

fn trash_dir() -> io::Result<PathBuf> {
    if let Ok(path) = env::var("DUPLENS_TRASH_DIR") {
        return Ok(PathBuf::from(path));
    }
    let home = env::var("HOME").map_err(|_| io_err("HOME is not set; cannot locate Trash"))?;
    if cfg!(target_os = "macos") {
        Ok(PathBuf::from(home).join(".Trash"))
    } else {
        Ok(PathBuf::from(home).join(".local/share/Trash/files"))
    }
}

fn unique_trash_path(trash: &Path, src: &Path) -> PathBuf {
    let name = src.file_name().and_then(|s| s.to_str()).unwrap_or("dupelens-file");
    let candidate = trash.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("dupelens-file");
    let ext = src.extension().and_then(|s| s.to_str());
    for i in 1..10_000 {
        let filename = if let Some(ext) = ext {
            format!("{}-dupelens-{}.{}", stem, i, ext)
        } else {
            format!("{}-dupelens-{}", stem, i)
        };
        let candidate = trash.join(filename);
        if !candidate.exists() {
            return candidate;
        }
    }
    trash.join(format!("dupelens-file-{}", now_millis()))
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn extract_move_to_trash_paths(source: &str) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    let marker = "\"move_to_trash\"";
    while let Some(rel) = source[pos..].find(marker) {
        let start = pos + rel;
        let Some(open_rel) = source[start..].find('[') else { return Err(io_err("malformed cleanup plan: missing array")); };
        let mut i = start + open_rel + 1;
        loop {
            skip_json_ws(source, &mut i);
            if i >= source.len() { return Err(io_err("malformed cleanup plan: unterminated array")); }
            let b = source.as_bytes()[i];
            if b == b']' { i += 1; break; }
            if b != b'"' { return Err(io_err("malformed cleanup plan: expected string path")); }
            let (value, next) = parse_json_string(source, i)?;
            out.push(value);
            i = next;
            skip_json_ws(source, &mut i);
            if i < source.len() && source.as_bytes()[i] == b',' { i += 1; }
        }
        pos = i;
    }
    Ok(out)
}

fn skip_json_ws(source: &str, i: &mut usize) {
    while *i < source.len() && source.as_bytes()[*i].is_ascii_whitespace() {
        *i += 1;
    }
}

fn parse_json_string(source: &str, start: usize) -> io::Result<(String, usize)> {
    let bytes = source.as_bytes();
    if start >= bytes.len() || bytes[start] != b'"' {
        return Err(io_err("expected JSON string"));
    }
    let mut out = String::new();
    let mut i = start + 1;
    let mut segment_start = i;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                out.push_str(&source[segment_start..i]);
                return Ok((out, i + 1));
            }
            b'\\' => {
                out.push_str(&source[segment_start..i]);
                i += 1;
                if i >= bytes.len() {
                    return Err(io_err("bad JSON escape"));
                }
                match bytes[i] {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => return Err(io_err("unicode escapes in paths are not supported by this minimal parser")),
                    _ => return Err(io_err("bad JSON escape")),
                }
                i += 1;
                segment_start = i;
                continue;
            }
            _ => {
                i += 1;
            }
        }
    }
    Err(io_err("unterminated JSON string"))
}

fn render_cleanup_plan_json(groups: &[DuplicateGroup]) -> String {
    let mut out = String::new();
    let total_wasted: u64 = groups.iter().map(|g| g.wasted_bytes).sum();
    out.push_str(&format!("{{\n  \"schema\": \"dupelens-cleanup-plan-v1\",\n  \"dry_run\": true,\n  \"action\": \"review_then_move_to_trash\",\n  \"total_reclaimable_bytes\": {},\n  \"groups\": [\n", total_wasted));
    for (i, g) in groups.iter().enumerate() {
        if i > 0 { out.push_str(",\n"); }
        let keep = &g.files[0];
        out.push_str(&format!("    {{\n      \"size\": {},\n      \"reclaimable_bytes\": {},\n      \"hash\": \"{:016x}\",\n      \"keep\": \"{}\",\n      \"move_to_trash\": [", g.size, g.wasted_bytes, g.hash, json_escape(&keep.path.display().to_string())));
        for (j, f) in g.files.iter().skip(1).enumerate() {
            if j > 0 { out.push_str(", "); }
            out.push_str(&format!("\"{}\"", json_escape(&f.path.display().to_string())));
        }
        out.push_str("\n      ]\n    }");
    }
    out.push_str("\n  ]\n}\n");
    out
}


fn render_ui_json(cfg: &Config, groups: &[DuplicateGroup], elapsed: Duration) -> String {
    let total_wasted: u64 = groups.iter().map(|g| g.wasted_bytes).sum();
    let total_files: usize = groups.iter().map(|g| g.files.len()).sum();
    let mode = if cfg.compare { "compare" } else { "scan" };
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"schema\": \"dupelens-scan-v1\",\n");
    out.push_str(&format!("  \"mode\": \"{}\",\n", mode));
    out.push_str(&format!("  \"scan_mode\": \"{}\",\n", json_escape(&cfg.scan_mode)));
    out.push_str(&format!("  \"min_size_bytes\": {},\n", cfg.min_size));
    out.push_str(&format!("  \"recursive\": {},\n", cfg.recursive));
    if cfg.compare {
        out.push_str(&format!("  \"primary_root\": \"{}\",\n", json_escape(&cfg.paths[0].display().to_string())));
    } else {
        out.push_str("  \"primary_root\": null,\n");
    }
    out.push_str("  \"roots\": [");
    for (i, path) in cfg.paths.iter().enumerate() {
        if i > 0 { out.push_str(", "); }
        out.push_str(&format!("\"{}\"", json_escape(&path.display().to_string())));
    }
    out.push_str("],\n");
    out.push_str(&format!("  \"summary\": {{\"groups\": {}, \"files\": {}, \"reclaimable_bytes\": {}, \"elapsed_seconds\": {:.3}}},\n", groups.len(), total_files, total_wasted, elapsed.as_secs_f64()));
    out.push_str("  \"groups\": [\n");
    for (i, g) in groups.iter().enumerate() {
        if i > 0 { out.push_str(",\n"); }
        let keep = &g.files[0];
        out.push_str(&format!("    {{\n      \"id\": \"{}\",\n      \"size\": {},\n      \"reclaimable_bytes\": {},\n      \"hash\": \"{:016x}\",\n      \"keep\": \"{}\",\n      \"trash_candidates\": [", group_id(g), g.size, g.wasted_bytes, g.hash, json_escape(&keep.path.display().to_string())));
        for (j, f) in g.files.iter().skip(1).enumerate() {
            if j > 0 { out.push_str(", "); }
            out.push_str(&format!("\"{}\"", json_escape(&f.path.display().to_string())));
        }
        out.push_str("],\n      \"files\": [\n");
        for (j, f) in g.files.iter().enumerate() {
            if j > 0 { out.push_str(",\n"); }
            let role = if j == 0 { "keep" } else { "trash_candidate" };
            out.push_str(&format!("        {{\"id\": \"{}\", \"path\": \"{}\", \"size\": {}, \"mtime\": {}, \"role\": \"{}\"}}", file_id(f), json_escape(&f.path.display().to_string()), f.size, f.mtime, role));
        }
        out.push_str("\n      ]\n    }");
    }
    out.push_str("\n  ]\n}\n");
    out
}

fn group_id(group: &DuplicateGroup) -> String {
    format!("{:016x}-{}", group.hash, group.size)
}

fn file_id(file: &FileInfo) -> String {
    let mut h = Fnv64::new();
    h.update(file.path.display().to_string().as_bytes());
    h.update(&file.size.to_le_bytes());
    h.update(&file.mtime.to_le_bytes());
    format!("{:016x}", h.finish())
}

fn render_json(groups: &[DuplicateGroup]) -> String {
    let mut out = String::new();
    let total_wasted: u64 = groups.iter().map(|g| g.wasted_bytes).sum();
    out.push_str(&format!("{{\n  \"total_wasted_bytes\": {},\n  \"groups\": [\n", total_wasted));
    for (i, g) in groups.iter().enumerate() {
        if i > 0 { out.push_str(",\n"); }
        out.push_str(&format!("    {{\n      \"size\": {},\n      \"wasted_bytes\": {},\n      \"hash\": \"{:016x}\",\n      \"files\": [\n", g.size, g.wasted_bytes, g.hash));
        for (j, f) in g.files.iter().enumerate() {
            if j > 0 { out.push_str(",\n"); }
            out.push_str(&format!("        {{\"path\": \"{}\", \"size\": {}, \"mtime\": {}}}", json_escape(&f.path.display().to_string()), f.size, f.mtime));
        }
        out.push_str("\n      ]\n    }");
    }
    out.push_str("\n  ]\n}\n");
    out
}

fn json_escape(s: &str) -> String {
    s.chars().flat_map(|c| match c {
        '"' => "\\\"".chars().collect::<Vec<_>>(),
        '\\' => "\\\\".chars().collect::<Vec<_>>(),
        '\n' => "\\n".chars().collect::<Vec<_>>(),
        '\r' => "\\r".chars().collect::<Vec<_>>(),
        '\t' => "\\t".chars().collect::<Vec<_>>(),
        _ => vec![c],
    }).collect()
}


fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let millis = duration.subsec_millis();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, seconds)
    } else {
        format!("{}.{:03}s", seconds, millis)
    }
}

fn format_bytes(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    for unit in units {
        if value < 1024.0 || unit == "TB" {
            if unit == "B" { return format!("{} B", n); }
            return format!("{:.1} {}", value, unit);
        }
        value /= 1024.0;
    }
    format!("{} B", n)
}

fn io_err(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg.to_string())
}
