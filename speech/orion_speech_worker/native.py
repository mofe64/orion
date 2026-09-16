"""Exec a Linux model server that cannot outlive its owning inference worker."""
import ctypes
import os
import signal
import sys


def main():
    parent = os.getppid()
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(1, signal.SIGTERM, 0, 0, 0) != 0:
        raise OSError(ctypes.get_errno(), "Could not bind model server lifetime")
    if os.getppid() != parent or parent == 1:
        raise SystemExit("Inference worker exited before model server started")
    os.execv(sys.argv[1], sys.argv[1:])


if __name__ == "__main__":
    main()
