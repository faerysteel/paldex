import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri injects this when developing against a physical device on the LAN.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  // Preview fixtures live under public/, but desktop bundles must never copy them.
  publicDir: false,

  // Tauri owns the terminal output; don't let Vite wipe its messages.
  clearScreen: false,

  server: {
    port: 1420,
    // A shifting port would silently break the Tauri dev window.
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      // Rust rebuilds are driven by cargo, not Vite.
      ignored: ["**/src-tauri/**"],
    },
  },
});
