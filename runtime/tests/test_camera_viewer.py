from daemon_support import DaemonTestCase, daemon, request


class CameraViewerTests(DaemonTestCase):
    def test_v2_marker_command_preserves_lamp_preferences_modes_and_motion(self):
        with daemon(following=True, hardware='v2') as path:
            self.ok(path, 'lamp 1 2 3 40')
            before = request(path, 'status')
            lamp = request(path, 'lamp-status')
            character = request(path, 'character status')
            self.ok(path, 'camera-viewer 3')
            self.assertFalse(request(path, 'camera-viewer 100')['ok'])
            self.assertEqual(request(path, 'lamp-status'), lamp)
            after = request(path, 'status')
            for field in ('mode', 'torque_enabled', 'motion'):
                self.assertEqual(after[field], before[field])
            self.assertEqual(request(path, 'character status')['routines']['mode'], character['routines']['mode'])
            self.ok(path, 'camera-viewer 0')

    def test_v1_rejects_marker(self):
        with daemon(following=True) as path:
            self.assertFalse(request(path, 'camera-viewer 3')['ok'])
