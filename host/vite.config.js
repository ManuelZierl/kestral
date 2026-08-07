import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import { svelteTesting } from "@testing-library/svelte/vite";
import { configDefaults } from "vitest/config";

const host = process.env.TAURI_DEV_HOST;
const hostApiTarget = process.env.KESTRAL_HOST_API_PROXY_TARGET || "http://127.0.0.1:4310";

/** @type {import("vite").Plugin} */
const canonicalLoopbackOrigin = {
  name: "canonical-loopback-origin",
  configureServer(server) {
    server.middlewares.use((request, response, next) => {
      let requestedOrigin;
      try {
        requestedOrigin = new URL(`http://${request.headers.host}`);
      } catch {
        next();
        return;
      }
      if (requestedOrigin.hostname !== "127.0.0.1") {
        next();
        return;
      }

      const port = requestedOrigin.port ? `:${requestedOrigin.port}` : "";
      const requestTarget = request.url?.startsWith("/") ? request.url : "/";
      response.statusCode = 307;
      response.setHeader("Location", `http://localhost${port}${requestTarget}`);
      response.setHeader("Cache-Control", "no-store");
      response.end();
    });
  },
};

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [canonicalLoopbackOrigin, sveltekit(), svelteTesting()],

  test: {
    environment: "jsdom",
    exclude: [...configDefaults.exclude, "provider-worker/**"],
    // Mounted Svelte component tests (GrantPolicyEditor, AppSidebar,
    // ChatExtensionSlot, …) legitimately take several seconds under jsdom, and
    // on a loaded machine or Windows CI runner they intermittently exceeded the
    // 5s default and flaked. Genuine hangs are still caught by the suite's
    // explicit hung-frame isolation tests, not by this ceiling.
    testTimeout: 15000,
    hookTimeout: 15000,
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    // Browser host mode has one public development origin. Vite serves the UI
    // and HMR on 1420 while keeping the backend's 4310 listener host-local.
    proxy: {
      "/api": {
        target: hostApiTarget,
        changeOrigin: false,
      },
    },
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
