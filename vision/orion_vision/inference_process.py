"""One synchronous inference child; bounded in-memory IPC, never a frame queue."""
import logging
import multiprocessing
from threading import Event, Lock
import time

LOG = logging.getLogger(__name__)
INFERENCE_TIMEOUT_SECONDS = 15.
STARTUP_TIMEOUT_SECONDS = 20.


def _worker(connection, factory, argument):
    detector = None
    try:
        detector = factory(argument)
        connection.send(('ready', None))
        while True:
            pixels = connection.recv()
            if pixels is None:
                return
            try:
                connection.send(('result', detector.detect(pixels)))
            except Exception as error:
                connection.send(('error', f'{type(error).__name__}: {error}'))
            finally:
                pixels = None
    except (EOFError, BrokenPipeError):
        pass
    except Exception as error:
        try:
            connection.send(('error', f'{type(error).__name__}: {error}'))
        except (EOFError, BrokenPipeError, OSError):
            pass
    finally:
        try:
            if detector is not None and hasattr(detector, 'close'):
                detector.close()
        finally:
            connection.close()


class InferenceProcess:
    def __init__(self, factory, argument, *, timeout=INFERENCE_TIMEOUT_SECONDS, name='Object'):
        self.name = name
        self.timeout = timeout
        self.lock = Lock()
        self.closed = Event()
        context = multiprocessing.get_context('spawn')
        self.connection, child = context.Pipe()
        self.process = context.Process(target=_worker, args=(child, factory, argument),
                                       name='orion-yolo' if name == 'Object' else 'orion-' + name.lower(), daemon=True)
        try:
            self.process.start()
            child.close()
            kind, message = self._receive(STARTUP_TIMEOUT_SECONDS)
            if kind != 'ready':
                raise RuntimeError(f'{self.name} detector startup failed: {message}')
        except BaseException:
            child.close()
            self.close()
            raise

    @property
    def available(self):
        return not self.closed.is_set() and self.process.is_alive()

    def _receive(self, timeout):
        deadline = time.monotonic() + timeout
        while not self.closed.is_set():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f'{self.name} inference worker timed out')
            if self.connection.poll(min(.05, remaining)):
                return self.connection.recv()
            if not self.process.is_alive():
                raise RuntimeError(f'{self.name} inference worker exited')
        raise RuntimeError(f'{self.name} inference worker closed')

    def _terminate(self):
        if self.process.pid is not None:
            if self.process.is_alive():
                self.process.terminate()
            self.process.join(1)
            if self.process.is_alive():
                self.process.kill()
                self.process.join(1)
        self.connection.close()

    def detect(self, pixels):
        # FacePipeline alone calls this. The lock also prevents accidental
        # concurrent callers from queuing frames or overlapping extraction.
        with self.lock:
            if not self.available:
                raise RuntimeError(f'{self.name} detector unavailable')
            started = time.monotonic()
            outcome = 'error'
            try:
                self.connection.send(pixels)
                kind, value = self._receive(self.timeout)
                if kind != 'result':
                    raise RuntimeError(f'{self.name} inference failed: {value}')
                outcome = 'ok'
                return value
            except (EOFError, OSError, RuntimeError):
                # Kill/join before returning CPU ownership to the inference
                # worker. A wedged child must not overlap the resumed detector.
                self.closed.set()
                self._terminate()
                raise
            finally:
                if self.name == 'Object':
                    LOG.info('YOLO inference outcome=%s duration_ms=%.1f', outcome,
                             (time.monotonic() - started) * 1000)

    def close(self):
        self.closed.set()
        with self.lock:
            self._terminate()
