mod backend;
pub mod cli;
#[cfg(feature = "desktop")]
mod commands;
mod download;
#[cfg(any(feature = "desktop", test))]
mod media;
mod process_control;
mod redaction;
mod runtime;
mod storage;
mod tools;
mod youtube_api_keys;

#[cfg(feature = "desktop")]
use tauri::{Manager, WindowEvent};

#[cfg(feature = "desktop")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::CloseRequested { .. }) {
                if let Some(state) = window.app_handle().try_state::<backend::AppState>() {
                    state.stop_all_processes();
                }
            }
        })
        .setup(|app| {
            app.manage(std::sync::Mutex::new(None::<media::MediaServer>));
            let runtime =
                runtime::Runtime::from_app(app.handle()).map_err(std::io::Error::other)?;
            let state =
                backend::AppState::new(&runtime.data_dir, true).map_err(std::io::Error::other)?;
            app.manage(state);

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::analyze_url,
            commands::analyze_formats,
            commands::start_download,
            commands::cancel_job,
            commands::list_jobs,
            commands::get_job,
            commands::open_output_path,
            commands::reveal_output_path,
            commands::create_video_thumbnail,
            commands::prepare_media_preview,
            commands::select_download_dir,
            commands::get_app_info,
            commands::check_app_update,
            commands::install_app_update,
            commands::check_tool_updates,
            commands::get_tool_platform,
            commands::install_tool_update,
            commands::get_settings,
            commands::update_settings,
            commands::list_youtube_api_keys,
            commands::add_youtube_api_key,
            commands::remove_youtube_api_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
