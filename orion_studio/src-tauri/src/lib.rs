mod pairing;
mod voice_worker;
mod settings;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(voice_worker::VoiceWorkerManager::default())
        .invoke_handler(tauri::generate_handler![
            voice_worker::start_voice_worker,
            settings::load_voice_settings,
            settings::save_voice_settings,
            settings::voice_model_locations,
            settings::choose_voice_folder,
            voice_worker::set_voice_microphone,
            pairing::load_pairing,
            pairing::save_pairing,
            pairing::forget_pairing
        ])
        .build(tauri::generate_context!())
        .expect("error while building Orion Studio")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                use tauri::Manager;
                app.state::<voice_worker::VoiceWorkerManager>().shutdown();
            }
        });
}
