import tempfile
import unittest
from pathlib import Path
from threading import Event, Thread, Lock
from unittest.mock import Mock
import cv2
import numpy as np
from orion_vision.detect_hands import Palm, PalmDetector, anchors, preprocess, decode
from orion_vision.hands import HandWindow
from orion_vision.pipeline import FacePipeline
from test_objects import CAMERA, intrinsics
from test_tracking import eventually


class PalmTests(unittest.TestCase):
    def test_reset_waits_for_an_approved_hands_publication(self):
        # Pause exactly where reset acquires the command lock. It must not
        # change the mode underneath the worker already publishing metadata.
        attempting = Event()
        class ObservedLock:
            def __init__(self): self.lock = Lock()
            def __enter__(self):
                attempting.set()
                self.lock.acquire()
            def __exit__(self, *args): self.lock.release()
        client = Mock()
        pipeline = FacePipeline(Mock(), intrinsics(), client)
        pipeline.reset(True, mode='hands')
        pipeline.commands = ObservedLock()
        with pipeline.commands:
            attempting.clear()
            thread = Thread(target=lambda: pipeline.reset(False, mode='hands', preserve_hands=True))
            thread.start()
            self.assertTrue(attempting.wait(1))
            pipeline._surroundings(pipeline.boot_clock(), [])
            self.assertEqual(client.surroundings.call_args.args[0]['tracking_mode'], 'hands')
        thread.join(1)
        self.assertFalse(thread.is_alive())
        self.assertFalse(pipeline.enabled)

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

    def test_window_uses_each_capture_transform_and_requires_continuity(self):
        window=HandWindow(); lens=intrinsics(); target=np.array([0.,-.5,0.])
        for i in range(11):
            angle=i*.015
            rotation=np.array([[np.cos(angle),-np.sin(angle),0],[np.sin(angle),np.cos(angle),0],[0,0,1]]) @ np.array(CAMERA['rotation'])
            origin=np.array([i*.002,0.,.2])
            local=rotation.T @ (target-origin)
            pixel=cv2.fisheye.projectPoints(local.reshape(1,1,3),np.zeros(3),np.zeros(3),lens.matrix,lens.distortion)[0].reshape(2)
            point=window.observe([Palm([pixel[0]-10,pixel[1]-10,20,20],.9)],1+i*200_000_000,lens,
                                 dict(position=origin.tolist(),rotation=rotation.tolist()))
            if i<10:self.assertIsNone(point)
        np.testing.assert_allclose(point,target,atol=1e-8)
        self.assertIsNone(window.observe([],2_200_000_001,lens,CAMERA))
        self.assertEqual(len(window.samples),0)
        window.observe([Palm([300,30,30,30],.9)],3_000_000_001,lens,CAMERA)
        self.assertIsNone(window.observe([Palm([300,30,30,30],.9)],5_000_000_001,lens,CAMERA))
        self.assertEqual(len(window.samples),1)

    def test_hands_mode_publishes_paired_hand_boxes_and_counts_hands(self):
        faces,palms,client=Mock(),Mock(),Mock()
        palms.detect.return_value=[Palm([320,180,64,36],.9)]
        client.camera_pose.return_value=CAMERA
        client.voice_active.return_value=False
        now=[10.]
        pipeline=FacePipeline(faces,intrinsics(),client,clock=lambda:now[0],boot_clock=lambda:int(now[0]*1e9),palms=palms)
        pipeline.start();self.addCleanup(pipeline.close)
        pipeline.reset(True,mode='hands')
        frame,rgb=np.zeros((360,640,3),np.uint8),np.zeros((360,640,3),np.uint8)
        stamp=int(now[0]*1e9)
        pipeline.submit(stamp,frame,rgb=rgb)
        eventually(lambda:pipeline.preview() is not None)
        captured,pixels,detections=pipeline.preview()
        self.assertEqual(captured,stamp);self.assertIs(pixels,rgb)
        self.assertEqual(detections,[{"class":"hand","track_id":1,"score":.9,"chosen_target":False,
            "capture_boottime_ns":str(stamp),"box":[.5,.5,.1,.1]}])
        self.assertEqual(pipeline.status()['hand_count'],1)
        faces.detect.assert_not_called()

    def test_single_worker_switches_detectors_and_late_hand_work_cannot_survive_mode_change(self):
        faces,palms,objects,client=Mock(),Mock(),Mock(),Mock()
        faces.detect.return_value=[]
        palms.detect.return_value=[Palm([300,30,30,30],.9)]
        client.camera_pose.return_value=CAMERA
        client.voice_active.return_value=False
        now=[10.]
        pipeline=FacePipeline(faces,intrinsics(),client,clock=lambda:now[0],boot_clock=lambda:int(now[0]*1e9),objects=objects,palms=palms)
        pipeline.start();self.addCleanup(pipeline.close)
        frame=np.zeros((360,640,3),np.uint8)
        pipeline.reset(True)
        pipeline.submit(int(now[0]*1e9),frame)
        eventually(lambda:faces.detect.call_count==1)
        pipeline.reset(True,mode='hands')
        for _ in range(11):
            before=palms.detect.call_count;now[0]+=.21
            pipeline.submit(int(now[0]*1e9),frame)
            eventually(lambda:palms.detect.call_count>before)
        self.assertEqual(faces.detect.call_count,1)
        self.assertEqual(pipeline.rate_hz,5)
        objects.detect.assert_not_called()
        self.assertIsNotNone(pipeline.hand_point)
        self.assertEqual(pipeline.observations(),[])
        self.assertEqual(client.surroundings.call_args.args[0]['tracking_mode'],'hands')
        entered,release=Event(),Event();self.addCleanup(release.set)
        def detect(_): entered.set();release.wait(2);return [Palm([300,30,30,30],.9)]
        palms.detect.side_effect=detect;now[0]+=.21
        pipeline.submit(int(now[0]*1e9),frame);self.assertTrue(entered.wait(1))
        pipeline.reset(True,mode='faces');client.surroundings.reset_mock();release.set()
        now[0]+=.21;pipeline.submit(int(now[0]*1e9),frame)
        eventually(lambda:faces.detect.call_count==2)
        self.assertEqual(pipeline.hand_count,0)
        self.assertIsNone(pipeline.hand_point)
        self.assertTrue(all(call.args[0]['tracking_mode']=='faces' for call in client.surroundings.call_args_list))
