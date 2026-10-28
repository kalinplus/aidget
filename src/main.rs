#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use std::sync::Arc;

use aidget::common::error::{Error, ErrorBody, Result};
use aidget::interfaces::cli::{Cli, Command};
use aidget::runtime::ui;
use aidget::widgets::store::Store;
use clap::error::ErrorKind;
use clap::Parser;
use tauri::Manager;

fn main() {
    let json = std::env::args_os().any(|argument| argument == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if json
                && !matches!(
                    error.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                ) =>
        {
            let body = serde_json::to_string(&serde_json::json!({
                "code": "invalid_params",
                "detail": error.to_string(),
            }))
            .expect("clap errors are serializable");
            eprintln!("{body}");
            std::process::exit(error.exit_code());
        }
        Err(error) => error.exit(),
    };
    let json = cli.json;
    let result = match cli.command {
        // Launching the bundled app (or `tauri dev`) passes no arguments.
        None => run_ui(true, false),
        Some(Command::Ui {
            background,
            no_main_window,
        }) => run_ui(!background && !no_main_window, background),
        Some(Command::Mcp) => run_mcp(),
        other => aidget::interfaces::cli::run(Cli {
            command: other,
            json,
        }),
    };
    if let Err(error) = result {
        if json {
            match serde_json::to_string(&ErrorBody::from(&error)) {
                Ok(body) => eprintln!("{body}"),
                Err(serialize_error) => eprintln!("aidget: {serialize_error}"),
            }
        } else {
            eprintln!("aidget: {error}");
        }
        std::process::exit(1);
    }
}

fn run_ui(show_main_window: bool, detach: bool) -> Result<()> {
    let store = Store::open_default();
    store.ensure_layout()?;
    store.prune_retained()?;
    aidget::runtime::process::publish_cli(store.root());
    let client = aidget::transport::client::Client::new(store.socket_path());
    if client.is_running() {
        if show_main_window {
            reopen_running_app(&client)?;
            return Ok(());
        }
        println!("aidget ui is already running");
        return Ok(());
    }
    if client.is_connectable() {
        // A socket that accepts but does not answer may belong to an instance
        // that is still starting. Never treat it as stale and bind over it.
        if client.wait_ready(std::time::Duration::from_secs(3)) {
            if show_main_window {
                reopen_running_app(&client)?;
            } else {
                println!("aidget ui is already running");
            }
            return Ok(());
        }
        return Err(Error::Other(
            "aidget ui socket is open but not responding; stop that process first".to_string(),
        ));
    }
    if detach {
        return aidget::runtime::process::spawn_ui();
    }

    aidget::agents::integration::ensure_skill(&store)?;
    let endpoint = aidget::transport::ipc::Endpoint::new(store.socket_path());
    let listener = endpoint
        .bind()
        .map_err(|error| Error::Other(format!("cannot bind socket: {error}")))?;
    aidget::widgets::template::bootstrap_welcome(&store)?;
    let store_for_ui = store.clone();
    let store_for_protocol = Arc::new(store.clone());
    let store_for_server = Arc::new(store.clone());

    let app = tauri::Builder::default()
        .manage(aidget::update::UpdateService::new())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["ui", "--no-main-window"]),
        ))
        .register_uri_scheme_protocol("aidget", move |ctx, request| {
            aidget::transport::server::protocol::handle(&ctx, request, &store_for_protocol)
        })
        .invoke_handler(tauri::generate_handler![
            widget_state,
            widget_loaded,
            widget_ready,
            persist_window_position,
            begin_widget_drag,
            start_widget_drag,
            end_widget_drag,
            main_state,
            main_painted,
            main_widget_cached_preview,
            set_widget_enabled,
            delete_widget,
            restore_widget,
            deleted_widget_preview,
            purge_deleted_widget,
            main_widget_preview,
            widget_location,
            widget_storage_get,
            widget_storage_set,
            widget_storage_remove,
            widget_storage_clear,
            integration_state,
            set_agent_integration,
            aidget::runtime::update_glue::update_state,
            aidget::runtime::update_glue::update_check,
            aidget::runtime::update_glue::update_download,
            aidget::runtime::update_glue::update_install,
            aidget::runtime::update_glue::update_auto_download,
            aidget::runtime::update_glue::update_set_auto_download
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(if show_main_window {
                tauri::ActivationPolicy::Regular
            } else {
                tauri::ActivationPolicy::Accessory
            });
            ui::start(app.handle(), store_for_ui.clone())?;
            if show_main_window {
                ui::show_main_now(app.handle())?;
            } else {
                ui::start_initial_widgets(app.handle())?;
            }
            aidget::transport::server::serve(
                app.handle().clone(),
                store_for_server,
                listener,
                endpoint,
            )?;
            tray(app.handle())?;
            aidget::runtime::update_glue::start_update_check(app.handle());
            if show_main_window {
                println!(
                    "aidget ui ready — data at {}",
                    store_for_ui.root().display()
                );
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .map_err(|error| Error::Other(error.to_string()))?;

    app.run(|handle, event| {
        match event {
            // A tray app must survive closing its last widget window.
            tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } => api.prevent_exit(),
            // A downloaded update also goes in as the app quits, so the next
            // launch is the new version even when the user never clicked
            // install.
            tauri::RunEvent::Exit => aidget::runtime::update_glue::install_on_quit(handle),
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                if let Err(error) = ui::show_main_now(handle) {
                    eprintln!("aidget: cannot open the main window: {error}");
                }
            }
            _ => {}
        }
    });
    Ok(())
}

/// Ask the running app to reopen its main window over the bridge.
fn reopen_running_app(client: &aidget::transport::client::Client) -> Result<()> {
    // Talk to the running instance directly. Falling back to `open` could
    // launch a second process with the same bundle id.
    client
        .call_with_timeout(
            "show_main",
            serde_json::json!({}),
            std::time::Duration::from_secs(3),
        )
        .map(|_| ())
}

fn run_mcp() -> Result<()> {
    let store = Store::open_default();
    store.ensure_layout()?;
    store.prune_retained()?;
    aidget::runtime::process::publish_cli(store.root());
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::Other(error.to_string()))?
        .block_on(aidget::interfaces::mcp::serve(store))
}

fn tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::TrayIconBuilder;

    let version = format!("版本 {}", app.package_info().version);
    let open = MenuItem::with_id(app, "open", "打开 aidget", true, None::<&str>)?;
    #[cfg(debug_assertions)]
    let restart = MenuItem::with_id(app, "restart", "重启 aidget (dev)", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let version = MenuItem::with_id(app, "version", version, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 aidget", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            #[cfg(debug_assertions)]
            &restart,
            &separator,
            &version,
            &quit,
        ],
    )?;

    let icon = tray_icon();
    let builder = TrayIconBuilder::with_id("aidget")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                if let Err(error) = ui::show_main_now(app) {
                    eprintln!("aidget: cannot open the main window: {error}");
                }
            }
            #[cfg(debug_assertions)]
            "restart" => app.request_restart(),
            "quit" => app.exit(0),
            _ => {}
        });
    // Template icons are macOS-only; other platforms draw the icon as-is.
    #[cfg(target_os = "macos")]
    let builder = builder.icon_as_template(true);
    builder.build(app)?;
    Ok(())
}

fn tray_icon() -> tauri::image::Image<'static> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let in_widget = (5..=26).contains(&x) && (7..=24).contains(&y);
            let in_chrome = in_widget && (7..=12).contains(&y);
            let border = in_widget && (x <= 6 || x >= 25 || y <= 8 || y >= 23);
            if in_chrome || border {
                let index = ((y * SIZE + x) * 4) as usize;
                rgba[index] = 0;
                rgba[index + 1] = 0;
                rgba[index + 2] = 0;
                rgba[index + 3] = 255;
            }
        }
    }
    tauri::image::Image::new_owned(rgba, SIZE, SIZE)
}

#[tauri::command]
fn widget_state(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> std::result::Result<ui::WidgetState, String> {
    ui::widget_payload(&app, &window).ok_or_else(|| "unknown widget window".to_string())
}

#[tauri::command]
fn widget_loaded(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    revision: u64,
) -> std::result::Result<(), String> {
    ui::mark_loaded(&app, &window, revision).map_err(|error| error.to_string())
}

#[tauri::command]
fn widget_ready(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    revision: u64,
) -> std::result::Result<(), String> {
    ui::mark_ready(&app, &window, revision).map_err(|error| error.to_string())
}

#[tauri::command]
fn persist_window_position(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> std::result::Result<(), String> {
    ui::persist_position(&app, &window).map_err(|error| error.to_string())
}

#[tauri::command]
fn begin_widget_drag(app: tauri::AppHandle) {
    ui::begin_drag(&app);
}

#[tauri::command]
async fn start_widget_drag(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> std::result::Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        ui::set_dragging_cursor(&app, true).map_err(|error| error.to_string())?;
        if let Err(error) = window.start_dragging() {
            let _ = ui::set_dragging_cursor(&app, false);
            return Err(error.to_string());
        }
        let released = ui::wait_for_drag_release(&app).await;
        let restored = ui::set_dragging_cursor(&app, false);
        released.map_err(|error| error.to_string())?;
        restored.map_err(|error| error.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        window.start_dragging().map_err(|error| error.to_string())
    }
}

#[tauri::command]
fn end_widget_drag(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> std::result::Result<(), String> {
    ui::end_drag(&app, &window).map_err(|error| error.to_string())
}

#[tauri::command]
fn main_state(app: tauri::AppHandle) -> std::result::Result<ui::MainState, String> {
    ui::main_payload(&app).map_err(|error| error.to_string())
}

/// Start desktop widget windows after the main page has painted its first frame.
#[tauri::command]
fn main_painted(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> std::result::Result<bool, String> {
    ui::mark_main_painted(&app, &window).map_err(|error| error.to_string())?;
    ui::start_initial_widgets(&app).map_err(|error| error.to_string())
}

/// Return a cached widget preview without capturing the widget WebView.
#[tauri::command]
async fn main_widget_cached_preview(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    id: String,
) -> std::result::Result<Option<String>, String> {
    ui::require_main_window_label(window.label()).map_err(|error| error.to_string())?;
    let root = app.state::<ui::UiState>().root.lock().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || ui::cached_preview(&Store::new(root), &id))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn integration_state() -> std::result::Result<aidget::agents::integration::IntegrationState, String>
{
    aidget::agents::integration::integration_state().map_err(|error| error.to_string())
}

#[tauri::command]
fn set_agent_integration(
    change: aidget::agents::integration::IntegrationChange,
) -> std::result::Result<aidget::agents::integration::IntegrationMutation, String> {
    aidget::agents::integration::set_agent_integration(change).map_err(|error| error.to_string())
}

#[tauri::command]
fn set_widget_enabled(
    app: tauri::AppHandle,
    id: String,
    enabled: bool,
) -> std::result::Result<(), String> {
    ui::set_widget_enabled(&app, &id, enabled).map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_widget(app: tauri::AppHandle, id: String) -> std::result::Result<(), String> {
    ui::delete_widget(&app, &id).map_err(|error| error.to_string())
}

#[tauri::command]
fn restore_widget(app: tauri::AppHandle, trash_name: String) -> std::result::Result<(), String> {
    ui::restore_widget(&app, &trash_name).map_err(|error| error.to_string())
}

#[tauri::command]
fn deleted_widget_preview(
    app: tauri::AppHandle,
    trash_name: String,
) -> std::result::Result<Option<String>, String> {
    ui::deleted_widget_preview(&app, &trash_name).map_err(|error| error.to_string())
}

#[tauri::command]
fn purge_deleted_widget(
    app: tauri::AppHandle,
    trash_name: String,
) -> std::result::Result<(), String> {
    ui::purge_deleted_widget(&app, &trash_name).map_err(|error| error.to_string())
}

#[tauri::command]
async fn main_widget_preview(
    app: tauri::AppHandle,
    id: String,
) -> std::result::Result<Option<String>, String> {
    ui::main_widget_preview(&app, &id)
        .await
        .map_err(|error| error.to_string())
}

/// Only the window label decides which widget asks; the page cannot name an id.
#[tauri::command]
async fn widget_location(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> std::result::Result<
    aidget::runtime::location::LocationFix,
    aidget::runtime::location::LocationFailure,
> {
    let id = ui::widget_id(&window)
        .map_err(|error| aidget::runtime::location::LocationFailure::from(&error))?;
    aidget::runtime::location::request(&app, &id)
        .await
        .map_err(|error| aidget::runtime::location::LocationFailure::from(&error))
}

fn widget_storage_context(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
) -> std::result::Result<(Store, String), aidget::widgets::storage::StorageFailure> {
    let id = ui::widget_id(window)
        .map_err(|error| aidget::widgets::storage::StorageFailure::from(&error))?;
    let root = app.state::<ui::UiState>().root.lock().unwrap().clone();
    let store = Store::new(root);
    aidget::widgets::storage::authorize(&store, &id)
        .map_err(|error| aidget::widgets::storage::StorageFailure::from(&error))?;
    Ok((store, id))
}

#[tauri::command]
fn widget_storage_get(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    key: String,
) -> std::result::Result<Option<String>, aidget::widgets::storage::StorageFailure> {
    let state = app.state::<ui::UiState>();
    let _guard = state.storage_lock.lock().unwrap();
    let (store, id) = widget_storage_context(&app, &window)?;
    aidget::widgets::storage::get(&store, &id, &key)
        .map_err(|error| aidget::widgets::storage::StorageFailure::from(&error))
}

#[tauri::command]
fn widget_storage_set(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    key: String,
    value: String,
) -> std::result::Result<(), aidget::widgets::storage::StorageFailure> {
    let state = app.state::<ui::UiState>();
    let _guard = state.storage_lock.lock().unwrap();
    let (store, id) = widget_storage_context(&app, &window)?;
    aidget::widgets::storage::set(&store, &id, &key, &value)
        .map_err(|error| aidget::widgets::storage::StorageFailure::from(&error))
}

#[tauri::command]
fn widget_storage_remove(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    key: String,
) -> std::result::Result<(), aidget::widgets::storage::StorageFailure> {
    let state = app.state::<ui::UiState>();
    let _guard = state.storage_lock.lock().unwrap();
    let (store, id) = widget_storage_context(&app, &window)?;
    aidget::widgets::storage::remove(&store, &id, &key)
        .map_err(|error| aidget::widgets::storage::StorageFailure::from(&error))
}

#[tauri::command]
fn widget_storage_clear(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> std::result::Result<(), aidget::widgets::storage::StorageFailure> {
    let state = app.state::<ui::UiState>();
    let _guard = state.storage_lock.lock().unwrap();
    let (store, id) = widget_storage_context(&app, &window)?;
    aidget::widgets::storage::clear(&store, &id)
        .map_err(|error| aidget::widgets::storage::StorageFailure::from(&error))
}
