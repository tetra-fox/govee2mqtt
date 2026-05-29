import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

// the daemon defaults to 8056 (src/commands/serve.rs). override with
// GOVEE2MQTT_DEV_TARGET if you run it on another port.
const daemon = process.env.GOVEE2MQTT_DEV_TARGET ?? "http://127.0.0.1:8056";

export default defineConfig(({ command }) => ({
  plugins: [tailwindcss(), svelte()],
  // build with relative asset urls so the bundled index.html works under any
  // mount path. ha ingress serves the addon at /api/hassio_ingress/<token>/,
  // so the default base: "/" would make the browser look for /assets/* on
  // the ha frontend itself and 404. dev server stays at "/" because vite's
  // hmr client wants an absolute base.
  base: command === "build" ? "./" : "/",
  server: {
    proxy: {
      "/api": { target: daemon, changeOrigin: true },
      "/ws": { target: daemon, changeOrigin: true, ws: true },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // the rust daemon embeds dist/ into the binary, so wire size = on-disk
    // binary footprint. squeeze it harder than esbuild's defaults: bump the
    // syntax target so async/optional-chaining/nullish-coalescing stay as
    // single chars instead of being lowered, switch to terser with two
    // passes + property mangling of internal `_` prefixed names, and drop
    // debugger statements + console.log we accidentally leave behind.
    target: "es2025",
    minify: "terser",
    terserOptions: {
      // tell terser the output baseline so it emits 2025 idioms (set
      // methods, Promise.try, iterator helpers) verbatim instead of falling
      // back to longer down-leveled forms.
      ecma: 2025,
      compress: {
        ecma: 2025,
        passes: 2,
        drop_debugger: true,
        pure_funcs: ["console.debug"],
      },
      mangle: {
        // private-by-convention members get short names; public APIs and
        // svelte-generated identifiers stay readable so stack traces remain
        // useful when debugging in prod.
        properties: { regex: /^_/ },
      },
      format: { ecma: 2025, comments: false },
    },
    // print real br/gzip sizes so the build log doubles as a budget check.
    reportCompressedSize: true,
  },
}));
