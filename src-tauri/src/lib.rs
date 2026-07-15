pub mod analysis_output_store;
pub mod app_settings;
pub mod commands;
pub mod display_state;
pub mod events;
pub mod pipeline;
pub mod rasterizer;
pub mod results_catalog;

pub use similarity_core;

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
            get_repetition_report,
            get_visualization_payload,
            analyze_text,
            list_rf_chapters,
            build_rf_chapter_scope,
            analyze_rf_chapter,
            estimate_rf_chapter,
            get_app_settings,
            save_app_settings,
            save_display_state,
            list_document_results,
            save_document_result,
            save_document_result_as,
            delete_document_result,
            set_active_document_result,
            serialize_analysis_output,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
