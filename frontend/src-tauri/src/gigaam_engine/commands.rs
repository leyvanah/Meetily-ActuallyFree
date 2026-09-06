// gigaam_engine/commands.rs
//
// Tauri commands for the GigaAM engine, mirroring the Parakeet ones so the
// settings screen can drive both the same way.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{command, AppHandle, Emitter, Runtime};

use super::engine::{
    DownloadProgress, GigaamEngine, GigaamModelStatus, DOWNLOAD_CANCELLED_MESSAGE, MODEL_NAME,
};

/// Global engine, created on first use.
pub static GIGAAM_ENGINE: Mutex<Option<Arc<GigaamEngine>>> = Mutex::new(None);

static MODELS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Point the engine at the app's models directory during startup.
pub fn set_models_directory<R: Runtime>(app: &AppHandle<R>) {
    let _ = app; // portable build resolves models relative to the executable
    let models_dir = crate::paths::models_dir();

    if !models_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&models_dir) {
            log::error!("Failed to create models directory: {}", e);
            return;
        }
    }

    *MODELS_DIR.lock().unwrap() = Some(models_dir);
}

fn engine() -> Option<Arc<GigaamEngine>> {
    GIGAAM_ENGINE.lock().unwrap().as_ref().cloned()
}

#[command]
pub async fn gigaam_init() -> Result<(), String> {
    let mut guard = GIGAAM_ENGINE.lock().unwrap();
    if guard.is_some() {
        return Ok(());
    }

    let models_dir = MODELS_DIR.lock().unwrap().clone();
    let created = GigaamEngine::new_with_models_dir(models_dir)
        .map_err(|e| format!("Failed to initialize GigaAM engine: {}", e))?;
    *guard = Some(Arc::new(created));
    Ok(())
}

#[command]
pub async fn gigaam_get_model_status() -> Result<GigaamModelStatus, String> {
    gigaam_init().await?;
    let engine = engine().ok_or("GigaAM engine not initialized")?;
    Ok(engine.status().await)
}

#[command]
pub async fn gigaam_is_model_loaded() -> Result<bool, String> {
    let Some(engine) = engine() else {
        return Ok(false);
    };
    Ok(engine.is_model_loaded().await)
}

#[command]
pub async fn gigaam_load_model<R: Runtime>(app_handle: AppHandle<R>) -> Result<(), String> {
    gigaam_init().await?;
    let engine = engine().ok_or("GigaAM engine not initialized")?;

    // Free the built-in LLM before loading STT so they never share memory
    crate::audio::common::prepare_for_stt().await;

    let _ = app_handle.emit(
        "gigaam-model-loading-started",
        serde_json::json!({ "modelName": MODEL_NAME }),
    );

    match engine.load_model().await {
        Ok(()) => {
            crate::audio::common::mark_stt_activity();
            let _ = app_handle.emit(
                "gigaam-model-loading-completed",
                serde_json::json!({ "modelName": MODEL_NAME }),
            );
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            let _ = app_handle.emit(
                "gigaam-model-loading-failed",
                serde_json::json!({ "modelName": MODEL_NAME, "error": message }),
            );
            Err(message)
        }
    }
}

#[command]
pub async fn gigaam_unload_model() -> Result<bool, String> {
    let Some(engine) = engine() else {
        return Ok(false);
    };
    Ok(engine.unload_model().await)
}

#[command]
pub async fn gigaam_download_model<R: Runtime>(app_handle: AppHandle<R>) -> Result<(), String> {
    gigaam_init().await?;
    let engine = engine().ok_or("GigaAM engine not initialized")?;

    let emitter = app_handle.clone();
    let _ = app_handle.emit(
        "gigaam-model-download-started",
        serde_json::json!({ "modelName": MODEL_NAME }),
    );

    let result = engine
        .download_model(Some(Box::new(move |progress: DownloadProgress| {
            let _ = emitter.emit(
                "gigaam-model-download-progress",
                serde_json::json!({
                    "modelName": MODEL_NAME,
                    "progress": progress,
                }),
            );
        })))
        .await;

    match result {
        Ok(()) => {
            let _ = app_handle.emit(
                "gigaam-model-download-complete",
                serde_json::json!({ "modelName": MODEL_NAME }),
            );
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            let event = if message.contains(DOWNLOAD_CANCELLED_MESSAGE) {
                "gigaam-model-download-cancelled"
            } else {
                "gigaam-model-download-failed"
            };
            let _ = app_handle.emit(
                event,
                serde_json::json!({ "modelName": MODEL_NAME, "error": message }),
            );
            Err(message)
        }
    }
}

#[command]
pub async fn gigaam_cancel_download() -> Result<(), String> {
    if let Some(engine) = engine() {
        engine.cancel_download();
    }
    Ok(())
}

#[command]
pub async fn gigaam_delete_model() -> Result<(), String> {
    gigaam_init().await?;
    let engine = engine().ok_or("GigaAM engine not initialized")?;
    engine.delete_model().await.map_err(|e| e.to_string())
}
