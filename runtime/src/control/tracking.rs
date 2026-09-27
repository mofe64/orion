//! Opt-in, lossy tracking capture. The controller never serializes or touches disk.
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::control::state::MovementPhase;
use crate::motion::pose::JointPositions;

const QUEUE_CAPACITY: usize = 512;
const FLUSH_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Serialize)]
pub struct TrackingJoint {
    pub commanded_position_rad: f64,
    pub measured_position_rad: f64,
    pub measured_velocity_rad_s: f64,
}

#[derive(Debug, Serialize)]
pub struct TrackingSample {
    pub sequence: u64,
    pub runtime_time_seconds: f64,
    pub elapsed_seconds: f64,
    pub trajectory_elapsed_seconds: f64,
    pub trajectory_revision: u64,
    pub run_id: u64,
    pub run_name: String,
    pub motion_name: String,
    pub phase: MovementPhase,
    pub command_written: bool,
    pub feedback_read_seconds: f64,
    pub control_work_seconds: f64,
    pub commanded_start: JointPositions,
    pub measured_start: JointPositions,
    pub joints: std::collections::BTreeMap<String, TrackingJoint>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrackingEvent {
    Sample(Box<TrackingSample>),
    End {
        run_id: u64,
        elapsed_seconds: f64,
        state: MovementPhase,
    },
}

/// Bounded try_send only. Full or failed queues lose records, never control ticks.
pub struct TrackingTelemetry {
    sender: Option<SyncSender<TrackingEvent>>,
    dropped: Arc<AtomicU64>,
    writer: Option<JoinHandle<()>>,
}

impl TrackingTelemetry {
    /// Refuse to overwrite an earlier experiment. Open before enabling hardware.
    pub fn create(path: impl AsRef<Path>, backend: &str) -> crate::Result<Self> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let header = serde_json::json!({
            "kind": "header", "schema_version": 1, "backend": backend,
            "build_revision": crate::BUILD_REVISION, "update_hz": 50,
            "started_at_unix_ns": SystemTime::now().duration_since(UNIX_EPOCH)
                .unwrap_or_default().as_nanos().to_string(),
            "feedback_before_write": true, "queue_capacity": QUEUE_CAPACITY,
        });
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let writer_dropped = Arc::clone(&dropped);
        let writer = thread::Builder::new()
            .name("orion-tracking".into())
            .spawn(move || {
                let result = (|| -> crate::Result<()> {
                    let mut output = BufWriter::with_capacity(256 * 1024, file);
                    serde_json::to_writer(&mut output, &header)?;
                    output.write_all(b"\n")?;
                    let mut last_flush = Instant::now();
                    let mut written = 0_u64;
                    loop {
                        match receiver
                            .recv_timeout(FLUSH_INTERVAL.saturating_sub(last_flush.elapsed()))
                        {
                            Ok(event) => {
                                serde_json::to_writer(&mut output, &event)?;
                                output.write_all(b"\n")?;
                                written += 1;
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                        if last_flush.elapsed() >= FLUSH_INTERVAL {
                            output.flush()?;
                            last_flush = Instant::now();
                        }
                    }
                    serde_json::to_writer(
                        &mut output,
                        &serde_json::json!({
                            "kind": "summary", "written_records": written,
                            "dropped_records": writer_dropped.load(Ordering::Relaxed),
                        }),
                    )?;
                    output.write_all(b"\n")?;
                    output.flush()?;
                    Ok(())
                })();
                if let Err(error) = result {
                    eprintln!(
                        "oriond: tracking writer failed; control continues without capture: {error}"
                    );
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            dropped,
            writer: Some(writer),
        })
    }

    pub fn record(&self, event: TrackingEvent) {
        if let Some(sender) = &self.sender {
            match sender.try_send(event) {
                Ok(()) => {}
                Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }
}

impl Drop for TrackingTelemetry {
    fn drop(&mut self) {
        // Joining happens only at daemon/core teardown, outside the control loop.
        self.sender.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> TrackingEvent {
        TrackingEvent::End {
            run_id: 1,
            elapsed_seconds: 2.0,
            state: MovementPhase::Completed,
        }
    }

    #[test]
    fn full_or_disconnected_queue_counts_loss_without_waiting() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let telemetry = TrackingTelemetry {
            sender: Some(sender),
            dropped: Arc::new(AtomicU64::new(0)),
            writer: None,
        };
        telemetry.record(event());
        telemetry.record(event());
        assert_eq!(telemetry.dropped.load(Ordering::Relaxed), 1);
        drop(receiver);
        telemetry.record(event());
        assert_eq!(telemetry.dropped.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn drains_on_shutdown_and_refuses_to_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tracking.jsonl");
        let telemetry = TrackingTelemetry::create(&path, "test").unwrap();
        telemetry.record(event());
        assert!(TrackingTelemetry::create(&path, "test").is_err());
        drop(telemetry);
        let rows: Vec<serde_json::Value> = std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows[0]["feedback_before_write"], true);
        assert_eq!(rows[1]["kind"], "end");
        assert_eq!(rows[2]["dropped_records"], 0);
        assert_eq!(rows[2]["written_records"], 1);
    }
}
