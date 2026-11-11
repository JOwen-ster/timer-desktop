mod audio;
mod mute;
mod timers;

use std::sync::Arc;
use tauri::{
    Manager, State, WindowEvent,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use timers::{Engine, Snapshot};

#[tauri::command]
fn list_timers(engine: State<'_, Arc<Engine>>) -> Snapshot {
    engine.snapshot()
}

#[tauri::command]
fn create_timer(
    engine: State<'_, Arc<Engine>>,
    name: String,
    seconds: u64,
) -> Result<Snapshot, String> {
    engine.create(name, seconds)
}

#[tauri::command]
fn edit_timer(
    engine: State<'_, Arc<Engine>>,
    id: u64,
    name: String,
    seconds: u64,
) -> Result<Snapshot, String> {
    engine.edit(id, name, seconds)
}

#[tauri::command]
fn toggle_timer(engine: State<'_, Arc<Engine>>, id: u64) -> Result<Snapshot, String> {
    engine.toggle(id)
}

#[tauri::command]
fn adjust_timer(
    engine: State<'_, Arc<Engine>>,
    id: u64,
    seconds: u64,
    subtract: bool,
) -> Result<Snapshot, String> {
    engine.adjust(id, seconds, subtract)
}

#[tauri::command]
fn delete_timer(engine: State<'_, Arc<Engine>>, id: u64) -> Result<Snapshot, String> {
    engine.delete(id)
}

fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app)
        }))
        .setup(|app| {
            let path = app.path().app_data_dir()?.join("timers.json");
            app.manage(Engine::open(path, app.handle().clone()));
            let show = MenuItem::with_id(app, "show", "Show timers", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .expect("bundled application icon")
                        .clone(),
                )
                .tooltip("Timers — running in the background")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_window(app),
                    "quit" => {
                        if app.state::<Arc<Engine>>().shutdown().is_ok() {
                            app.exit(0);
                        } else {
                            show_window(app);
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) = window.hide() {
                    eprintln!("Could not hide timer window: {error}");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_timers,
            create_timer,
            edit_timer,
            toggle_timer,
            adjust_timer,
            delete_timer
        ])
        .build(tauri::generate_context!())
        .expect("error while building timer application");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if app.state::<Arc<Engine>>().shutdown().is_err() {
                api.prevent_exit();
                show_window(app);
            }
        }
    });
}
