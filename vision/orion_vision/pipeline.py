"""One inference worker and one replaceable frame slot; no callback inference."""
from threading import Condition, Event, Lock, Thread
import time
import math
import cv2

from .gaze_client import boottime_ns
from .tracker import MAX_AGE_NS, Tracker, choose_track
from .find import FindError, FindJob, locate
from .behaviours import KEYS, ObjectMap, overlaps
from .novelty import NoveltyTrack
from .detector_policy import backend, rates

GAZE_RETRY_SECONDS = 3.
TARGET_AREA_RATIO = 1.5
TARGET_HOLD_NS = 1_000_000_000


class FacePipeline:
    def __init__(self, detector, intrinsics, client, clock=time.monotonic, boot_clock=boottime_ns,
                 objects=None, palms=None, accelerator=None):
        self.detector, self.intrinsics, self.client = detector, intrinsics, client
        self.clock, self.boot_clock = clock, boot_clock
        self.changed, self.stop = Condition(), Event()
        self.commands = Lock()
        self.pending = None
        self.latest_preview = None
        self.preview_generation = 0
        self.enabled = False
        self.generation = 0
        self.capture_generation = 0
        self.dropped = self.inferences = 0
        self.tracker = Tracker()
        self.error = None
        self.rate_hz = rates(detector).idle_hz
        self.accelerator = accelerator
        self.next_voice_poll = 0.
        self.thread = None
        self.gaze_available = True
        self.next_gaze_poll = 0.
        self.gaze_error = None
        self.target_selected = False
        self.last_gaze_capture = 0
        self.target_track_id = None
        self.speaker_track_id = None
        self.speaker_until = 0.
        self.challenger_track_id = None
        self.challenger_since_ns = None
        self.objects, self.palms = objects, palms
        self.mode = "off"
        self.hand_count = 0
        # Room light from the camera, in lux, when the camera reports it.
        self.room_lux = None
        self.camera_enabled = False
        self.find_job = None
        self.behaviours = dict.fromkeys(KEYS, False)
        self.object_map = ObjectMap()
        self.next_object_pass = 0.
        self.next_palm_pass = 0.
        self.next_surroundings = 0.
        self.object_baseline = False
        self.novelty = self.novelty_observation = None

    def start(self):
        self.thread = Thread(target=self._run, name="vision-inference", daemon=True)
        self.thread.start()

    def reset(self, enabled, camera_enabled=None, mode=None):
        # Match inference's commands -> changed lock order: a reset cannot
        # change the mode underneath an approved tracking publication.
        with self.commands:
            with self.changed:
                self.enabled = enabled
                self.mode = mode if mode is not None else ("faces" if enabled else "off")
                self.hand_count = 0
                self.camera_enabled = enabled if camera_enabled is None else camera_enabled
                self.generation += 1
                self.capture_generation += 1
                if self.find_job is not None:
                    self.find_job.error = FindError('camera_changed', 'Capture or tracking changed during lookup')
                    if not self.find_job.running:
                        self.find_job.done.set()
                self.pending = None
                self.latest_preview = None
                self.preview_generation += 1
                self.tracker.clear()
                self.object_map.clear()
                self.object_baseline = False
                self.novelty = self.novelty_observation = None
                self.target_track_id = self.challenger_track_id = self.challenger_since_ns = None
                self.speaker_track_id = None
                self.speaker_until = 0.
                self.changed.notify_all()
            try:
                self.client.clear_surroundings()
            except (OSError, ValueError, AttributeError):
                pass
            if not enabled or self.mode != "faces":
                self._clear_target()

    def configure_behaviours(self, value):
        with self.changed:
            if value == self.behaviours:
                return
            self.behaviours = value.copy()
            self.object_map.clear()
            self.object_baseline = False
            self.novelty = self.novelty_observation = None
            # Invalidate detector work in flight without resetting the cooldown.
            self.generation += 1
            self.pending = None
            self.next_surroundings = 0.
        if self.mode == 'hands':
            return  # Preference changes do not leave the explicit tracking mode.
        with self.commands:
            try:
                self.client.clear_surroundings()
            except (OSError, ValueError, AttributeError):
                pass

    def _background_objects(self, stamp, buffer, faces, generation):
        now = self.clock()
        with self.changed:
            wanted = self.enabled and self.mode in ("idle", "faces") and (self.behaviours['object_glances'] or self.behaviours['new_object_peek']) and self.objects is not None
        if not wanted or now < self.next_object_pass:
            return
        # This shares the inference worker with YuNet and explicit /find.
        # A refusal also backs off; switches cannot create inference bursts.
        self.next_object_pass = now + rates(self.objects).objects_seconds
        if not self.client.background_ready() or not self.client.find_allowed():
            return
        camera = self.client.camera_pose(stamp)
        try:
            objects = self.objects.detect(buffer)
        finally:
            # A failed accelerator can switch this detector to the CPU here.
            self.next_object_pass = now + rates(self.objects).objects_seconds
        candidates = locate([obj for obj in objects if obj.category != 'person'
            and not any(overlaps(obj.box, face.box) for face in faces)], self.intrinsics, camera, stamp)
        if not self.client.find_allowed() or not self.client.background_ready():
            return
        added = []
        with self.changed:
            if self.enabled and generation == self.generation and (self.behaviours['object_glances'] or self.behaviours['new_object_peek']):
                observed = self.object_map.update(candidates, stamp)
                had_baseline, self.object_baseline = self.object_baseline, True
                if had_baseline and self.behaviours['new_object_peek'] and self.novelty is None:
                    added = observed
        # Feature extraction must not hold the camera submission lock.
        for candidate in sorted(added, key=lambda entry: -entry['confidence']):
            if candidate['confidence'] < .6:
                continue
            try:
                track = NoveltyTrack(candidate, buffer, stamp)
            except (ValueError, cv2.error):
                continue
            with self.changed:
                if self.enabled and generation == self.generation:
                    self.novelty = track
            break

    def _advance_novelty(self, stamp, buffer, faces, generation):
        with self.changed:
            track = self.novelty
        if track is None:
            return
        try:
            if not self.client.find_allowed():
                raise ValueError('Voice has priority')
            obj = track.advance(buffer, stamp, faces)
            if obj is None:
                raise ValueError('Object persistence is unverified')
            candidate = locate([obj], self.intrinsics, self.client.camera_pose(stamp), stamp)[0]
            value = {key: candidate[key] for key in ('category', 'point', 'azimuth', 'elevation')}
            value.update(id=track.candidate['id'], interest_key=track.candidate['interest_key'],
                         capture_boottime_ns=stamp, since_boottime_ns=track.since)
        except (OSError, ValueError, cv2.error):
            with self.changed:
                if self.novelty is track:
                    self.novelty = self.novelty_observation = None
            return
        with self.changed:
            if self.enabled and generation == self.generation and self.novelty is track:
                self.novelty_observation = value

    def _idle_palms_wanted(self):
        return (self.mode == 'idle' and self.behaviours.get('demo_mode') is True
                and self.palms is not None)

    def _idle_palms(self, buffer, generation):
        """Demo mode looks for a raised hand while idle, at a low rate."""
        now = self.clock()
        with self.changed:
            wanted = self.enabled and self._idle_palms_wanted()
        if not wanted or now < self.next_palm_pass:
            return
        self.next_palm_pass = now + (.2 if backend(self.palms) == 'hailo' else .5)
        palms = self.palms.detect(buffer)
        with self.changed:
            if self.enabled and generation == self.generation and self._idle_palms_wanted():
                self.hand_count = len(palms)

    def _surroundings(self, stamp, tracks):
        if self.clock() < self.next_surroundings:
            return False
        self.next_surroundings = self.clock() + .5
        self.client.surroundings({'capture_boottime_ns': stamp,
            'settings': self.behaviours.copy(), 'camera_on': self.camera_enabled,
            'tracking_mode': self.mode if self.enabled else 'off',
            'hand_count': self.hand_count,
            'room_lux': self.room_lux,
            'objects': self.object_map.snapshot(self.boot_clock()),
            'novelty': self.novelty_observation,
            'faces': [{'track_id': track.id, 'azimuth': float(math.atan2(math.sin(track.state[0]), math.cos(track.state[0]))),
                       'elevation': float(track.state[1]), 'frontal': track.face.looking_at_orion}
                      for track in tracks if self.mode != "hands"],
            'looking_away': not any(track.face.looking_at_orion for track in tracks) if tracks and self.mode != 'hands' else None})
        return True

    def close(self):
        self.reset(False)
        self.stop.set()
        with self.changed:
            self.changed.notify_all()
        for detector in (self.objects, self.palms, self.detector, self.accelerator):
            if detector is not None and hasattr(detector, 'close'):
                detector.close()
        if self.thread:
            self.thread.join(timeout=2)

    def submit(self, stamp, buffer, metadata=None, geometry=None, rgb=None):
        lux = (metadata or {}).get('Lux')
        with self.changed:
            self.room_lux = float(lux) if isinstance(lux, (int, float)) and math.isfinite(lux) and 0 <= lux <= 200000 else None
            if self.camera_enabled and (self.enabled or self.find_job is not None) and buffer is not None:
                if self.intrinsics.geometry is not None:
                    try:
                        self.intrinsics.validate_capture(buffer, metadata, geometry)
                    except ValueError as error:
                        self.dropped += 1
                        self.error = str(error)
                        self.clear_preview()
                        return
                self.dropped += self.pending is not None
                self.pending = (stamp, buffer, self.generation, rgb, self.preview_generation)
                self.changed.notify()

    def preview(self):
        with self.changed:
            return self.latest_preview if self.enabled else None

    def clear_preview(self):
        # Viewer departure invalidates RGB in flight without resetting gaze tracks.
        with self.changed:
            self.latest_preview = None
            self.preview_generation += 1
            if self.pending is not None:
                stamp, buffer, generation, _, _ = self.pending
                self.pending = (stamp, buffer, generation, None, self.preview_generation)

    def observations(self):
        with self.changed:
            return self.tracker.observations(self.boot_clock()) if self.enabled and self.mode in ("idle", "faces") else []

    def status(self):
        with self.changed:
            return {"enabled": self.enabled, "rate_hz": self.rate_hz if self.enabled else 0, "inferences": self.inferences,
                "dropped_frames": self.dropped, "error": self.error,
                "gaze_available": self.gaze_available, "gaze_error": self.gaze_error,
                "object_lookup": self.find_job is not None, "objects_available": self.objects is not None and getattr(self.objects, "available", True) is not False,
                "palms_available": self.palms is not None, "tracking_mode": self.mode,
                "backends": {name: backend(detector) if detector is not None else 'unavailable'
                             for name, detector in (('faces', self.detector), ('palms', self.palms), ('objects', self.objects))},
                "hand_count": self.hand_count if self.mode == "hands" else 0}

    def needs_frame(self):
        with self.changed:
            return self.camera_enabled and (self.enabled or self.find_job is not None)

    def find(self, category, session=None, timeout=5.):
        from .detect_objects import CATEGORIES
        if category not in CATEGORIES:
            raise FindError('unknown_category', 'Use one of the COCO object categories')
        with self.changed:
            if not self.camera_enabled:
                raise FindError('camera_off', 'Camera is disabled')
            if self.objects is None:
                raise FindError('objects_unavailable', 'Object detector is unavailable')
            if self.find_job is not None:
                raise FindError('find_busy', 'An object lookup is already running')
            job = FindJob(category, session, self.capture_generation, self.clock() + timeout)
            self.find_job = job
            self.pending = self.latest_preview = None
            self.changed.notify_all()
        if not job.done.wait(timeout):
            with self.changed:
                job.error = job.error or FindError('find_timeout', 'Object lookup timed out')
                if not job.running:
                    job.done.set()
                self.changed.notify_all()
            # The worker retains CPU ownership until the child returns or is
            # killed and joined, even if the HTTP lookup deadline has expired.
            job.done.wait()
        if job.error:
            raise job.error
        return job.result

    def _find(self, job, stamp, buffer):
        try:
            if job.done.is_set() or self.clock() >= job.deadline:
                return
            if not self.client.find_allowed(job.session):
                raise FindError('voice_active', 'Object lookup is unavailable during voice activity')
            # Read geometry before inference; the measured pose history is only 1 s.
            camera = self.client.camera_pose(stamp)
            objects = self.objects.detect(buffer)
            candidates = locate([obj for obj in objects if obj.category == job.category],
                                self.intrinsics, camera, stamp)
            if not self.client.find_allowed(job.session):
                raise FindError('voice_active', 'Voice ownership changed during lookup')
            with self.changed:
                if not self.camera_enabled or job.generation != self.capture_generation:
                    raise FindError('camera_changed', 'Capture or tracking changed during lookup')
                if not job.done.is_set():
                    job.result = {'status': 'not_found' if not candidates else
                        'ambiguous' if len(candidates) > 1 else 'found', 'candidates': candidates}
        except Exception as error:
            if not job.done.is_set():
                job.error = error
        finally:
            with self.changed:
                if job.result is None and job.error is None:
                    job.error = FindError('find_timeout', 'Object lookup timed out')
                job.done.set()
                if self.find_job is job:
                    self.find_job = None

    def _clear_target(self):
        if self.target_selected:
            self.target_selected = False
            try:
                self.client.clear()
            except (OSError, ValueError):
                pass

    def _send_gaze(self, chosen):
        now = self.clock()
        try:
            if chosen is None:
                self._clear_target()
                return
            # Idle observes faces for autonomous reactions, but follows only
            # the speaker selected by a wake during a confirmed conversation.
            if self.mode == 'idle' and (chosen.id != self.speaker_track_id
                                       or self.client.conversation_active() is not True):
                self._clear_target()
                return
            if not self.gaze_available and now >= self.next_gaze_poll:
                self.next_gaze_poll = now + GAZE_RETRY_SECONDS
                self.gaze_available = self.client.gaze_available()
            if not self.gaze_available:
                return
            if chosen:
                if chosen.capture_ns <= self.last_gaze_capture:
                    return  # A retained track is not a fresh camera measurement.
                if self.client.target(chosen) is False:
                    self.gaze_available = False
                    self.next_gaze_poll = now + GAZE_RETRY_SECONDS
                    self.gaze_error = "Runtime gaze is unavailable"
                else:
                    self.target_selected = True
                    self.last_gaze_capture = chosen.capture_ns
                    self.gaze_error = None
        except (OSError, ValueError, KeyError) as error:
            # Motion availability does not invalidate detector/ray evidence.
            if self.mode == 'idle':
                self._clear_target()
            self.gaze_available = False
            self.next_gaze_poll = now + GAZE_RETRY_SECONDS
            self.gaze_error = str(error)

    def speaker(self, since_ns):
        with self.changed:
            result = self.tracker.speaker(since_ns, self.boot_clock())
            chosen = result['speaker']
            if chosen:
                self.speaker_track_id = chosen['track_id']
                self.speaker_until = self.clock() + 3.
                self.target_track_id = self.speaker_track_id
                self.challenger_track_id = self.challenger_since_ns = None
            return result

    def _choose_target(self, tracks, stamp):
        if self.speaker_track_id is not None and self.clock() < self.speaker_until:
            # A disappearing speaker never causes a turn to a bystander.
            return next((track for track in tracks if track.id == self.speaker_track_id), None)
        self.speaker_track_id = None
        if self.mode == 'idle':
            self.target_track_id = self.challenger_track_id = self.challenger_since_ns = None
            return None
        current = next((track for track in tracks if track.id == self.target_track_id), None)
        if current is None:
            chosen = choose_track(tracks)
            self.target_track_id = chosen.id if chosen else None
            self.challenger_track_id = self.challenger_since_ns = None
            return chosen
        area = current.face.box[2] * current.face.box[3]
        challenger = choose_track([track for track in tracks if track.id != current.id
            and track.capture_ns == stamp and track.face.looking_at_orion
            and track.face.box[2] * track.face.box[3] >= TARGET_AREA_RATIO * area])
        if challenger is None:
            self.challenger_track_id = self.challenger_since_ns = None
        elif challenger.id != self.challenger_track_id:
            self.challenger_track_id, self.challenger_since_ns = challenger.id, stamp
        elif stamp - self.challenger_since_ns >= TARGET_HOLD_NS:
            self.target_track_id = challenger.id
            self.challenger_track_id = self.challenger_since_ns = None
            return challenger
        return current

    def _tracking_frame(self, stamp, buffer, generation, rgb=None, preview_generation=None):
        detector = self.palms if self.mode == 'hands' else self.detector
        faces = detector.detect(buffer)
        bearings = self.client.bearings(stamp, self.intrinsics.rays([face.centre for face in faces])) if faces else []
        with self.changed:
            self.inferences += 1
            if not self.enabled or generation != self.generation:
                return
            # Drop late results instead of sending a stale target.
            if not 0 <= self.boot_clock() - stamp <= 1_000_000_000:
                self.error = "Detection result is stale"
                self.latest_preview = None
                return
            if self.mode == 'hands':
                self.hand_count = len(faces)
            elif not self._idle_palms_wanted():
                self.hand_count = 0
            fresh = self.tracker.update(stamp, faces, bearings)
            tracks = [track for track in self.tracker.tracks
                if 0 <= self.boot_clock() - track.capture_ns <= MAX_AGE_NS]
            chosen = self._choose_target(tracks, stamp)
            if rgb is not None and preview_generation == self.preview_generation:
                detections = [{"class": "hand" if self.mode == "hands" else "face", "box": track.observation(stamp)["box"],
                    "score": track.face.confidence, "track_id": track.id,
                    "chosen_target": track is chosen, "capture_boottime_ns": str(stamp)}
                    for track in fresh]
                self.latest_preview = (stamp, rgb, detections)
            else:
                self.latest_preview = None
        if self.mode != 'hands':
            self._advance_novelty(stamp, buffer, faces, generation)
        with self.commands:
            with self.changed:
                if not self.enabled or generation != self.generation:
                    return
            self._send_gaze(chosen)
            try:
                self._surroundings(stamp, fresh)
            except (OSError, ValueError, AttributeError) as error:
                self.gaze_error = str(error)
        try:
            if self.mode != 'hands':
                self._background_objects(stamp, buffer, faces, generation)
        except (OSError, ValueError, AttributeError):
            pass
        try:
            self._idle_palms(buffer, generation)
        except (OSError, ValueError, AttributeError):
            pass

        with self.changed:
            self.error = None

    def _run(self):
        next_inference = 0.
        last_inference = None
        active = True
        while not self.stop.is_set():
            with self.changed:
                if self.find_job and self.find_job.done.is_set():
                    self.find_job = None
                self.changed.wait_for(lambda: self.stop.is_set() or self.pending is not None, timeout=.1)
                if self.stop.is_set():
                    return
                if self.pending is None:
                    continue
                job = self.find_job
                if job is not None:
                    job.running = True
                    stamp, buffer, _, _, _ = self.pending
                    self.pending = None
            if job is not None:
                self._find(job, stamp, buffer)
                continue
            now = self.clock()
            if now >= self.next_voice_poll:
                try:
                    active = self.client.voice_active()
                except (OSError, ValueError, KeyError, TypeError):
                    active = True  # Reduced CPU use while voice state is unknown.
                if self.speaker_track_id is not None:
                    try:
                        conversing = self.client.conversation_active()
                    except (OSError, ValueError, KeyError, TypeError):
                        conversing = False
                    if conversing:
                        with self.changed:
                            self.speaker_until = now + 3.
                self.next_voice_poll = now + .2
            policy = rates(self.palms if self.mode == 'hands' else self.detector)
            self.rate_hz = policy.voice_hz if active else policy.idle_hz
            if last_inference is not None:
                next_inference = last_inference + 1 / self.rate_hz
            if now < next_inference:
                self.stop.wait(min(next_inference - now, .05))
                continue
            with self.changed:
                if not self.enabled or self.pending is None:
                    continue
                stamp, buffer, generation, rgb, preview_generation = self.pending
                self.pending = None
            last_inference = self.clock()
            next_inference = last_inference + 1 / self.rate_hz
            try:
                self._tracking_frame(stamp, buffer, generation, rgb, preview_generation)
            except Exception as error:
                with self.changed:
                    self.error = str(error)
                    self.tracker.clear()
                    self.target_track_id = self.challenger_track_id = self.challenger_since_ns = None
                    self.speaker_track_id = None
                    self.speaker_until = 0.
                    self.latest_preview = None
                with self.commands:
                    self._clear_target()
