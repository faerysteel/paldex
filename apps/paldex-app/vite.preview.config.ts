import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

// Preview harness: renders real components against captured fixture data.
export default defineConfig({
  plugins: [react()],
  root: "preview",
  publicDir: fileURLToPath(new URL("./public", import.meta.url)),
  resolve: {
    alias: {
      "@tauri-apps/api/core": fileURLToPath(new URL("./preview/mock-core.ts", import.meta.url)),
      "@tauri-apps/api/event": fileURLToPath(new URL("./preview/mock-event.ts", import.meta.url)),
      "@tauri-apps/plugin-dialog": fileURLToPath(
        new URL("./preview/mock-dialog.ts", import.meta.url),
      ),
    },
  },
  server: { port: 5199, strictPort: true },
});
