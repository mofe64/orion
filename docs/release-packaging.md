# Raspberry Pi releases

`scripts/build_release.sh vX.Y.Z` builds `artifacts/orion-X.Y.Z-arm64.tar.gz`
and its `.sha256` sidecar in ARM64 Docker containers. The same entry point runs
on GitHub’s `ubuntu-24.04-arm` runner for `v*` tags. Tags containing a hyphen
produce prereleases, so a test tag does not become the installer’s latest stable
release.

Shared Rust executables, the GGUF server, Python 3.12.13 and component wheels
build on Debian Bookworm (glibc 2.36). Camera/KMS/Hailo Python bindings build
separately against Raspberry Pi’s Bookworm and Trixie libraries. The installer
selects those bindings using `VERSION_CODENAME`; Python stays the same on both.
The build checks exact camera ABI versions and uses the HailoRT tag matching apt.
The `rpi-libcamera` build wrapper is pinned by Git revision because its required
private-header fix is not published on PyPI. It accepts each OS's pinned
libcamera source revision through Meson; the wrapper version does not select
the system ABI. Orion's small Meson patch handles the dependency layout in
those revisions, allows the binding's private base headers and leaves wheel
placement to meson-python. Patch failures stop the build. C++20 supports
Trixie's libcamera API.
KMS also uses a pinned wrapper and apt-matching source commit. Its wrapper
patch follows KMS's `utils` to `libutils` option rename. Both binding builds
use Python 3.12 and import in an isolated environment before packaging.

The tarball includes source/data needed at runtime, wheels, portable Python,
uv, systemd templates, pinned lighting driver sources, `bundle.json` and
`SHA256SUMS`. It excludes model weights, Git state, venvs and compiler caches.
The installer verifies the archive before extraction and verifies every
manifest file before creating release environments. Venvs install offline
without source builds or OS site-packages.
The interpreter publishes at `/opt/orion/python` only after a temporary copy
passes its version and standard-library checks. Failed copies are removed;
an unexpected existing directory requires inspection before retrying.

Models use pinned URLs and SHA-256 hashes from `prepare_pi_voice_stack.py`,
`release-models.json` and `check_hailo.py`. Downloads publish atomically and
verify existing files before reuse. Model Zoo benchmarking runs only on an
explicit `--check`; it does not change vision detector routing.

The `orion-models-v1` asset release supplies the existing NCNN export separately
from application tarballs. Preserve its exact weights and AGPL-3.0 attribution.
Publish those pinned assets before testing a clean installation.

Local development builds record `source_dirty` in `release.json`; tag builds
use the clean tagged tree. Git’s file inventory excludes ignored credentials and
recordings from the temporary Docker source. Compiler output uses a Linux Docker
volume rather than the source bind mount.

The installer retains `~/.local/share/orion/voice-stack/releases/` and the
existing transaction snapshot/rollback layout. It installs services and command
launchers together with built-in YAML; pairing, user assets, credentials and
shared model files persist. The direct servo check runs after the old daemon
releases the bus. A failed check leaves motion off and prints the zeroing step.
Systemd also runs that torque-free check before each daemon start.

Validate the runtime and other Rust crates with `cargo test --manifest-path
COMPONENT/Cargo.toml`. Run each Python component’s tests in its managed 3.12
venv. Geometry validation includes every-pose pre/post migration FK and
URDF/MJCF agreement; photo alignment is a separate measured correction with
projection uncertainty recorded in `model_reference-v2.json`.

Container tests cannot establish servo commissioning, camera capture, lighting
boot persistence or Hailo device/FPS results. Record those on the Pi after a
reboot. Rollback restores software and configuration; it cannot undo EEPROM
zeroing. A release using the old V2 coordinate convention must not restart
after rest-frame zeroing.

The installer supplies matching Pi 5 kernel and header metapackages. Lighting
builds with DKMS for the installed Pi 5 header trees, including the next kernel
when the flashed image's running kernel is older. Reboot before commissioning.
