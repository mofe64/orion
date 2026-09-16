use crate::{Host, Request};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

pub const PROTOCOL: u32 = 1;
const LIMIT: u64 = 1024 * 1024;

pub fn service_home() -> Result<PathBuf, String> {
    std::env::var_os("ORION_STUDIO_SERVICE_HOME")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| crate::settings::expand_path("~/.local/share/orion/studio-service"))
}
pub fn private_directory(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub struct Owner(File);
impl Owner {
    pub fn acquire(directory: &Path) -> Result<Self, String> {
        private_directory(directory)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("owner.lock"))
            .map_err(|e| e.to_string())?;
        file.try_lock()
            .map_err(|_| "Another Orion service process owns voice.")?;
        // The OS releases the lock on exit, including crashes. Never unlink the lock file.
        let _ = std::fs::remove_file(directory.join("connection.json"));
        Ok(Self(file))
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[derive(Serialize, Deserialize)]
pub struct Connection {
    pub protocol: u32,
    pub address: SocketAddr,
    pub token: String,
}
#[derive(Serialize, Deserialize)]
struct Envelope {
    protocol: u32,
    token: String,
    request: Request,
}

pub struct Publication(PathBuf);
impl Publication {
    pub fn new(directory: &Path, connection: &Connection) -> Result<Self, String> {
        let path = directory.join("connection.json");
        let temporary = directory.join(format!("connection-{}.tmp", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(connection).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        std::fs::rename(temporary, &path).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for Publication {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn call(directory: &Path, request: &Request) -> Result<Value, String> {
    let bytes = std::fs::read(directory.join("connection.json"))
        .map_err(|_| "Orion voice service is stopped. Start orion-voice-stack on the Pi.")?;
    let connection: Connection =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid Orion service connection")?;
    if connection.protocol != PROTOCOL || !connection.address.ip().is_loopback() {
        return Err("Incompatible Orion service connection".into());
    }
    let mut stream = TcpStream::connect_timeout(&connection.address, Duration::from_secs(2))
        .map_err(|_| "Orion voice service is unavailable. Check orion-voice-stack on the Pi.")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(135)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let mut bytes = serde_json::to_vec(
        &json!({"protocol": PROTOCOL, "token": connection.token, "request": request}),
    )
    .map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if bytes.len() as u64 > LIMIT {
        return Err("Service request is too large".into());
    }
    stream
        .write_all(&bytes)
        .map_err(|_| "Orion service disconnected")?;
    let mut result = String::new();
    std::io::BufReader::new(Read::take(stream, LIMIT))
        .read_line(&mut result)
        .map_err(|_| "Orion service response timed out")?;
    let value: Value =
        serde_json::from_str(&result).map_err(|_| "Invalid Orion service response")?;
    if value["ok"] != true {
        return Err(value["error"]
            .as_str()
            .unwrap_or("Background request failed")
            .into());
    }
    Ok(value["result"].clone())
}

pub async fn serve(stream: tokio::net::TcpStream, token: &str, host: Arc<Host>) {
    let mut input = tokio::io::BufReader::new(stream.take(LIMIT));
    let mut line = String::new();
    let request = tokio::time::timeout(Duration::from_secs(5), input.read_line(&mut line)).await;
    let response = if matches!(request, Ok(Ok(_))) && line.ends_with('\n') {
        match serde_json::from_str::<Envelope>(&line) {
            Ok(envelope) if envelope.protocol == PROTOCOL && envelope.token == token => {
                match tokio::time::timeout(
                    Duration::from_secs(130),
                    host.dispatch(envelope.request),
                )
                .await
                {
                    Ok(Ok(result)) => json!({"ok": true, "result": result}),
                    Ok(Err(error)) => json!({"ok": false, "error": error}),
                    Err(_) => json!({"ok": false, "error": "Service request timed out"}),
                }
            }
            _ => json!({"ok": false, "error": "Invalid service request or credentials"}),
        }
    } else {
        json!({"ok": false, "error": "Incomplete or oversized service request"})
    };
    let bytes = format!("{response}\n");
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        input.into_inner().into_inner().write_all(bytes.as_bytes()),
    )
    .await;
}
