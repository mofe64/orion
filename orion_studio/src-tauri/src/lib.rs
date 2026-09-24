mod agent;
mod coordinator;
mod pairing;
mod settings;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(orion_service::Backend)
        .invoke_handler(tauri::generate_handler![
            coordinator::start_voice_worker,
            agent::load_agent_profile,
            agent::change_agent_profile,
            settings::load_voice_settings,
            settings::load_voice_history,
            settings::save_voice_settings,
            settings::voice_model_locations,
            settings::choose_voice_folder,
            coordinator::set_voice_microphone,
            pairing::load_pairing,
            pairing::save_pairing,
            pairing::forget_pairing
        ])
        .build(tauri::generate_context!())
        .expect("error while building Orion Studio")
        .run(|_, _| {});
}
