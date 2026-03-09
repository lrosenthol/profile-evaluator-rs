use std::fs;
use std::path::{Path, PathBuf};

use profile_evaluator_rs::evaluate_texts;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct SelectedFile {
    path: String,
    contents: String,
}

#[tauri::command]
fn select_and_load_file(kind: String) -> Result<Option<SelectedFile>, String> {
    let mut dialog = rfd::FileDialog::new();
    match kind.as_str() {
        "json" => {
            dialog = dialog.add_filter("JSON", &["json"]);
        }
        "yaml" => {
            dialog = dialog.add_filter("YAML", &["yaml", "yml"]);
        }
        _ => {}
    }

    let Some(path) = dialog.pick_file() else {
        return Ok(None);
    };

    let contents = fs::read_to_string(&path).map_err(|err| err.to_string())?;
    Ok(Some(SelectedFile {
        path: path.to_string_lossy().to_string(),
        contents,
    }))
}

#[tauri::command]
fn evaluate_profile(
    source_json: String,
    profile_yaml: String,
    profile_path: Option<String>,
) -> Result<serde_json::Value, String> {
    let include_base_dir = profile_path
        .as_deref()
        .map(Path::new)
        .and_then(Path::parent)
        .map(PathBuf::from);

    evaluate_texts(&profile_yaml, &source_json, include_base_dir.as_deref())
        .map_err(|err| err.to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            select_and_load_file,
            evaluate_profile
        ])
        .run(tauri::generate_context!())
        .expect("failed to run tauri application");
}
