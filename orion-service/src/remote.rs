use crate::{Request, pairing};
use serde_json::{Value, json};
use std::time::Duration;

async fn json_response(mut response: reqwest::Response) -> Result<Value, String> {
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
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let base = pairing.url.trim_end_matches('/');
    let status = client
        .get(format!("{base}/api/v2/voice/status"))
        .bearer_auth(&pairing.token)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map_err(|_| "Could not reach Orion's voice service")?;
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
