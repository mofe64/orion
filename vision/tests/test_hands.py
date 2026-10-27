import tempfile
import unittest
from pathlib import Path
from threading import Event, Thread
from unittest.mock import Mock
import cv2
import numpy as np
from orion_vision.detect_hands import Palm, PalmDetector, anchors, preprocess, decode
from orion_vision.pipeline import FacePipeline
from orion_vision.gaze_client import GazeClient, RuntimeRejected
from test_objects import intrinsics
from test_tracking import eventually, face


class PalmTests(unittest.TestCase):
    def pipeline(self, mode='hands'):
        detector, client = Mock(), Mock()
        detector.detect.return_value = [Palm([320,180,64,36],.9)]
        client.bearings.return_value = [{'azimuth': .2, 'elevation': .6}]
        client.voice_active.return_value = False
        now = [10.]
        pipeline = FacePipeline(detector, intrinsics(), client, clock=lambda: now[0],
            boot_clock=lambda: int(now[0] * 1e9), palms=detector)
        pipeline.reset(True, mode=mode)
        return pipeline, detector, client, now

    def test_first_raised_palm_sends_a_bearing_without_desk_projection_or_delay(self):
        pipeline, detector, client, now = self.pipeline()
        frame = np.zeros((360,640,3),np.uint8)
        stamp = int(now[0] * 1e9)
        pipeline._tracking_frame(stamp, frame, pipeline.generation, frame, pipeline.preview_generation)
        track = client.target.call_args.args[0]
        np.testing.assert_allclose(track.state[:2], [.2, .6])
        self.assertEqual(track.capture_ns, stamp)
        client.camera_pose.assert_not_called()
        client.bearings.assert_called_once()
        transport = GazeClient()
        transport.command = Mock(return_value={'accepted': True})
        transport.target(track)
        import json
        payload = json.loads(transport.command.call_args.args[0].removeprefix('gaze target '))
        self.assertEqual(payload['class'], 'hand')
        self.assertEqual(payload['elevation'], .6)
        self.assertNotIn('point', payload)
        self.assertTrue(pipeline.preview()[2][0]['chosen_target'])

    def test_hand_and_face_frames_share_filtering_and_loss(self):
        states = []
        for mode in ['faces', 'hands']:
            pipeline, detector, client, now = self.pipeline(mode)
            frame = np.zeros((360,640,3),np.uint8)
            values = []
            for index, (azimuth, elevation) in enumerate([(.2,.5),(.22,.52),(.21,.48)]):
                detection = face((352,198),64) if mode == 'faces' else Palm([320,166,64,64],.95)
                detector.detect.return_value = [detection]
                client.bearings.return_value = [{'azimuth':azimuth,'elevation':elevation}]
                pipeline._tracking_frame(int(now[0]*1e9),frame,pipeline.generation)
                values.append(client.target.call_args.args[0].state.copy())
                now[0] += .1
            detector.detect.return_value = []
            pipeline._tracking_frame(int(now[0]*1e9),frame,pipeline.generation)
            self.assertFalse(client.clear.called)
            now[0] += 1.1
            pipeline._tracking_frame(int(now[0]*1e9),frame,pipeline.generation)
            client.clear.assert_called_once()
            states.append(values)
        np.testing.assert_array_equal(states[0], states[1])

    def test_demo_mode_counts_raised_palms_while_idle_at_a_low_rate(self):
        faces, palms, client = Mock(), Mock(), Mock()
        faces.detect.return_value = []
        palms.detect.return_value = [Palm([320,180,64,36],.9)]
        client.voice_active.return_value = False
        now = [10.]
        pipeline = FacePipeline(faces, intrinsics(), client, clock=lambda: now[0],
            boot_clock=lambda: int(now[0] * 1e9), palms=palms)
        pipeline.reset(True, mode='idle')
        frame = np.zeros((360,640,3),np.uint8)
        pipeline._tracking_frame(int(now[0]*1e9), frame, pipeline.generation)
        palms.detect.assert_not_called()
        self.assertEqual(pipeline.hand_count, 0)
        pipeline.configure_behaviours({**pipeline.behaviours, 'demo_mode': True})
        now[0] += .1
        pipeline._tracking_frame(int(now[0]*1e9), frame, pipeline.generation)
        self.assertEqual(pipeline.hand_count, 1)
        now[0] += .1
        pipeline._tracking_frame(int(now[0]*1e9), frame, pipeline.generation)
        self.assertEqual(palms.detect.call_count, 1)  # CPU palms run at 2 Hz
        client.target.assert_not_called()  # Idle never follows the palm itself
        pipeline.configure_behaviours({**pipeline.behaviours, 'demo_mode': False})
        now[0] += 1
        pipeline._tracking_frame(int(now[0]*1e9), frame, pipeline.generation)
        self.assertEqual(pipeline.hand_count, 0)

    def test_lively_counts_only_palms_held_up_near_a_face(self):
        from orion_vision.detect_faces import Face
        faces, palms, client = Mock(), Mock(), Mock()
        face = Face([300,100,60,60],[[310,120],[340,120],[325,135],[312,150],[338,150]],.9)
        faces.detect.return_value = [face]
        desk, wave = Palm([320,300,60,40],.9), Palm([400,90,50,50],.9)
        palms.detect.return_value = [desk]
        client.voice_active.return_value = False
        client.bearings.return_value = [{"azimuth": 0., "elevation": 0.}]
        now = [10.]
        pipeline = FacePipeline(faces, intrinsics(), client, clock=lambda: now[0],
            boot_clock=lambda: int(now[0] * 1e9), palms=palms)
        pipeline.reset(True, mode='idle')
        pipeline.configure_behaviours({**pipeline.behaviours, 'proactive_speech': True})
        frame = np.zeros((360,640,3),np.uint8)
        pipeline._tracking_frame(int(now[0]*1e9), frame, pipeline.generation)
        self.assertEqual((pipeline.hand_count, pipeline.raised_hands), (1, 0))
        palms.detect.return_value = [desk, wave]
        now[0] += .5
        pipeline._tracking_frame(int(now[0]*1e9), frame, pipeline.generation)
        self.assertEqual(palms.detect.call_count, 1)  # Lively CPU palms run at 1 Hz
        now[0] += .6
        pipeline._tracking_frame(int(now[0]*1e9), frame, pipeline.generation)
        self.assertEqual((pipeline.hand_count, pipeline.raised_hands), (2, 1))
        # A hand at the chin or chest, beside the face, is not a wave.
        palms.detect.return_value = [Palm([360,160,40,40],.9), Palm([300,190,60,40],.9)]
        now[0] += 1.1
        pipeline._tracking_frame(int(now[0]*1e9), frame, pipeline.generation)
        self.assertEqual((pipeline.hand_count, pipeline.raised_hands), (2, 0))

    def test_motion_rejection_preserves_paired_preview(self):
        pipeline, detector, client, now = self.pipeline()
        client.target.side_effect = RuntimeRejected('Servo bus unavailable')
        frame = np.zeros((360,640,3),np.uint8)
        stamp = int(now[0]*1e9)
        pipeline._tracking_frame(stamp,frame,pipeline.generation,frame,pipeline.preview_generation)
        self.assertEqual(pipeline.preview()[0],stamp)
        self.assertEqual(pipeline.status()['gaze_error'],'Servo bus unavailable')
        self.assertIsNone(pipeline.status()['error'])

    def test_worker_uses_face_rate_and_discards_late_palms_after_mode_change(self):
        pipeline, detector, client, now = self.pipeline()
        pipeline.start(); self.addCleanup(pipeline.close)
        frame = np.zeros((360,640,3),np.uint8)
        pipeline.submit(int(now[0]*1e9),frame)
        eventually(lambda: client.target.called)
        self.assertEqual(pipeline.rate_hz,10)
        entered, release = Event(), Event(); self.addCleanup(release.set)
        def detect(_):
            entered.set(); release.wait(2)
            return [Palm([320,180,64,36],.9)]
        detector.detect.side_effect=detect
        now[0]+=.21
        pipeline.submit(int(now[0]*1e9),frame)
        self.assertTrue(entered.wait(1))
        pipeline.reset(True,mode='faces')
        client.target.reset_mock()
        release.set()
        eventually(lambda: pipeline.pending is None)
        self.assertEqual(pipeline.hand_count,0)
        self.assertFalse(client.target.called)

    def test_letterbox_preserves_rgb_and_model_decoder_uses_xywh_nms(self):
        blob, geometry = preprocess(np.full((360,640,3), [240,30,10], np.uint8))
        self.assertEqual(blob.shape,(1,192,192,3))
        self.assertEqual(blob[0,41,0].tolist(),[0.,0.,0.])
        np.testing.assert_allclose(blob[0,42,0],[240/255,30/255,10/255])
        self.assertEqual(blob[0,150,0].tolist(),[0.,0.,0.])
        boxes=np.zeros((1,2016,18),np.float32)
        scores=np.full((1,2016,1),-100.,np.float32)
        # Two overlapping model anchors encode the same 100x100-pixel palm.
        for i in (0,1):
            boxes[0,i,:4]=[96-anchors()[i,0]*192,96-anchors()[i,1]*192,30,30]
            scores[0,i,0]=10-i
        result=decode([boxes,scores],geometry)
        self.assertEqual(len(result),1)
        np.testing.assert_allclose(result[0].box,[270,130,100,100])
        boxes[0,0,0]=np.nan
        self.assertEqual(len(decode([boxes,scores],geometry)),1)
        with self.assertRaises(ValueError): decode([boxes[:,:1],scores],geometry)
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'model.onnx';path.write_bytes(b'corrupt')
            with self.assertRaisesRegex(ValueError,'SHA-256'): PalmDetector(path)
