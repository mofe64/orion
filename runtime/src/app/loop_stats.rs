//! How well the 50 Hz control loop keeps time, for Studio and the recorder.
use std::collections::VecDeque;

/// A tick that starts this much after its slot is late: half a period.
const LATE_MS: f64 = 10.0;
/// Recent readings cover this many seconds.
const RECENT_SECONDS: usize = 10;

#[derive(Clone, Copy, Debug, Default)]
struct Second {
    at: u64,
    worst_work_ms: f64,
    worst_delay_ms: f64,
    late: u32,
    ticks: u32,
}

#[derive(Debug, Default)]
pub(super) struct LoopStats {
    seconds: VecDeque<Second>,
    late_total: u64,
    ticks_total: u64,
    bus_reconnects: u64,
}

impl LoopStats {
    /// One tick: how long after its slot it started, and how long its work took.
    pub(super) fn record(&mut self, second: u64, delay_ms: f64, work_ms: f64) {
        if self.seconds.back().is_none_or(|last| last.at != second) {
            self.seconds.push_back(Second {
                at: second,
                ..Default::default()
            });
            while self
                .seconds
                .front()
                .is_some_and(|first| second - first.at >= RECENT_SECONDS as u64)
            {
                self.seconds.pop_front();
            }
        }
        let current = self.seconds.back_mut().unwrap();
        current.worst_work_ms = current.worst_work_ms.max(work_ms);
        current.worst_delay_ms = current.worst_delay_ms.max(delay_ms);
        current.ticks += 1;
        self.ticks_total += 1;
        if delay_ms > LATE_MS {
            current.late += 1;
            self.late_total += 1;
        }
    }
    pub(super) fn bus_reconnected(&mut self) {
        self.bus_reconnects += 1;
    }
    pub(super) fn status(&self) -> serde_json::Value {
        let recent = |value: fn(&Second) -> f64| self.seconds.iter().map(value).fold(0.0, f64::max);
        serde_json::json!({
            "period_ms": super::OBSERVE_PERIOD.as_secs_f64() * 1000.0,
            "recent_seconds": RECENT_SECONDS,
            "worst_work_ms": recent(|second| second.worst_work_ms),
            "worst_delay_ms": recent(|second| second.worst_delay_ms),
            "late_ticks": self.seconds.iter().map(|second| second.late).sum::<u32>(),
            "ticks": self.seconds.iter().map(|second| second.ticks).sum::<u32>(),
            "late_ticks_total": self.late_total,
            "ticks_total": self.ticks_total,
            "bus_reconnects": self.bus_reconnects,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_ticks_count_for_ten_seconds_and_in_the_total() {
        let mut stats = LoopStats::default();
        stats.record(0, 2.0, 4.0);
        stats.record(0, 14.0, 30.0);
        stats.record(5, 1.0, 3.0);
        let status = stats.status();
        assert_eq!(status["late_ticks"], 1);
        assert_eq!(status["worst_work_ms"], 30.0);
        assert_eq!(status["ticks"], 3);
        stats.record(12, 1.0, 3.0);
        stats.bus_reconnected();
        let status = stats.status();
        assert_eq!(status["late_ticks"], 0);
        assert_eq!(status["late_ticks_total"], 1);
        assert_eq!(status["worst_work_ms"], 3.0);
        assert_eq!(status["bus_reconnects"], 1);
    }
}
