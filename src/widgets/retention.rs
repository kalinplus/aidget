use std::ffi::{OsStr, OsString};
use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::common::error::{Error, Result};
use crate::common::utils::atomic_write;

pub const SETTINGS_FILE: &str = "settings.toml";
pub const DEFAULT_SCREENSHOT_RETENTION: usize = 30;
pub const DEFAULT_TRASH_RETENTION: usize = 20;
pub const DEFAULT_AUTO_DOWNLOAD: bool = true;

/// User-editable retention limits stored at the root of the aidget data directory.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Maximum number of managed PNG files under `state/screenshots/`.
    pub screenshot_retention: NonZeroUsize,
    /// Maximum number of widget directories under `trash/`.
    pub trash_retention: NonZeroUsize,
    /// Whether a newer release is downloaded without asking. Defaults on, the
    /// way Magpie ships it; a click on "Download update" downloads regardless.
    #[serde(default = "auto_download_default")]
    pub auto_download: bool,
}

fn auto_download_default() -> bool {
    DEFAULT_AUTO_DOWNLOAD
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            screenshot_retention: NonZeroUsize::new(DEFAULT_SCREENSHOT_RETENTION)
                .expect("default screenshot retention is non-zero"),
            trash_retention: NonZeroUsize::new(DEFAULT_TRASH_RETENTION)
                .expect("default trash retention is non-zero"),
            auto_download: DEFAULT_AUTO_DOWNLOAD,
        }
    }
}

/// Create `settings.toml` with defaults when absent, then validate its contents.
pub fn ensure(root: &Path) -> Result<()> {
    let path = settings_path(root);
    if !path.exists() {
        atomic_write(&path, default_document().as_bytes())?;
    }
    load(root).map(|_| ())
}

/// Read and strictly validate `settings.toml`.
pub fn load(root: &Path) -> Result<Settings> {
    let bytes = fs::read(settings_path(root))?;
    let text =
        std::str::from_utf8(&bytes).map_err(|error| Error::Other(format!("toml: {error}")))?;
    toml::from_str(text).map_err(|error| Error::Other(format!("toml: {error}")))
}

/// Remove the oldest managed PNGs until the screenshot directory fits `limit`.
pub fn prune_screenshots(dir: &Path, limit: NonZeroUsize) -> Result<()> {
    for (_, _, path) in excess_entries(entries(dir, RetentionKind::Screenshot)?, limit) {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Remove the oldest widget directories until `trash/` fits `limit`.
pub fn prune_trash(dir: &Path, limit: NonZeroUsize) -> Result<()> {
    for (_, _, path) in excess_entries(entries(dir, RetentionKind::Trash)?, limit) {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// Write a screenshot and then apply its directory's configured retention.
pub fn write_screenshot(
    root: &Path,
    screenshots_dir: &Path,
    path: &Path,
    bytes: &[u8],
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    let settings = load(root)?;
    prune_screenshots(screenshots_dir, settings.screenshot_retention)
}

fn settings_path(root: &Path) -> PathBuf {
    root.join(SETTINGS_FILE)
}

fn default_document() -> String {
    document(&Settings::default())
}

/// Render `settings` as the file a user edits, comments included.
fn document(settings: &Settings) -> String {
    format!(
        "# Maximum number of screenshots kept in state/screenshots.\n\
screenshot_retention = {}\n\
\n\
# Maximum number of deleted widgets kept in trash.\n\
trash_retention = {}\n\
\n\
# Download a newer release as soon as one is found.\n\
auto_download = {}\n",
        settings.screenshot_retention, settings.trash_retention, settings.auto_download
    )
}

/// Replace `settings.toml` with `settings`.
pub fn save(root: &Path, settings: &Settings) -> Result<()> {
    atomic_write(&settings_path(root), document(settings).as_bytes())
}

enum RetentionKind {
    Screenshot,
    Trash,
}

type RetentionEntry = (SystemTime, OsString, PathBuf);

fn entries(dir: &Path, kind: RetentionKind) -> Result<Vec<RetentionEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        // User-created files and symlinks in managed directories are not ours to remove.
        let managed = match kind {
            RetentionKind::Screenshot => {
                file_type.is_file() && entry.path().extension() == Some(OsStr::new("png"))
            }
            RetentionKind::Trash => file_type.is_dir(),
        };
        if managed {
            entries.push((
                entry.metadata()?.modified()?,
                entry.file_name(),
                entry.path(),
            ));
        }
    }
    // Retain the oldest first so callers can remove the excess prefix.
    entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    Ok(entries)
}

fn excess_entries(entries: Vec<RetentionEntry>, limit: NonZeroUsize) -> Vec<RetentionEntry> {
    let remove = entries.len().saturating_sub(limit.get());
    entries.into_iter().take(remove).collect()
}
