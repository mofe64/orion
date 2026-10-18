# Raspberry Pi releases

## App and asset packages

An app release contains Rust executables, Python 3.12.13, uv, component wheels, native wheels, systemd templates and lighting driver sources. It excludes model weights, Git data, virtual environments and compiler caches.

Bookworm builds shared executables against glibc 2.36. Bookworm and Trixie build separate camera, KMS and Hailo bindings. An application binary interface defines how compiled programs call a library. The installer selects native wheels from `VERSION_CODENAME`.

`scripts/release-native.json` defines native package versions and source revisions. The build applies camera and KMS patches from `scripts/native/`. A patch error stops the build.

An asset pack contains built-in YAML from `motion/config`, `motion/motions` and `scenes`. It also contains files from `audio/cues`. It excludes every `user` folder.

`orion-assets-X.Y.Z.tar.gz.manifest.json` defines the asset format version, joint-profile checksum and file checksums. The archive contains the same manifest as `asset-manifest.json`. The joint-profile checksum covers `hardware/profiles/v2-joints.json` from the app used for compilation.

App archives contain a compressed default pack for first installation. They do not contain loose built-in movement or cue files. Services read these files from the persistent catalog.

## Release branches and versions

`VERSION` contains the app version as `X.Y.Z`. `ASSETS_VERSION` contains the asset version in the same form. A release tag is a fixed name for one published version.

Pull requests into `release` build the selected package and run its checks. They publish nothing. Pushes to `release` create a tag and publish the package after the checks pass. An existing tag stops the workflow.

A change limited to asset paths and `ASSETS_VERSION` selects the asset build. This build downloads the latest stable app release. Its `oriond --validate-catalog` command compiles every pose, motion and scene without hardware access. V2 uses the app joint profile. V1 uses the app simulator calibration.

Any other change selects the full app build and its release checks. The full build also prepares the default asset pack. Its workflow publishes a new asset tag when that tag does not exist.

1. Change `ASSETS_VERSION` when you change built-in assets.
2. Change `VERSION` for a full app release.
3. Open a pull request into `release`.
4. Merge the pull request after its checks pass.

App tags use `vX.Y.Z`. Asset tags use `assets-vX.Y.Z`. Asset releases do not replace the latest app release. The installer selects each release type separately.

`workflow_dispatch` builds test prereleases from the selected branch. Supply a tag such as `v0.1.0-test.1`. A manual run cannot publish a stable release.

## Build locally

Run these steps from the repository root. Docker must support Linux ARM64 containers.

1. Check that all native package versions remain available.

   ```bash
   uv run --project motion --locked --managed-python python scripts/native_packages.py --check-archives
   ```

2. Build the app archive and asset pack.

   ```bash
   scripts/build_release.sh vX.Y.Z
   ```

The build writes packages, checksums and the asset manifest to `artifacts/`. An optional second argument selects another output directory.

The build copies Git's file inventory into a temporary source directory. Ignored credentials and recordings stay outside that directory. Local changes set `source_dirty` in `release.json`.

`.release-cache` holds Cargo files and the pinned llama.cpp build. GitHub Actions saves this directory between runs. The llama.cpp cache key includes its version and build configuration. Cargo keys include the Rust toolchain, lockfiles and Rust source files. Matching toolchain and lockfile keys can restore Cargo files after Rust source changes.

The unpacked payload uses a Linux Docker volume. This preserves file names that differ only in letter case on macOS.

The build runs [service startup checks](../tests/test_release_services.py) with separate release and catalog folders. The fixture has no Git metadata or `AGENTS.md`. It checks all six managed services and compiles a trajectory through the gateway. Device access and inference use test substitutes.

## Install and deploy

The [quickstart](quickstart.md) gives the published-release procedure.

1. Install a local app archive with its adjacent checksum file.

   ```bash
   bash scripts/install.sh --from artifacts/orion-X.Y.Z-arm64.tar.gz
   ```

2. Update only built-in assets when required.

   ```bash
   bash scripts/install.sh --assets
   ```

3. Select a particular asset release when required.

   ```bash
   bash scripts/install.sh --assets --version assets-vX.Y.Z
   ```

4. Deploy an app release from your computer through SSH.

   ```bash
   scripts/deploy_pi.sh --host USER@HOST --version vX.Y.Z
   ```

SSH requires an accepted host key. The remote login user needs sudo access.

The installer checks the archive checksum before extraction. It also checks each asset checksum and the app joint-profile checksum. An asset format or joint-profile mismatch stops installation.

Full installation fetches the latest compatible asset pack. First installation can use the bundled default when no compatible published pack exists. Later installations require a compatible published pack.

Asset updates use `catalog_plan` and the existing transaction snapshots. User assets remain unchanged. Asset updates restart `oriond` only when it was running. The previous catalog remains available through `--rollback`.

The installer creates offline virtual environments with `/opt/orion/python`. It holds pinned camera, KMS and installed Hailo packages with `apt-mark`. Rollback restores previous package versions before it restarts services.

## Models and release state

Models use pinned URLs and hashes from `prepare_pi_voice_stack.py` and `release-models.json`. The [vision reference](vision-reference.md#model-installation) defines optional Hailo models and their manifest. Downloads use temporary files before publication. Reuse includes a checksum check.

The `orion-models-v1` release supplies the NCNN model separately. Its AGPL-3.0 attribution stays with the model.

Release directories and transaction snapshots live under `~/.local/share/orion/voice-stack/`. Pairing, user assets, credentials and shared models persist across updates.

The service switch stops the old daemon before it checks the servo bus. First installation and old-frame transitions stay stopped for servo setup. Ordinary app updates resume after the hardware check passes.

Rollback cannot reverse servo EEPROM zeroing. Rest-frame releases require compatible V2 joint coordinates.
