use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::prelude::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::{OrionRuntimeError as Error, Result};

const UNIX_PATH_CAPACITY: usize = 108;
const COMMAND_CAPACITY: usize = 4_096;
const CLIENT_CAPACITY: usize = 32;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(1);

pub struct UnixCommandServer {
    listener: UnixListener,
    path: PathBuf,
    owns_path: bool,
    pending: Vec<PendingClient>,
}

struct PendingClient {
    stream: UnixStream,
    input: Vec<u8>,
    output: Option<Vec<u8>>,
    written: usize,
    connected_at: Instant,
}

impl PendingClient {
    /// Return true while more socket I/O is needed. No peer can block a motor tick.
    fn poll<F: FnMut(&str) -> String>(&mut self, handler: &mut F) -> bool {
        if self.connected_at.elapsed() >= CLIENT_TIMEOUT {
            return false;
        }
        if self.output.is_none() {
            let mut buffer = [0_u8; COMMAND_CAPACITY];
            let eof = match self
                .stream
                .read(&mut buffer[..COMMAND_CAPACITY - self.input.len()])
            {
                Ok(0) => true,
                Ok(received) => {
                    self.input.extend_from_slice(&buffer[..received]);
                    false
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return true,
                Err(_) => return false,
            };
            if let Some(end) = self.input.iter().position(|byte| *byte == b'\n') {
                self.input.truncate(end);
            } else if self.input.len() == COMMAND_CAPACITY {
                self.output = Some(
                    b"{\"ok\":false,\"error\":\"Command exceeds socket capacity\"}\n".to_vec(),
                );
            } else if !eof {
                return true;
            }
            if self.output.is_none() {
                if self.input.is_empty() {
                    return false;
                }
                self.output = Some(match std::str::from_utf8(&self.input) {
                    Ok(command) => (handler(command.trim()) + "\n").into_bytes(),
                    Err(_) => b"{\"ok\":false,\"error\":\"Command must be UTF-8\"}\n".to_vec(),
                });
            }
        }
        let output = self
            .output
            .as_ref()
            .expect("complete command has a response");
        match self.stream.write(&output[self.written..]) {
            Ok(0) => false,
            Ok(written) => {
                self.written += written;
                self.written < output.len()
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => true,
            Err(_) => false,
        }
    }
}

impl UnixCommandServer {
    pub fn bind(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        validate_socket_path(path)?;
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.file_type().is_socket() => {
                return Err(Error::Runtime(format!(
                    "Refusing to replace non-socket path: {}",
                    path.display()
                )));
            }
            Ok(_) => fs::remove_file(path).map_err(|error| {
                Error::Runtime(format!(
                    "Could not remove stale Orion socket '{}': {error}",
                    path.display()
                ))
            })?,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(Error::Runtime(format!(
                    "Could not inspect Orion socket path '{}': {error}",
                    path.display()
                )));
            }
        }

        let listener = UnixListener::bind(path).map_err(|error| {
            Error::Runtime(format!(
                "Could not bind Orion Unix socket '{}': {error}",
                path.display()
            ))
        })?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o660)).map_err(|error| {
            Error::Runtime(format!(
                "Could not set Orion socket permissions '{}': {error}",
                path.display()
            ))
        })?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            path: path.to_owned(),
            owns_path: true,
            pending: Vec::new(),
        })
    }

    pub fn serve_pending<F>(&mut self, mut handler: F) -> Result<()>
    where
        F: FnMut(&str) -> String,
    {
        for _ in 0..CLIENT_CAPACITY {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    // Match C++ accept4(SOCK_NONBLOCK): a client that connects
                    // without sending a command must not stall the 50 Hz loop.
                    stream.set_nonblocking(true)?;
                    // Accept may precede the first write. Keep incomplete lines
                    // across ticks instead of interpreting WouldBlock as empty input.
                    if self.pending.len() < CLIENT_CAPACITY {
                        self.pending.push(PendingClient {
                            stream,
                            input: Vec::new(),
                            output: None,
                            written: 0,
                            connected_at: Instant::now(),
                        });
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => {
                    return Err(Error::Runtime(format!(
                        "Could not accept Orion status client: {error}"
                    )));
                }
            }
        }
        self.pending.retain_mut(|client| client.poll(&mut handler));
        Ok(())
    }
}

impl Drop for UnixCommandServer {
    fn drop(&mut self) {
        if self.owns_path {
            let _ = fs::remove_file(&self.path);
            self.owns_path = false;
        }
    }
}

pub fn request_daemon(path: impl AsRef<Path>, command: &str) -> Result<String> {
    let path = path.as_ref();
    validate_socket_path(path)?;
    let mut stream = UnixStream::connect(path)
        .map_err(|error| Error::Runtime(format!("Could not connect to Orion daemon: {error}")))?;
    stream
        .write_all(format!("{command}\n").as_bytes())
        .map_err(|error| Error::Runtime(format!("Could not request Orion status: {error}")))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| Error::Runtime(format!("Could not read Orion status: {error}")))?;
    Ok(response)
}

fn validate_socket_path(path: &Path) -> Result<()> {
    let length = path.as_os_str().as_bytes().len();
    if length == 0 || length >= UNIX_PATH_CAPACITY {
        return Err(Error::InvalidArgument(
            "Unix socket path is empty or too long.".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn delayed_and_split_commands_execute_once_without_blocking_ticks() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("oriond.sock");
        let mut server = UnixCommandServer::bind(&path).unwrap();
        let mut client = UnixStream::connect(&path).unwrap();
        server
            .serve_pending(|_| panic!("connection without input is not a command"))
            .unwrap();
        client.write_all(b"character sta").unwrap();
        server
            .serve_pending(|_| panic!("partial line is not a command"))
            .unwrap();
        client.write_all(b"tus\n").unwrap();
        let mut calls = 0;
        server
            .serve_pending(|command| {
                calls += 1;
                assert_eq!(command, "character status");
                "done".into()
            })
            .unwrap();
        server
            .serve_pending(|_| panic!("command must not execute twice"))
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert_eq!(calls, 1);
        assert_eq!(response, "done\n");
    }

    #[test]
    fn incomplete_clients_expire_and_oversized_commands_never_execute() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("oriond.sock");
        let mut server = UnixCommandServer::bind(&path).unwrap();
        let _idle = UnixStream::connect(&path).unwrap();
        server.serve_pending(|_| panic!("idle client")).unwrap();
        assert_eq!(server.pending.len(), 1);
        server.pending[0].connected_at = Instant::now() - CLIENT_TIMEOUT;
        server.serve_pending(|_| panic!("expired client")).unwrap();
        assert!(server.pending.is_empty());
        let mut large = UnixStream::connect(&path).unwrap();
        large.write_all(&vec![b'x'; COMMAND_CAPACITY]).unwrap();
        server
            .serve_pending(|_| panic!("oversized command"))
            .unwrap();
        let mut response = String::new();
        large.read_to_string(&mut response).unwrap();
        assert!(response.contains("exceeds socket capacity"));
    }

    #[test]
    fn refuses_to_replace_a_regular_file() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("oriond.sock");
        fs::write(&path, "keep me").unwrap();
        let error = UnixCommandServer::bind(&path).err().unwrap().to_string();
        assert!(error.contains("Refusing to replace non-socket path"));
        assert_eq!(fs::read_to_string(path).unwrap(), "keep me");
    }

    #[test]
    fn serves_a_command_and_removes_socket_on_drop() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("oriond.sock");
        let mut server = UnixCommandServer::bind(&path).unwrap();
        let client_path = path.clone();
        let client = thread::spawn(move || request_daemon(client_path, "status").unwrap());
        for _ in 0..100 {
            server
                .serve_pending(|command| format!("handled:{command}"))
                .unwrap();
            if client.is_finished() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(client.join().unwrap(), "handled:status\n");
        drop(server);
        assert!(!path.exists());
    }
}
