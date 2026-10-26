use orion_service::{Backend, Request, pairing::Pairing};

#[tauri::command]
pub async fn load_pairing(backend: tauri::State<'_, Backend>) -> Result<serde_json::Value, String> {
    backend.request(Request::LoadPairing).await
}
#[tauri::command]
pub async fn save_pairing(
    backend: tauri::State<'_, Backend>,
    pairing: Pairing,
) -> Result<serde_json::Value, String> {
    backend.request(Request::SavePairing(pairing)).await
}
#[tauri::command]
pub async fn forget_pairing(
    backend: tauri::State<'_, Backend>,
) -> Result<serde_json::Value, String> {
    backend.request(Request::ForgetPairing).await
}
#[tauri::command]
pub async fn list_pairings(
    backend: tauri::State<'_, Backend>,
) -> Result<serde_json::Value, String> {
    backend.request(Request::ListPairings).await
}
#[tauri::command]
pub async fn select_pairing(
    backend: tauri::State<'_, Backend>,
    url: String,
) -> Result<serde_json::Value, String> {
    backend.request(Request::SelectPairing(url)).await
}

/// Name lookups can return an IPv6 link-local address first, which macOS
/// drops on some Wi-Fi networks. Use the first IPv4 address when one exists.
#[tauri::command]
pub async fn resolve_gateway(url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || ipv4_origin(&url))
        .await
        .map_err(|error| error.to_string())
}

fn ipv4_origin(url: &str) -> String {
    use std::net::{IpAddr, ToSocketAddrs};
    let Some((scheme, authority)) = url.split_once("://") else {
        return url.to_owned();
    };
    let Some((host, port)) = authority.rsplit_once(':') else {
        return url.to_owned();
    };
    if host.starts_with('[') || host.parse::<IpAddr>().is_ok() {
        return url.to_owned();
    }
    let Ok(port) = port.parse::<u16>() else {
        return url.to_owned();
    };
    (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.find(|address| address.is_ipv4()))
        .map(|address| format!("{scheme}://{}:{port}", address.ip()))
        .unwrap_or_else(|| url.to_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_resolve_to_ipv4_and_addresses_stay() {
        assert_eq!(
            super::ipv4_origin("http://localhost:7447"),
            "http://127.0.0.1:7447"
        );
        assert_eq!(
            super::ipv4_origin("http://192.168.1.9:7447"),
            "http://192.168.1.9:7447"
        );
        assert_eq!(
            super::ipv4_origin("http://[fe80::1]:7447"),
            "http://[fe80::1]:7447"
        );
        assert_eq!(
            super::ipv4_origin("http://no-such-host.invalid:7447"),
            "http://no-such-host.invalid:7447"
        );
    }
}
