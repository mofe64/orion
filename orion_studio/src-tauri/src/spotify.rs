use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub async fn spotify_sign_in(
    app: tauri::AppHandle,
    client_id: String,
) -> Result<serde_json::Value, String> {
    let sign_in = orion_service::spotify::begin(client_id).await?;
    app.opener()
        .open_url(&sign_in.url, None::<&str>)
        .map_err(|_| "Studio couldn't open your browser. Please try again.")?;
    sign_in.finish().await
}
