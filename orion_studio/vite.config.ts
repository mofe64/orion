import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  assetsInclude: ["**/*.stl"],
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    fs: { allow: [path.resolve(import.meta.dirname, ".."), path.resolve(import.meta.dirname)] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // Tauri ships inside current system WebViews; Studio does not target legacy browsers.
    target: "es2022",
    minify: process.env.TAURI_ENV_DEBUG ? false : "oxc",
    sourcemap: Boolean(process.env.TAURI_ENV_DEBUG),
  },
});
