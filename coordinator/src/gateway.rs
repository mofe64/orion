use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Clone)]
pub(crate) struct Gateway {
    client: reqwest::Client,
    url: String,
    token: String,
}
pub(crate) type ActiveRun = Arc<Mutex<Option<u64>>>;
impl Gateway {
    pub fn new(url: &str, token: &str) -> Result<Self, String> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
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
