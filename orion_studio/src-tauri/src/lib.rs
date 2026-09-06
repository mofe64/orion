mod pairing;
mod agent;
mod coordinator;
mod settings;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(coordinator::CoordinatorManager::default())
        .manage(agent::AgentManager::default())
        .invoke_handler(tauri::generate_handler![
            coordinator::start_voice_worker,
            settings::load_voice_settings,
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
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                use tauri::Manager;
                app.state::<coordinator::CoordinatorManager>().shutdown();
                app.state::<agent::AgentManager>().shutdown();
            }
        });
}
