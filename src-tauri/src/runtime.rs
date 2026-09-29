use crate::backend::DownloadJobEvent;
use std::{path::PathBuf, sync::Arc};

/// The download engine needs paths and an event sink, without a GUI runtime.
#[derive(Clone)]
pub struct Runtime {
    pub data_dir: PathBuf,
    pub resource_dir: Option<PathBuf>,
    pub download_dir: PathBuf,
    pub on_event: Arc<dyn Fn(DownloadJobEvent) + Send + Sync>,
}

impl Runtime {
    #[cfg(feature = "desktop")]
    pub fn from_app(app: &tauri::AppHandle) -> Result<Self, String> {
        use tauri::{Emitter, Manager};
        let handle = app.clone();
        Ok(Self {
            data_dir: app.path().app_data_dir().map_err(|e| e.to_string())?,
            resource_dir: app.path().resource_dir().ok(),
            download_dir: app.path().download_dir().map_err(|e| e.to_string())?,
            on_event: Arc::new(move |event| {
                let _ = handle.emit("download:job-event", event);
            }),
        })
    }
}
