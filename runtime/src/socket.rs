use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::prelude::OsStrExt;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

const UNIX_PATH_CAPACITY: usize = 108;
const COMMAND_CAPACITY: usize = 4_096;

pub struct UnixCommandServer {
    listener: UnixListener,
    path: PathBuf,
    owns_path: bool,
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
        })
    }

    pub fn serve_pending<F>(&self, mut handler: F) -> Result<()>
    where
        F: FnMut(&str) -> String,
    {
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    // Match C++ accept4(SOCK_NONBLOCK): a client that connects
                    // without sending a command must not stall the 50 Hz loop.
                    stream.set_nonblocking(true)?;
                    let mut buffer = [0_u8; COMMAND_CAPACITY];
                    let received = stream.read(&mut buffer).unwrap_or(0);
                    let command = String::from_utf8_lossy(&buffer[..received]);
                    let response = handler(command.trim()) + "\n";
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(error) => {
                    return Err(Error::Runtime(format!(
                        "Could not accept Orion status client: {error}"
                    )));
                }
            }
        }
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
        let server = UnixCommandServer::bind(&path).unwrap();
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
