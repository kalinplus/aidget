//! The Tauri side of the self-update feature.
//!
//! Commands, the launch check and the exit-time install live here so `main.rs`
//! stays the wiring it is meant to be. Policy that mixes the release feed with
//! the user's settings sits here too: `crate::update::UpdateService` knows only
//! about releases and downloads.

use crate::widgets::store::Store;
use tauri::Manager;

use crate::runtime::ui;

/// Swap a downloaded update in as the app quits.
///
/// Goes through the service so a failure lands on the same `Error` state and the
/// same message as an install the user asked for.
pub fn install_on_quit(handle: &tauri::AppHandle) {
    use tauri::Manager;

    let Some(service) = handle.try_state::<crate::update::UpdateService>() else {
        return;
    };
    if service.staged().is_none() {
        return;
    }
    // A distribution package is installed with the administrator's password.
    // Asking for it as the window disappears is worse than keeping the download:
    // Magpie does not ask on quit either, and the Install button tries again.
    if needs_administrator() {
        println!(
            "aidget: the downloaded update needs an administrator; it waits for the Install button"
        );
        return;
    }
    let snapshot = service.install_blocking();
    match &snapshot.error {
        Some(error) => eprintln!("aidget: cannot install on quit: {error}"),
        None => println!("aidget: installed the downloaded update on quit"),
    }
}

/// Whether installing what was downloaded needs elevation.
fn needs_administrator() -> bool {
    use crate::update::github::PackageKind;

    matches!(
        crate::update::target::current().map(|target| target.kind),
        Some(PackageKind::Deb | PackageKind::Rpm)
    )
}

/// Ask the release feed once, shortly after launch.
///
/// One check only: the update flow has no poller or scheduler, so every later
/// check comes from the Check button.
pub fn start_update_check(app: &tauri::AppHandle) {
    use tauri::Manager;

    let service = app.state::<crate::update::UpdateService>().inner().clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Let the windows and widgets settle before touching the network.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        check_and_maybe_download(&app, service).await;
    });
}

/// Run one check, then download when a newer release was found and the user left
/// `auto_download` on.
///
/// The policy lives here rather than in [`crate::update::UpdateService`] so the
/// service stays about the release feed and the download itself.
pub async fn check_and_maybe_download(
    app: &tauri::AppHandle,
    service: crate::update::UpdateService,
) -> crate::update::UpdateSnapshot {
    let started = std::time::Instant::now();
    let mut snapshot = service.check().await;
    if snapshot.state == crate::update::UpdateState::Found && auto_download_enabled(app) {
        snapshot = service.download().await;
    }
    let repository = crate::update::github::repository();
    match &snapshot.error {
        Some(error) => println!(
            "aidget: update check {:?} from {repository} in {:?}: {error}",
            snapshot.state,
            started.elapsed()
        ),
        None => println!(
            "aidget: update check {:?} from {repository} in {:?}",
            snapshot.state,
            started.elapsed()
        ),
    }
    snapshot
}

/// The data directory of the running app.
pub fn store_root(app: &tauri::AppHandle) -> std::path::PathBuf {
    use tauri::Manager;

    app.state::<ui::UiState>().root.lock().unwrap().clone()
}

/// Whether a newer release is downloaded without asking.
pub fn auto_download_enabled(app: &tauri::AppHandle) -> bool {
    match Store::new(store_root(app)).load_settings() {
        Ok(settings) => settings.auto_download,
        Err(error) => {
            eprintln!("aidget: cannot read settings: {error}");
            false
        }
    }
}

#[tauri::command]
pub fn update_state(
    service: tauri::State<'_, crate::update::UpdateService>,
) -> crate::update::UpdateSnapshot {
    service.snapshot()
}

#[tauri::command]
pub async fn update_check(app: tauri::AppHandle) -> crate::update::UpdateSnapshot {
    let service = app.state::<crate::update::UpdateService>().inner().clone();
    check_and_maybe_download(&app, service).await
}

#[tauri::command]
pub async fn update_download(app: tauri::AppHandle) -> crate::update::UpdateSnapshot {
    let service = app.state::<crate::update::UpdateService>().inner().clone();
    // Returns on `Downloading` so the window can draw progress; the work goes on
    // in the background.
    let snapshot = service.start_download();
    println!("aidget: update download {:?}", snapshot.state);
    snapshot
}

#[tauri::command]
pub async fn update_install(app: tauri::AppHandle) -> crate::update::UpdateSnapshot {
    use tauri::Manager;

    let service = app.state::<crate::update::UpdateService>().inner().clone();
    let started = std::time::Instant::now();
    let staged = service.staged();
    println!(
        "aidget: update install from {:?}",
        staged.as_ref().map(|path| path.display().to_string())
    );
    let snapshot = service.install().await;
    println!(
        "aidget: update install {:?} in {:?}",
        snapshot.state,
        started.elapsed()
    );
    if snapshot.error.is_none() {
        relaunch_and_exit(&app);
    }
    snapshot
}

/// Start the version that was just swapped in, then leave.
pub fn relaunch_and_exit(app: &tauri::AppHandle) {
    match crate::update::install::relaunch() {
        Ok(()) => app.exit(0),
        Err(error) => eprintln!("aidget: cannot relaunch: {error}"),
    }
}

#[tauri::command]
pub fn update_auto_download(app: tauri::AppHandle) -> std::result::Result<bool, String> {
    let root = store_root(&app);
    // The commands can run before, or without, the layout being created, so make
    // sure the defaults file exists instead of failing with a bare io error.
    crate::widgets::retention::ensure(&root).map_err(|error| error.to_string())?;
    Store::new(root)
        .load_settings()
        .map(|settings| settings.auto_download)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn update_set_auto_download(
    app: tauri::AppHandle,
    enabled: bool,
) -> std::result::Result<(), String> {
    let root = store_root(&app);
    crate::widgets::retention::ensure(&root).map_err(|error| error.to_string())?;
    let store = Store::new(root);
    let mut settings = store.load_settings().map_err(|error| error.to_string())?;
    settings.auto_download = enabled;
    store
        .save_settings(&settings)
        .map_err(|error| error.to_string())
}
