use aidget::widgets::retention;
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use tempfile::TempDir;

fn root() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    fs::create_dir_all(root.join("state/screenshots")).unwrap();
    fs::create_dir_all(root.join("trash")).unwrap();
    retention::ensure(&root).unwrap();
    (dir, root)
}

fn set_modified(path: &Path, timestamp: u64) {
    fs::File::open(path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(timestamp))
        .unwrap();
}

#[test]
fn settings_are_created_with_defaults_and_reject_invalid_shapes() {
    let (_dir, root) = root();
    assert_eq!(retention::SETTINGS_FILE, "settings.toml");
    let settings = retention::load(&root).unwrap();
    assert_eq!(settings.screenshot_retention.get(), 30);
    assert_eq!(settings.trash_retention.get(), 20);

    let path = root.join(retention::SETTINGS_FILE);
    let defaults = fs::read_to_string(&path).unwrap();
    assert!(defaults.contains("# Maximum number of screenshots"));
    assert!(defaults.contains("screenshot_retention = 30"));
    assert!(defaults.contains("trash_retention = 20"));
    for invalid in [
        "",
        "screenshot_retention = 30\ntrash_retention = 0\n",
        "screenshot_retention = 30\ntrash_retention = 20\nunknown = true\n",
        "screenshot_retention = 30\nscreenshot_retention = 10\ntrash_retention = 20\n",
        "not = [valid",
    ] {
        fs::write(&path, invalid).unwrap();
        assert!(retention::load(&root).is_err(), "{invalid}");
    }
}

#[test]
fn ensure_preserves_existing_toml_comments_and_formatting() {
    let (_dir, root) = root();
    let path = root.join(retention::SETTINGS_FILE);
    let custom = "\
# Keep enough screenshots for the current debugging session.\n\
screenshot_retention = 12\n\
\n\
# Pruned widgets remain recoverable for a shorter period.\n\
trash_retention = 7\n";
    fs::write(&path, custom).unwrap();

    retention::ensure(&root).unwrap();

    assert_eq!(fs::read_to_string(path).unwrap(), custom);
}

#[test]
fn screenshot_retention_keeps_newest_managed_pngs() {
    let (_dir, root) = root();
    let screenshots = root.join("state/screenshots");
    for index in 0..31 {
        let path = screenshots.join(format!("widget-20261022T0000{index:02}.png"));
        fs::write(&path, b"png").unwrap();
        set_modified(&path, 1_000 + index);
    }
    let note = screenshots.join("keep.txt");
    fs::write(&note, b"not a screenshot").unwrap();
    let outside = root.join("outside.png");
    fs::write(&outside, b"custom output").unwrap();
    let settings = retention::load(&root).unwrap();

    retention::prune_screenshots(&screenshots, settings.screenshot_retention).unwrap();
    retention::prune_screenshots(&screenshots, settings.screenshot_retention).unwrap();

    let screenshots: Vec<_> = fs::read_dir(&screenshots)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .collect();
    assert_eq!(screenshots.len(), 30);
    assert!(!screenshots
        .iter()
        .any(|entry| entry.file_name() == "widget-20261022T000000.png"));
    assert!(screenshots
        .iter()
        .any(|entry| entry.file_name() == "widget-20261022T000030.png"));
    assert!(note.exists());
    assert!(outside.exists());
}

#[test]
fn write_screenshot_prunes_managed_directory() {
    let (_dir, root) = root();
    let screenshots = root.join("state/screenshots");
    for index in 0..31 {
        let path = screenshots.join(format!("old-20261022T0000{index:02}.png"));
        fs::write(&path, b"png").unwrap();
        set_modified(&path, 1_000 + index);
    }
    let outside = root.join("custom.png");

    retention::write_screenshot(&root, &screenshots, &outside, b"new custom png").unwrap();

    let managed: Vec<_> = fs::read_dir(&screenshots)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .collect();
    assert_eq!(managed.len(), 30);
    assert!(!screenshots.join("old-20261022T000000.png").exists());
    assert!(outside.exists());
}

#[test]
fn trash_retention_keeps_newest_directories() {
    let (_dir, root) = root();
    let trash = root.join("trash");
    for index in 0..21 {
        let path = trash.join(format!("widget-20261022T0000{index:02}"));
        fs::create_dir(&path).unwrap();
        set_modified(&path, 1_000 + index);
    }
    let note = trash.join("keep.txt");
    fs::write(&note, b"not a trashed widget").unwrap();
    let settings = retention::load(&root).unwrap();

    retention::prune_trash(&trash, settings.trash_retention).unwrap();
    retention::prune_trash(&trash, settings.trash_retention).unwrap();

    let directories = fs::read_dir(&trash)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .count();
    assert_eq!(directories, 20);
    assert!(!trash.join("widget-20261022T000000").exists());
    assert!(trash.join("widget-20261022T000020").exists());
    assert!(note.exists());
}

#[test]
fn auto_download_defaults_on_and_survives_a_save() {
    let (_dir, root) = root();
    assert!(retention::load(&root).unwrap().auto_download);

    let mut settings = retention::load(&root).unwrap();
    settings.auto_download = false;
    retention::save(&root, &settings).unwrap();

    assert!(!retention::load(&root).unwrap().auto_download);
    assert!(fs::read_to_string(root.join(retention::SETTINGS_FILE))
        .unwrap()
        .contains("auto_download = false"));
}

#[test]
fn a_settings_file_without_auto_download_still_loads() {
    let (_dir, root) = root();
    fs::write(
        root.join(retention::SETTINGS_FILE),
        "screenshot_retention = 5\ntrash_retention = 5\n",
    )
    .unwrap();

    assert!(retention::load(&root).unwrap().auto_download);
}

#[test]
fn a_missing_settings_file_must_be_created_before_it_can_be_read() {
    let (dir, _root) = root();
    let fresh = dir.path().join("never-initialized");

    // Reading first fails the way the update commands used to: the layout was
    // never created, so there is no file to read.
    assert!(retention::load(&fresh).is_err());

    retention::ensure(&fresh).unwrap();
    assert!(retention::load(&fresh).unwrap().auto_download);
}
