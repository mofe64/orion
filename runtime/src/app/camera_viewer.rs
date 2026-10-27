/// A bounded, renewable lease. Socket work stays in UnixCommandServer; this
/// state only parses a fixed command and compares the daemon's monotonic time.
#[derive(Default)]
pub(super) struct CameraViewer {
    expires_at: f64,
    /// The light the owner chose; a lease without one keeps the last choice.
    pixel: Option<usize>,
}
/// The largest light ring Orion drives.
const MAX_PIXEL: usize = 63;

impl CameraViewer {
    pub(super) fn command(&mut self, command: &str, now: f64, supported: bool) -> Option<String> {
        let value = command.strip_prefix("camera-viewer ")?;
        if !supported {
            return Some(
                serde_json::json!({"ok":false,"error":"Camera viewer marker requires V2."})
                    .to_string(),
            );
        }
        let mut parts = value.split(' ');
        let seconds = match parts.next() {
            Some("0") => 0.0,
            Some("3") => 3.0,
            _ => return Some(serde_json::json!({"ok":false,"error":"Expected camera-viewer 0 or camera-viewer 3 [LIGHT]."}).to_string()),
        };
        let pixel = match (parts.next(), parts.next()) {
            (None, None) => self.pixel,
            (Some(light), None) => match light.parse::<usize>() {
                Ok(light) if light <= MAX_PIXEL => Some(light),
                _ => return Some(serde_json::json!({"ok":false,"error":"Expected a light number from 0 to 63."}).to_string()),
            },
            _ => return Some(serde_json::json!({"ok":false,"error":"Expected camera-viewer 0 or camera-viewer 3 [LIGHT]."}).to_string()),
        };
        self.pixel = pixel;
        self.expires_at = now + seconds;
        Some(
            serde_json::json!({"ok":true,"command":"camera_viewer","expires_in_seconds":seconds})
                .to_string(),
        )
    }

    pub(super) fn active(&self, now: f64) -> bool {
        now < self.expires_at
    }

    /// The light to show while the lease runs.
    pub(super) fn pixel(&self, now: f64) -> Option<usize> {
        self.active(now).then(|| {
            self.pixel
                .unwrap_or(crate::expression::rest::V2_CAMERA_VIEWER_PIXEL)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LightingDevice, Result, Rgbw8, expression::rest::RestLighting};
    use std::{cell::RefCell, rc::Rc};

    struct ObservedRing(Rc<RefCell<Vec<Rgbw8>>>);
    impl LightingDevice for ObservedRing {
        fn pixel_count(&self) -> usize {
            24
        }
        fn render(&mut self, pixels: &[Rgbw8]) -> Result<()> {
            *self.0.borrow_mut() = pixels.to_vec();
            Ok(())
        }
    }

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
    fn the_chosen_light_stays_until_another_is_chosen() {
        let mut marker = CameraViewer::default();
        marker.command("camera-viewer 3", 0.0, true).unwrap();
        assert_eq!(marker.pixel(1.0), Some(16));
        marker.command("camera-viewer 3 5", 1.0, true).unwrap();
        assert_eq!(marker.pixel(2.0), Some(5));
        marker.command("camera-viewer 3", 2.0, true).unwrap();
        assert_eq!(marker.pixel(3.0), Some(5));
        assert_eq!(marker.pixel(9.0), None);
    }

    #[test]
    fn invalid_or_unsupported_commands_do_not_set_a_lease() {
        let mut marker = CameraViewer::default();
        assert!(marker.command("lamp 0 0 0 0", 0.0, true).is_none());
        for (command, supported) in [
            ("camera-viewer 3", false),
            ("camera-viewer 100", true),
            ("camera-viewer NaN", true),
            ("camera-viewer 3 64", true),
            ("camera-viewer 3 -1", true),
            ("camera-viewer 3 4 5", true),
        ] {
            let response: serde_json::Value =
                serde_json::from_str(&marker.command(command, 0.0, supported).unwrap()).unwrap();
            assert_eq!(response["ok"], false);
            assert!(!marker.active(0.0));
        }
    }

    #[test]
    fn expired_lease_restores_the_latest_underlying_frame() {
        let observed = Rc::new(RefCell::new(Vec::new()));
        let mut light = RestLighting::new(Box::new(ObservedRing(observed.clone())));
        let mut viewer = CameraViewer::default();
        viewer.command("camera-viewer 3", 10.0, true).unwrap();
        light.set_camera_viewer(viewer.pixel(10.0));
        light.update(false, 10.0).unwrap();
        light.render_uniform(Rgbw8::OFF).unwrap();
        let latest = (0..24)
            .map(|index| Rgbw8::new(index, 2, 3, 4))
            .collect::<Vec<_>>();
        light.render(&latest).unwrap();
        assert_ne!(*observed.borrow(), latest);
        light.set_camera_viewer(viewer.pixel(13.0));
        light.update(false, 13.0).unwrap();
        assert_eq!(*observed.borrow(), latest);
    }
}
