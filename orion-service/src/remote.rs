use crate::{Request, pairing};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Name lookup gets its own budget. On some networks macOS answers a `.local`
/// name only after waiting out a missing IPv4 record (about 5 s), which must
/// not consume the 3 s request timeout.
const RESOLVE_BUDGET: Duration = Duration::from_secs(10);

/// Addresses that last answered for each paired host, so later requests skip
/// the system resolver. An address that stops answering is looked up again.
fn address_cache() -> &'static Mutex<HashMap<String, Vec<SocketAddr>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<SocketAddr>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Build a client pinned to the gateway's addresses. IP-literal hosts need no
/// lookup. Returns whether the addresses came from the cache.
pub(crate) async fn gateway_client(base: &str, fresh: bool) -> Result<(reqwest::Client, bool), String> {
    let builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
    let url = url::Url::parse(base).map_err(|_| "Invalid Orion address")?;
    let host = match url.host() {
        Some(url::Host::Domain(host)) => host.to_owned(),
        _ => return Ok((builder.build().map_err(|e| e.to_string())?, false)),
    };
    let port = url.port_or_known_default().unwrap_or(80);
    let cached = if fresh {
        None
    } else {
        address_cache().lock().unwrap().get(&host).cloned()
    };
    let (addresses, from_cache) = match cached {
        Some(addresses) => (addresses, true),
        None => {
            let mut addresses: Vec<SocketAddr> = tokio::time::timeout(
                RESOLVE_BUDGET,
                tokio::net::lookup_host((host.as_str(), port)),
            )
            .await
            .map_err(|_| {
                format!(
                    "Could not reach Orion's voice service: {host} was not found on this network"
                )
            })?
            .map_err(|_| {
                format!(
                    "Could not reach Orion's voice service: {host} was not found on this network"
                )
            })?
            .collect();
            // Prefer IPv4: link-local IPv6 depends on the interface scope.
            addresses.sort_by_key(|address| !address.is_ipv4());
            addresses.dedup();
            if addresses.is_empty() {
                return Err(format!(
                    "Could not reach Orion's voice service: {host} was not found on this network"
                ));
            }
            (addresses, false)
        }
    };
    let client = builder
        .resolve_to_addrs(&host, &addresses)
        .build()
        .map_err(|e| e.to_string())?;
    address_cache().lock().unwrap().insert(host, addresses);
    Ok((client, from_cache))
}

fn forget_addresses(base: &str) {
    if let Ok(url) = url::Url::parse(base)
        && let Some(url::Host::Domain(host)) = url.host()
    {
        address_cache().lock().unwrap().remove(host);
    }
}

pub(crate) async fn json_response(mut response: reqwest::Response) -> Result<Value, String> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read Orion voice response")?
    {
        if body.len() + chunk.len() > 1024 * 1024 {
            return Err("Oversized Orion voice response".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| "Invalid Orion voice response".into())
}

pub async fn request(request: &Request) -> Result<Value, String> {
    let pairing = pairing::load_pairing()
        .await?
        .ok_or("Pair Orion to use its onboard voice service")?;
    request_paired(request, &pairing).await
}

async fn request_paired(request: &Request, pairing: &pairing::Pairing) -> Result<Value, String> {
    let base = pairing.url.trim_end_matches('/');
    let probe = |client: reqwest::Client| async move {
        client
            .get(format!("{base}/api/v2/voice/status"))
            .bearer_auth(&pairing.token)
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .map(|response| (client, response))
    };
    let (client, from_cache) = gateway_client(base, false).await?;
    let (client, status) = match probe(client).await {
        Ok(result) => result,
        // The lamp may have a new DHCP address; look its name up once more.
        Err(_) if from_cache => {
            forget_addresses(base);
            probe(gateway_client(base, true).await?.0).await.map_err(
                |_| "Could not reach Orion's voice service: found the lamp but it did not answer",
            )?
        }
        Err(_) => {
            forget_addresses(base);
            return Err(
                "Could not reach Orion's voice service: found the lamp but it did not answer"
                    .into(),
            );
        }
    };
    if status.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(
            "Orion does not advertise an onboard voice service. Install the Pi voice stack.".into(),
        );
    }
    let status: Value = json_response(
        status
            .error_for_status()
            .map_err(|_| "Could not authenticate Orion's voice service")?,
    )
    .await?;
    if status["onboard"] != true {
        return Err(
            "Orion does not advertise an onboard voice service. Install the Pi voice stack.".into(),
        );
    }
    let starting = matches!(request, Request::Start(_) | Request::StartSaved);
    let request = if starting {
        json!({"method":"start_saved"})
    } else {
        serde_json::to_value(request).map_err(|e| e.to_string())?
    };
    let response = client
        .post(format!("{base}/api/v2/voice/request"))
        .bearer_auth(&pairing.token)
        .timeout(Duration::from_secs(140))
        .json(&request)
        .send()
        .await
        .map_err(|_| "Orion's voice service did not respond")?;
    let success = response.status().is_success();
    let mut value: Value = json_response(response).await?;
    if !success {
        return Err(value["error"]["message"]
            .as_str()
            .unwrap_or("Orion voice request failed")
            .into());
    }
    if starting {
        value["url"] = format!("{base}/api/v2/voice/events").into();
        value["token"] = pairing.token.clone().into();
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn gateway(
        status: &'static str,
        payload: &'static str,
    ) -> (pairing::Pairing, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let pairing = pairing::Pairing {
            url: format!("http://{}", listener.local_addr().unwrap()),
            token: "a".repeat(32),
        };
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                bytes.push(stream.read_u8().await.unwrap());
            }
            let header = String::from_utf8(bytes).unwrap();
            assert!(header.starts_with("GET /api/v2/voice/status "));
            stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).as_bytes()).await.unwrap();
        });
        (pairing, task)
    }

    #[tokio::test]
    async fn missing_or_non_onboard_service_returns_error_without_local_fallback() {
        for (status, payload) in [("404 Not Found", "{}"), ("200 OK", r#"{"onboard":false}"#)] {
            let (pairing, task) = gateway(status, payload).await;
            let error = request_paired(&Request::StartSaved, &pairing)
                .await
                .unwrap_err();
            assert!(error.contains("Install the Pi voice stack"));
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn hostname_pairing_caches_and_recovers_a_stale_address() {
        let (mut pairing, task) = gateway("200 OK", r#"{"onboard":false}"#).await;
        let port = pairing.url.rsplit(':').next().unwrap().to_owned();
        pairing.url = format!("http://localhost:{port}");
        // A cached address that no longer answers (nothing listens on ::1).
        address_cache().lock().unwrap().insert(
            "localhost".into(),
            vec![format!("[::1]:{port}").parse().unwrap()],
        );
        let error = request_paired(&Request::StartSaved, &pairing)
            .await
            .unwrap_err();
        assert!(error.contains("Install the Pi voice stack"), "{error}");
        task.await.unwrap();
        let cached = address_cache().lock().unwrap()["localhost"].clone();
        assert!(cached[0].is_ipv4(), "{cached:?}");
    }

    #[tokio::test]
    async fn unavailable_pi_returns_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let pairing = pairing::Pairing {
            url: format!("http://{}", listener.local_addr().unwrap()),
            token: "a".repeat(32),
        };
        drop(listener);
        assert!(
            request_paired(&Request::StartSaved, &pairing)
                .await
                .unwrap_err()
                .contains("Could not reach")
        );
    }
}
