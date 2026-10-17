# Raspberry Pi releases

## Package contents

A release contains Rust executables, Python 3.12.13, uv, component wheels, native wheels, systemd templates and lighting driver sources. It excludes model weights, Git data, virtual environments and compiler caches.

Bookworm builds shared executables against glibc 2.36. Bookworm and Trixie build separate camera, KMS and Hailo bindings. An application binary interface defines how compiled programs call a library. The installer selects matching native wheels from `VERSION_CODENAME`.

`scripts/release-native.json` defines native package versions and source revisions. The build applies the patches in `scripts/native/` to the camera and KMS wrappers. A patch error stops the build.

## Build a release

Run these steps from the repository root. Docker must support Linux ARM64 containers.

1. Check that all native package versions remain available.

   ```bash
   uv run --project motion --locked --managed-python python scripts/native_packages.py --check-archives
   ```

2. Build the archive with a release tag.

   ```bash
   scripts/build_release.sh vX.Y.Z
   ```

The build writes `artifacts/orion-X.Y.Z-arm64.tar.gz` and its adjacent `.sha256` file. An optional second argument selects the output directory.

The build copies Git's file inventory into a temporary source directory. Ignored credentials and recordings stay outside that directory. Local changes set `source_dirty` in `release.json`.

Compiler caches and the unpacked payload use Linux Docker volumes. This preserves case-distinct file names on macOS. Cleanup removes temporary data without following compiler symlinks into the cache.

## Publish a release

The workflow in `.github/workflows/release.yml` runs on `v*` tags. It checks archive pins before building. It attaches the tarball and checksum to the GitHub release.

A tag containing a hyphen creates a prerelease. The installer's default selection uses the latest stable release.

## Install and deploy

The [quickstart](quickstart.md) gives the published-release procedure.

1. Install a local archive on the Pi with its adjacent checksum file.

   ```bash
   bash scripts/install.sh --from artifacts/orion-X.Y.Z-arm64.tar.gz
   ```

2. Deploy a published version from your computer through SSH.

   ```bash
   scripts/deploy_pi.sh --host USER@HOST --version vX.Y.Z
   ```

3. Deploy a local archive through SSH when needed.

   ```bash
   scripts/deploy_pi.sh --host USER@HOST --from artifacts/orion-X.Y.Z-arm64.tar.gz
   ```

SSH requires an accepted host key. The remote login user needs sudo access.

The installer verifies archive checksums before running bundled code. It verifies each payload file before creating offline virtual environments. `/opt/orion/python` supplies their interpreter without OS site-packages.

The installer holds pinned camera, KMS and installed Hailo packages with `apt-mark`. Release changes replace these holds. Rollback restores previous versions before restarting services. A missing pinned version stops installation with instructions to use a newer release.

## Models and release state

Models use pinned URLs and hashes from `prepare_pi_voice_stack.py`, `release-models.json` and `check_hailo.py`. Downloads use temporary files before publication. Reuse includes a checksum check.

The `orion-models-v1` release supplies the NCNN model separately. Its AGPL-3.0 attribution stays with the model.

Release directories and transaction snapshots live under `~/.local/share/orion/voice-stack/`. Pairing, user assets, credentials and shared models persist across updates.

The service switch stops the old daemon before checking the servo bus. First installation and old-frame transitions stay stopped for servo setup. Ordinary updates resume after the hardware check passes.

Rollback cannot reverse servo EEPROM zeroing. Rest-frame releases require compatible V2 joint coordinates.
