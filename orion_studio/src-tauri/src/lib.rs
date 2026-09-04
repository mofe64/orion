mod pairing;
mod voice_worker;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(voice_worker::VoiceWorkerManager::default())
        .invoke_handler(tauri::generate_handler![
            voice_worker::start_voice_worker,
            voice_worker::stop_voice_worker,
            pairing::load_pairing,
            pairing::save_pairing,
            pairing::forget_pairing
        ])
        .run(tauri::generate_context!())
        .expect("error while running Orion Studio");
}
