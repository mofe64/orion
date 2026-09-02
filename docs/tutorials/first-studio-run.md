# Run Orion Studio

Orion Studio can browse, edit, and preview local assets without connecting to
the Pi or enabling Voice.

## Prerequisites

Install:

- Node.js 20 or newer.
- pnpm 11.
- The stable Rust toolchain.
- The [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for
  your operating system.

The repository is commissioned on macOS Apple Silicon. Windows and Linux are
uncommissioned build targets; see
[platform support](../reference/platform-support.md).

## 1. Install the Studio dependencies

From the repository root:

```bash
cd orion_studio
pnpm install
```

## 2. Start the desktop application

```bash
pnpm tauri dev
```

Studio resolves the Orion checkout from its Tauri crate during source
development. If the checkout is relocated or packaged, provide it explicitly:

```bash
ORION_PROJECT_ROOT=/absolute/path/to/orion pnpm tauri dev
```

## 3. Confirm local operation

The application should load the real Orion URDF and the built-in pose, motion,
and scene libraries. Editing a joint slider or timeline remains local and does
not move hardware.

The UI-only development server is useful for frontend work:

```bash
pnpm dev
```

It cannot launch native workers or use the complete Studio-to-Pi path. Use
`pnpm tauri dev` for Voice or native integration.

## 4. Validate the application

```bash
pnpm test
pnpm build
```

To use the workstation microphone, continue with
[the Studio Voice tutorial](first-studio-voice-run.md). To control physical
hardware, follow [Connect Studio to the Pi](../../orion_studio/README.md#connect-studio-to-the-pi).
