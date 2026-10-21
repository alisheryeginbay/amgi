//! Finding and opening the collection.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use amgi_core::{CardCollection, DB_FILE};

use crate::text;

/// The app's preferences: the bundle's, then a `swift run` build's, which is named after its executable.
const APP_DOMAINS: [&str; 2] = ["io.github.alisheryeginbay.amgi", "Amgi"];

pub fn open(dir: Option<&Path>) -> Result<Arc<CardCollection>, String> {
    let root = match dir {
        Some(dir) => dir.to_path_buf(),
        None => find_upwards().or_else(app_folder).ok_or(
            "no collection found here or in the app; pass --dir or set AMGI_DIR".to_string(),
        )?,
    };
    if !root.is_dir() {
        return Err(format!("{} is not a folder", root.display()));
    }
    let collection = CardCollection::open(root.to_string_lossy().into_owned()).map_err(|e| e.to_string())?;
    collection.set_utc_offset(text::utc_offset(text::now()));
    if let Some(hour) = app_setting("rolloverHour").and_then(|h| h.parse().ok()) {
        collection.set_rollover_hour(hour);
    }
    Ok(collection)
}

/// The nearest folder at or above the current one holding a review database, as git finds `.git`.
fn find_upwards() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    cwd.ancestors().find(|dir| dir.join(DB_FILE).is_file()).map(Path::to_path_buf)
}

/// The folder last opened in the app.
fn app_folder() -> Option<PathBuf> {
    APP_DOMAINS.iter().find_map(|domain| {
        let path = PathBuf::from(read_default(domain, "collectionPath")?);
        path.is_dir().then_some(path)
    })
}

/// A setting from the app, such as when the study day starts, from the first app that has it.
fn app_setting(key: &str) -> Option<String> {
    APP_DOMAINS.iter().find_map(|domain| read_default(domain, key))
}

fn read_default(domain: &str, key: &str) -> Option<String> {
    let out = Command::new("defaults").args(["read", domain, key]).output().ok()?;
    let value = String::from_utf8(out.stdout).ok()?;
    out.status.success().then(|| value.trim_end_matches('\n').to_string())
}
