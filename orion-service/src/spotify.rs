//! Desktop loopback callback. Spotify credentials are exchanged on the lamp.
use crate::{pairing, remote};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub const REDIRECT_URI: &str = "http://127.0.0.1:49687/callback";

pub struct SignIn {
    pub url: String,
    listener: TcpListener,
    pairing: pairing::Pairing,
    state: String,
}

async fn gateway(pairing: &pairing::Pairing, action: &str, body: Value) -> Result<Value, String> {
    let (client, _) = remote::gateway_client(&pairing.url, false).await?;
    let response = client
        .post(format!(
            "{}/api/v2/spotify/auth/{action}",
            pairing.url.trim_end_matches('/')
        ))
        .bearer_auth(&pairing.token)
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| "Orion did not answer. Check its connection and try again.")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("Update Orion to set up Spotify.".into());
    }
    let ok = response.status().is_success();
    let value = remote::json_response(response).await?;
    if !ok {
        return Err(value["error"]["message"]
            .as_str()
            .unwrap_or("Spotify sign-in could not finish. Please try again.")
            .into());
    }
    Ok(value)
}

pub async fn begin(client_id: String) -> Result<SignIn, String> {
    if client_id.len() != 32 || !client_id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Paste the 32-character Client ID from your Spotify app dashboard.".into());
    }
    let pairing = pairing::load_pairing()
        .await?
        .ok_or("Connect Orion before signing in with Spotify.")?;
    let listener = TcpListener::bind("127.0.0.1:49687").await
        .map_err(|_| "Studio couldn't open the Spotify sign-in page on this computer. Close any other sign-in window and try again.")?;
    let value = gateway(
        &pairing,
        "start",
        json!({"client_id":client_id,"redirect_uri":REDIRECT_URI}),
    )
    .await?;
    let url = value["url"]
        .as_str()
        .ok_or("Orion did not return a Spotify sign-in page.")?
        .to_owned();
    let parsed = url::Url::parse(&url).map_err(|_| "Invalid Spotify sign-in page")?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("accounts.spotify.com")
        || parsed.path() != "/authorize"
    {
        return Err("Orion did not return a Spotify sign-in page.".into());
    }
    let state = parsed
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .ok_or("Spotify sign-in is incomplete.")?;
    Ok(SignIn {
        url,
        listener,
        pairing,
        state,
    })
}

fn callback(request: &str, state: &str) -> Option<Value> {
    let mut parts = request.lines().next()?.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    if !target.starts_with("/callback?") {
        return None;
    }
    let url = url::Url::parse(&format!("http://127.0.0.1:49687{target}")).ok()?;
    let fields: std::collections::BTreeMap<_, _> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if fields.get("state").map(String::as_str) != Some(state) {
        return None;
    }
    Some(json!({"state":state,"code":fields.get("code"),"error":fields.get("error")}))
}

impl SignIn {
    pub async fn finish(self) -> Result<Value, String> {
        tokio::time::timeout(Duration::from_secs(180), async {
            loop {
                let (mut stream, _) = self.listener.accept().await.map_err(|_| "Spotify sign-in closed. Please try again.")?;
                let request = tokio::time::timeout(Duration::from_secs(5), async {
                    let mut bytes = vec![];
                    while bytes.len() < 16_384 && !bytes.ends_with(b"\r\n\r\n") { bytes.push(stream.read_u8().await?); }
                    Ok::<_,std::io::Error>(String::from_utf8_lossy(&bytes).into_owned())
                }).await;
                let value = request.ok().and_then(Result::ok).and_then(|request| callback(&request, &self.state));
                let (result, body) = if let Some(value) = value {
                    let result = gateway(&self.pairing, "finish", value).await;
                    let body = if result.is_ok() {"Spotify is connected. You can return to Orion Studio."} else {"Spotify sign-in could not finish. Return to Orion Studio for help."};
                    (Some(result),body)
                } else { (None,"This sign-in link is not active. Return to Orion Studio.") };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",body.len());
                let _ = stream.write_all(response.as_bytes()).await;
                if let Some(result) = result { return result; }
            }
        }).await.map_err(|_| "Spotify sign-in timed out. Check that the redirect URI matches Studio, then sign in again.".to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_requires_the_expected_path_and_state_and_decodes_the_code() {
        let result = callback(
            "GET /callback?state=expected&code=one%2Btwo HTTP/1.1\r\n\r\n",
            "expected",
        )
        .unwrap();
        assert_eq!(result["code"], "one+two");
        for request in [
            "GET /callback?state=other&code=stolen HTTP/1.1",
            "GET /else?state=expected&code=x HTTP/1.1",
            "POST /callback?state=expected&code=x HTTP/1.1",
        ] {
            assert!(callback(request, "expected").is_none());
        }
        assert_eq!(
            callback(
                "GET /callback?state=expected&error=access_denied HTTP/1.1",
                "expected"
            )
            .unwrap()["error"],
            "access_denied"
        );
    }
}
