use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Deserialize)]
struct ScanRequest {
    mode: String,
    primary: String,
    backup: Option<String>,
    scan_mode: String,
    recursive: bool,
    include_hidden: bool,
    include_dev_folders: bool,
}

const DEV_SEGMENTS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".cache",
    "__pycache__",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn core_binary() -> PathBuf {
    let name = if cfg!(windows) { "dupelens-core.exe" } else { "dupelens-core" };
    repo_root().join("core").join("target").join("release").join(name)
}

#[tauri::command]
async fn scan_duplicates(request: ScanRequest) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || run_scan(request))
        .await
        .map_err(|err| format!("Scan worker failed: {err}"))?
}

fn run_scan(request: ScanRequest) -> Result<Value, String> {
    let core = core_binary();
    if !core.exists() {
        return Err(format!(
            "Core binary missing. Run `cd core && cargo build --release` first. Expected: {}",
            core.display()
        ));
    }

    let mut cmd = Command::new(&core);
    if request.mode == "compare" {
        let backup = request
            .backup
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Backup/import folder is required for compare mode".to_string())?;
        cmd.arg("--compare");
        if request.recursive {
            cmd.arg("-r");
        }
        cmd.arg("--mode")
            .arg(&request.scan_mode)
            .arg("--quiet")
            .arg("--ui-json")
            .arg(request.primary.trim())
            .arg(backup);
    } else {
        if request.recursive {
            cmd.arg("-r");
        }
        cmd.arg("--mode")
            .arg(&request.scan_mode)
            .arg("--quiet")
            .arg("--ui-json")
            .arg(request.primary.trim());
    }
    if request.include_hidden {
        cmd.arg("--include-hidden");
    }

    let output = cmd
        .output()
        .map_err(|err| format!("Failed to start Rust core: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if stderr.is_empty() { stdout } else { stderr });
    }

    let mut result: Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("Rust core returned invalid JSON: {err}"))?;
    if !request.include_dev_folders {
        hide_developer_groups(&mut result);
    }
    Ok(result)
}

fn hide_developer_groups(result: &mut Value) {
    let Some(groups) = result.get_mut("groups").and_then(Value::as_array_mut) else {
        return;
    };
    let before = groups.len();
    groups.retain(|group| !group_contains_dev_path(group));
    let hidden = before.saturating_sub(groups.len());

    let files: u64 = groups
        .iter()
        .map(|group| group.get("files").and_then(Value::as_array).map(|files| files.len() as u64).unwrap_or(0))
        .sum();
    let reclaimable: u64 = groups
        .iter()
        .map(|group| group.get("reclaimable_bytes").and_then(Value::as_u64).unwrap_or(0))
        .sum();

    if let Some(summary) = result.get_mut("summary").and_then(Value::as_object_mut) {
        summary.insert("groups".to_string(), Value::from(groups.len() as u64));
        summary.insert("files".to_string(), Value::from(files));
        summary.insert("reclaimable_bytes".to_string(), Value::from(reclaimable));
        summary.insert("hidden_developer_groups".to_string(), Value::from(hidden as u64));
    }
}

fn group_contains_dev_path(group: &Value) -> bool {
    group
        .get("files")
        .and_then(Value::as_array)
        .map(|files| {
            files.iter().any(|file| {
                file.get("path")
                    .and_then(Value::as_str)
                    .map(path_has_dev_segment)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn path_has_dev_segment(path: &str) -> bool {
    path.split(['/', '\\'])
        .any(|segment| DEV_SEGMENTS.iter().any(|blocked| segment == *blocked))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![scan_duplicates])
        .run(tauri::generate_context!())
        .expect("error while running DupeLens desktop app");
}
