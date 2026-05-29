import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

// the daemon defaults to 8056 (src/commands/serve.rs). override with
// GOVEE2MQTT_DEV_TARGET if you run it on another port.
const daemon = process.env.GOVEE2MQTT_DEV_TARGET ?? "http://127.0.0.1:8056";

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  server: {
    proxy: {
      "/api": { target: daemon, changeOrigin: true },
      "/ws": { target: daemon, changeOrigin: true, ws: true },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
