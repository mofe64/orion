# Vision constraints

Captured frames stay in memory: never write them to disk. Replay fixtures are
synthetic input images, not captured camera frames. Only explicitly authorised
interfaces may send frames off the Pi (the authenticated Studio feed).

The service never moves the robot except through runtime `gaze` commands.
Never write servo goals or bypass the trajectory compiler and calibration.

Inference is bounded. Keep the latest frame and drop older frames instead of
queuing work. Camera callbacks only copy buffers and metadata; JPEG encoding,
file access and model work run outside them and outside the runtime tick.
