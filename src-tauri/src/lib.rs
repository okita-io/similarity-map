pub mod benchmark;
pub mod cancellation;
pub mod centroid;
pub mod clustering;
pub mod color;
pub mod commands;
pub mod display_state;
pub mod embedding;
pub mod events;
pub mod hash;
pub mod importer;
pub mod model;
pub mod pipeline;
pub mod rasterizer;
pub mod storage;
pub mod subcell;
pub mod types;
pub mod windowing;

use commands::*;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            check_document_session,
            restore_session,
            discard_job,
            ensure_embedding_model,
            estimate_analysis,
            analyze_document,
            cancel_analysis,
            resume_analysis,
            raster_pages,
            get_page_detail,
            get_cluster_registry,
            save_display_state,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
