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
        request
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())
    }
    pub async fn upload(&self, path: &str, pcm: &[u8], request_id: &str) -> Result<Value, String> {
        self.client
            .post(format!("{}{path}", self.url))
            .bearer_auth(&self.token)
            .header("Content-Type", "audio/wav")
            .header("X-Orion-Voice-Request-ID", request_id)
            .body(wav(pcm))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())
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
    #[test]
    fn wav_is_mono_24khz_pcm16() {
        let bytes = wav(&[1, 2, 3, 4]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 48);
        assert_eq!(&bytes[24..28], &24000u32.to_le_bytes());
        assert_eq!(&bytes[44..], &[1, 2, 3, 4]);
    }
}
