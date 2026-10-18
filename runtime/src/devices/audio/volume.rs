use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
pub struct VolumeStatus {
    pub volume: u8,
    pub muted: bool,
    pub epoch: String,
    pub revision: u64,
    pub save_error: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct SavedVolume {
    volume: u8,
}

#[derive(Debug)]
pub struct VolumeControl {
    pub(super) level: Arc<AtomicU8>,
    epoch: String,
    revision: u64,
    sender: Option<mpsc::SyncSender<()>>,
    writer: Option<JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
}

impl Default for VolumeControl {
    fn default() -> Self {
        Self {
            level: Arc::new(AtomicU8::new(100)),
            epoch: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                .to_string(),
            revision: 0,
            sender: None,
            writer: None,
            error: Arc::new(Mutex::new(None)),
        }
    }
}

impl VolumeControl {
    /// Load once at startup. Subsequent disk writes never run on the motor loop.
    pub fn persistent(path: PathBuf) -> crate::Result<Self> {
        let mut control = Self::default();
        match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<SavedVolume>(&bytes) {
                Ok(saved) if saved.volume <= 100 => {
                    control.level.store(saved.volume, Ordering::Relaxed)
                }
                _ => eprintln!("oriond: saved volume is invalid; using 100"),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => eprintln!("oriond: could not read saved volume: {error}"),
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let level = Arc::clone(&control.level);
        let error = Arc::clone(&control.error);
        control.writer = Some(
            thread::Builder::new()
                .name("orion-volume-store".into())
                .spawn(move || {
                    // Wakeups coalesce. Read the latest value after each wakeup so a
                    // full mailbox cannot discard the final position of a dragged dial.
                    while receiver.recv().is_ok() {
                        let result = save(&path, level.load(Ordering::Relaxed));
                        if let Err(reason) = &result {
                            eprintln!("oriond: could not save volume: {reason}");
                        }
                        *error.lock().unwrap() = result
                            .err()
                            .map(|_| "Orion couldn't save the volume. Please try again.".into());
                    }
                })?,
        );
        control.sender = Some(sender);
        Ok(control)
    }

    pub fn set(&mut self, level: u8) -> crate::Result<()> {
        if level > 100 {
            return Err(crate::Error::InvalidArgument(
                "Volume must be 0–100.".into(),
            ));
        }
        if self.level.swap(level, Ordering::Relaxed) != level {
            self.revision += 1;
        }
        if let Some(sender) = &self.sender {
            match sender.try_send(()) {
                Ok(()) | Err(mpsc::TrySendError::Full(())) => {}
                Err(mpsc::TrySendError::Disconnected(())) => {
                    return Err(crate::Error::Runtime("Volume storage stopped.".into()));
                }
            }
        }
        Ok(())
    }

    pub fn status(&self) -> VolumeStatus {
        let volume = self.level.load(Ordering::Relaxed);
        VolumeStatus {
            volume,
            muted: volume == 0,
            epoch: self.epoch.clone(),
            revision: self.revision,
            save_error: self.error.try_lock().ok().and_then(|value| value.clone()),
        }
    }
}

impl Drop for VolumeControl {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

fn save(path: &Path, volume: u8) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".volume-{}-{}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        serde_json::to_writer(&mut file, &SavedVolume { volume })?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapid_changes_persist_the_last_level_including_mute() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audio.json");
        for final_level in [40, 0] {
            let mut volume = VolumeControl::persistent(path.clone()).unwrap();
            for level in (0..=100).rev() {
                volume.set(level).unwrap();
            }
            volume.set(final_level).unwrap();
            drop(volume); // Drains the final coalesced write.
            let restored = VolumeControl::persistent(path.clone()).unwrap();
            assert_eq!(restored.status().volume, final_level);
            assert_eq!(restored.status().muted, final_level == 0);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn status_revisions_track_changes_and_restarts_have_a_new_epoch() {
        let mut volume = VolumeControl::default();
        let first = volume.status();
        volume.set(40).unwrap();
        assert_eq!(volume.status().revision, first.revision + 1);
        volume.set(40).unwrap();
        assert_eq!(volume.status().revision, first.revision + 1);
        assert!(volume.set(101).is_err());
        assert_eq!(volume.status().volume, 40);
        assert_ne!(VolumeControl::default().status().epoch, first.epoch);
    }
}
