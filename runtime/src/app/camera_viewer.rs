/// A bounded, renewable lease. Socket work stays in UnixCommandServer; this
/// state only parses a fixed command and compares the daemon's monotonic time.
#[derive(Default)]
pub(super) struct CameraViewer {
    expires_at: f64,
}

impl CameraViewer {
    pub(super) fn command(&mut self, command: &str, now: f64, supported: bool) -> Option<String> {
        let value = command.strip_prefix("camera-viewer ")?;
        if !supported {
            return Some(
                serde_json::json!({"ok":false,"error":"Camera viewer marker requires V2."})
                    .to_string(),
            );
        }
        let seconds = match value {
            "0" => 0.0,
            "3" => 3.0,
            _ => return Some(serde_json::json!({"ok":false,"error":"Expected camera-viewer 0 or camera-viewer 3."}).to_string()),
        };
        self.expires_at = now + seconds;
        Some(
            serde_json::json!({"ok":true,"command":"camera_viewer","expires_in_seconds":seconds})
                .to_string(),
        )
    }

    pub(super) fn active(&self, now: f64) -> bool {
        now < self.expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_renews_expires_and_clears() {
        let mut marker = CameraViewer::default();
        assert!(!marker.active(0.0));
        marker.command("camera-viewer 3", 10.0, true).unwrap();
        assert!(marker.active(12.0));
        assert!(!marker.active(13.0));
        marker.command("camera-viewer 3", 12.0, true).unwrap();
        assert!(marker.active(14.0));
        marker.command("camera-viewer 0", 14.0, true).unwrap();
        assert!(!marker.active(14.0));
    }

    #[test]
    fn invalid_or_unsupported_commands_do_not_set_a_lease() {
        let mut marker = CameraViewer::default();
        assert!(marker.command("lamp 0 0 0 0", 0.0, true).is_none());
        for (command, supported) in [
            ("camera-viewer 3", false),
            ("camera-viewer 100", true),
            ("camera-viewer NaN", true),
        ] {
            let response: serde_json::Value =
                serde_json::from_str(&marker.command(command, 0.0, supported).unwrap()).unwrap();
            assert_eq!(response["ok"], false);
            assert!(!marker.active(0.0));
        }
    }
}
