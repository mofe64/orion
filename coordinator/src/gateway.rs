const AIM_LIGHT_TIMEOUT_SECONDS: u64 = 65;

use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Clone)]
pub(crate) struct Gateway {
    pub voice_resources: Arc<Mutex<()>>,
    client: reqwest::Client,
    url: String,
    token: String,
}
pub(crate) type ActiveRun = Arc<Mutex<Option<u64>>>;
impl Gateway {
    pub fn new(url: &str, token: &str) -> Result<Self, String> {
        Ok(Self {
            voice_resources: Arc::new(Mutex::new(())),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())?,
            url: url.trim_end_matches('/').into(),
            token: token.into(),
        })
    }
    pub async fn request(&self, path: &str, body: Option<Value>) -> Result<Value, String> {
        let request = match body {
            Some(body) => self.client.post(format!("{}{path}", self.url)).json(&body),
            None => self.client.get(format!("{}{path}", self.url)),
        };
        let response = request
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        self.decode(response).await
    }
    async fn decode(&self, response: reqwest::Response) -> Result<Value, String> {
        let status = response.status();
        let value = response.json::<Value>().await;
        if !status.is_success() {
            let message = value
                .as_ref()
                .ok()
                .and_then(|value| value["error"]["message"].as_str());
            return Err(message
                .map(|message| self.model_error(message))
                .unwrap_or_else(|| format!("HTTP {status}")));
        }
        value.map_err(|e| e.without_url().to_string())
    }
    pub async fn camera_snapshot(&self) -> Result<Vec<u8>, String> {
        // Only the explicit agent event reaches this endpoint. The gateway
        // requires its camera-viewer light lease before delivering any bytes.
        let mut response = self
            .client
            .get(format!("{}/api/v2/camera/snapshot", self.url))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "Camera snapshot unavailable (HTTP {})",
                response.status()
            ));
        }
        let maximum = orion_agent::tools::look::MAX_SNAPSHOT_BYTES;
        if response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            != Some("image/jpeg")
            || response
                .content_length()
                .is_some_and(|size| size > maximum as u64)
        {
            return Err("Camera returned an invalid or oversized JPEG".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| e.without_url().to_string())?
        {
            if bytes.len() + chunk.len() > maximum {
                return Err("Camera returned an oversized JPEG".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        orion_agent::tools::look::validate_snapshot(&bytes)?;
        Ok(bytes)
    }
    fn model_error(&self, message: &str) -> String {
        // A proxy error must not echo credentials or an authenticated endpoint.
        let mut safe = regex::Regex::new(r#"(?i)https?://[^\s\"<>]+"#)
            .unwrap()
            .replace_all(message, "[gateway]")
            .into_owned();
        if !self.token.is_empty() {
            safe = safe.replace(&self.token, "[redacted]");
        }
        safe
    }

    pub async fn upload(
        &self,
        path: &str,
        pcm: &[u8],
        request_id: &str,
        cues: &[orion_agent::ReactionCue],
    ) -> Result<Value, String> {
        if cues.len() > 4 {
            return Err("Too many speech cues on a chunk".into());
        }
        let mut request = self
            .client
            .post(format!("{}{path}", self.url))
            .bearer_auth(&self.token)
            .header("Content-Type", "audio/wav")
            .header("X-Orion-Voice-Request-ID", request_id)
            .body(wav(pcm));
        if !cues.is_empty() {
            request = request.header(
                "X-Orion-Speech-Cues",
                cues.iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        let response = request
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        self.decode(response).await
    }

    pub async fn set_lighting(&self, parameters: Value) -> Result<Value, String> {
        let result = self
            .request(
                "/api/v2/operations",
                Some(json!({"operation":"lamp_effect","settings":parameters})),
            )
            .await?;
        if result["accepted"] != true || result["result"]["ok"] != true {
            return Err(self.model_error(
                result["result"]["error"]
                    .as_str()
                    .unwrap_or("Pi did not confirm the lighting change"),
            ));
        }
        Ok(json!({"applied":true,"settings":parameters}))
    }
    pub async fn robot_operation(
        &self,
        mut parameters: Value,
        session: &str,
    ) -> Result<Value, String> {
        if parameters["operation"] == "sleep" {
            parameters["session_id"] = session.into();
        }
        let result = self.request("/api/v2/operations", Some(parameters)).await?;
        if result["accepted"] != true || result["result"]["ok"] != true {
            return Err(self.model_error(
                result["result"]["error"]
                    .as_str()
                    .unwrap_or("Pi did not accept the requested operation"),
            ));
        }
        Ok(result["result"].clone())
    }
    pub async fn aim_light(&self, mut parameters: Value, session: &str) -> Result<Value, String> {
        let resources = self.voice_resources.clone().try_lock_owned().map_err(
            |_| "Aiming must be requested before speech starts; please try after this reply",
        )?;
        parameters["session_id"] = session.into();
        let gateway = self.clone();
        // A cancelled voice task drops its JoinHandle, not the worker owning
        // the CPU gate. A later ASR job waits for remote lookup/recovery to end.
        tokio::spawn(async move {
            let _resources = resources;
            let response = gateway
                .client
                .post(format!("{}/api/v2/operations", gateway.url))
                .timeout(Duration::from_secs(AIM_LIGHT_TIMEOUT_SECONDS))
                .bearer_auth(&gateway.token)
                .json(&parameters)
                .send()
                .await
                .map_err(|e| e.without_url().to_string())?;
            let value = gateway.decode(response).await?;
            if value["accepted"] != true || value["result"]["ok"] != true {
                return Err(gateway.model_error(
                    value["result"]["error"]
                        .as_str()
                        .unwrap_or("Aiming was refused"),
                ));
            }
            Ok(value["result"].clone())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    pub async fn cancel(&self, active: &ActiveRun) {
        if let Some(run) = active.lock().await.take() {
            let _ = self
                .request(
                    "/api/v2/operations",
                    Some(json!({"operation":"cancel", "kind":"speech", "run_id":run})),
                )
                .await;
        }
    }
}
fn wav(pcm: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(44 + pcm.len());
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&24000u32.to_le_bytes());
    bytes.extend_from_slice(&48000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    bytes.extend_from_slice(pcm);
    bytes
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn snapshot_peer(headers: &str, body: Vec<u8>) -> (Gateway, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gateway = Gateway::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "secret",
        )
        .unwrap();
        let headers = headers.to_owned();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![];
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET /api/v2/camera/snapshot HTTP/1.1"));
            assert!(request.contains("authorization: Bearer secret"));
            stream.write_all(headers.as_bytes()).await.unwrap();
            // Oversized/malformed responses may be rejected before the body.
            let _ = stream.write_all(&body).await;
        });
        (gateway, task)
    }

    #[tokio::test]
    async fn snapshot_fetch_is_authenticated_bounded_and_jpeg_only() {
        let jpeg = vec![0xff, 0xd8, 0xff, 0, 0xff, 0xd9];
        let (gateway, peer) = snapshot_peer("HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: 6\r\nConnection: close\r\n\r\n", jpeg.clone()).await;
        assert_eq!(gateway.camera_snapshot().await.unwrap(), jpeg);
        peer.await.unwrap();
        for (headers, body) in [
            (
                "HTTP/1.1 503 Unavailable\r\nConnection: close\r\n\r\n",
                jpeg.clone(),
            ),
            (
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nConnection: close\r\n\r\n",
                vec![],
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n",
                jpeg,
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n",
                vec![],
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nConnection: close\r\n\r\n",
                b"not an image".to_vec(),
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nConnection: close\r\n\r\n",
                vec![0xff; 1048577],
            ),
        ] {
            let (gateway, peer) = snapshot_peer(headers, body).await;
            let error = gateway.camera_snapshot().await.unwrap_err();
            assert!(!error.contains("http://") && !error.contains("secret"));
            peer.await.unwrap();
        }
    }
    async fn exercise_aim_gate(cancel: bool) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gateway = Gateway::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "secret",
        )
        .unwrap();
        let held = gateway.voice_resources.lock().await;
        assert!(
            gateway
                .aim_light(json!({"operation":"aim_light","category":"book"}), "a")
                .await
                .unwrap_err()
                .contains("before speech")
        );
        drop(held);
        let copy = gateway.clone();
        let lookup = tokio::spawn(async move {
            copy.aim_light(json!({"operation":"aim_light","category":"book"}), "a")
                .await
        });
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(stream.read_u8().await.unwrap());
        }
        let header = String::from_utf8(header).unwrap();
        let size: usize = header
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_owned)
            })
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0; size];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["session_id"],
            "a"
        );
        assert!(gateway.voice_resources.try_lock().is_err());
        if cancel {
            lookup.abort();
            tokio::task::yield_now().await;
            assert!(gateway.voice_resources.try_lock().is_err());
        }
        let response = r#"{"accepted":true,"result":{"ok":true,"status":"not_found"}}"#;
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
        if cancel {
            assert!(lookup.await.unwrap_err().is_cancelled());
            tokio::time::timeout(Duration::from_secs(2), async {
                while gateway.voice_resources.try_lock().is_err() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        } else {
            assert_eq!(lookup.await.unwrap().unwrap()["status"], "not_found");
        }
        assert!(gateway.voice_resources.try_lock().is_ok());
    }
    #[tokio::test]
    async fn aiming_excludes_asr_and_speech_until_the_server_finishes() {
        exercise_aim_gate(false).await;
    }
    #[tokio::test]
    async fn cancelled_caller_cannot_release_the_inflight_lookup_gate() {
        exercise_aim_gate(true).await;
    }
    async fn peer(status: &str, body: &str) -> (Gateway, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gateway = Gateway::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "private-token",
        )
        .unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(stream.read_u8().await.unwrap());
            }
            let header = String::from_utf8(header).unwrap();
            let size = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "0".into())
                .parse()
                .unwrap();
            stream.read_exact(&mut vec![0; size]).await.unwrap();
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        (gateway, task)
    }
    #[tokio::test]
    async fn home_operations_preserve_gateway_success_and_allowlist_refusal() {
        let (gateway, task) = peer("200 OK", r#"{"accepted":true,"result":{"ok":true,"devices":[{"entity_id":"light.desk"}]}}"#).await;
        let result = gateway.robot_operation(json!({"operation":"home","request":{"action":"list"}}), "home-test").await.unwrap();
        assert_eq!(result["devices"][0]["entity_id"], "light.desk");
        task.await.unwrap();
        let (gateway, task) = peer("403 Forbidden", r#"{"error":{"message":"That device is not exposed to Orion"}}"#).await;
        let error = gateway.robot_operation(json!({"operation":"home","request":{"action":"control","entity":"switch.private","command":"on"}}), "home-test").await.unwrap_err();
        assert!(error.contains("not exposed"));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn http_rejections_preserve_reasons_with_a_status_fallback() {
        for (body, expected) in [
            (
                r#"{"error":{"message":"Alarm must be in the next 366 days"}}"#,
                "Alarm must be in the next 366 days",
            ),
            ("not json", "HTTP 409 Conflict"),
            (r#"{"error":{}}"#, "HTTP 409 Conflict"),
        ] {
            let (gateway, task) = peer("409 Conflict", body).await;
            assert_eq!(
                gateway
                    .request("/api/v2/operations", None)
                    .await
                    .unwrap_err(),
                expected
            );
            task.await.unwrap();
        }
    }
    #[tokio::test]
    async fn runtime_result_errors_are_preserved_for_lighting_and_robot_operations() {
        for lighting in [true, false] {
            let (gateway, task) = peer("200 OK", r#"{"accepted":false,"result":{"ok":false,"error":"Wait for the current scene or speech to finish."}}"#).await;
            let result = if lighting {
                gateway.set_lighting(json!({"brightness":0.35})).await
            } else {
                gateway
                    .robot_operation(json!({"operation":"routines"}), "test")
                    .await
            };
            assert_eq!(
                result.unwrap_err(),
                "Wait for the current scene or speech to finish."
            );
            task.await.unwrap();
        }
    }
    #[tokio::test]
    async fn model_errors_never_expose_tokens_or_urls() {
        let gateway = Gateway::new("http://127.0.0.1:1/private", "private-token").unwrap();
        let error = gateway
            .request("/api/v2/operations", None)
            .await
            .unwrap_err();
        assert!(!error.contains("http://") && !error.contains("private-token"));
        let error = gateway
            .model_error("Bearer private-token at http://other.invalid/private?q=private-token");
        assert!(!error.contains("http://") && !error.contains("private-token"));
    }
    #[test]
    fn wav_is_mono_24khz_pcm16() {
        let bytes = wav(&[1, 2, 3, 4]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 48);
        assert_eq!(&bytes[24..28], &24000u32.to_le_bytes());
        assert_eq!(&bytes[44..], &[1, 2, 3, 4]);
    }
}
