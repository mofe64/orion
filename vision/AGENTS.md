# Vision constraints

Captured camera frames stay in memory. Never write them to disk. Synthetic replay fixtures can use image files. Only explicitly authorized interfaces can send frames outside the Pi. The authenticated Studio feed is one such interface.

Send movement requests through runtime gaze commands. Never write servo goals directly. Keep the trajectory compiler and joint-limit checks in the movement path.

Keep only the latest frame for inference. Drop older frames instead of queuing them. Camera callbacks copy buffers and metadata only. Run JPEG encoding, file access and model work outside callbacks and the runtime tick.
