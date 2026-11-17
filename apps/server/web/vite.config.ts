import { sentryVitePlugin } from "@sentry/bundler-plugins/vite";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

export default defineConfig(({ command, mode }) => {
  const env = loadEnv(mode, process.cwd(), "VITE_");
  const authToken = process.env.SENTRY_AUTH_TOKEN;
  const uploadSourceMaps = command === "build" && Boolean(env.VITE_SENTRY_DSN);
  if (uploadSourceMaps) {
    for (const variable of ["SENTRY_AUTH_TOKEN", "SENTRY_ORG", "SENTRY_PROJECT"] as const) {
      if (!process.env[variable]?.trim()) {
        throw new Error(`${variable} is required to build with VITE_SENTRY_DSN`);
      }
    }
  }

  return {
    build: {
      // Uninstrumented builds don't generate maps that could be served publicly.
      sourcemap: uploadSourceMaps ? "hidden" : false,
    },
    plugins: [
      react(),
      tailwindcss(),
      ...(uploadSourceMaps
        ? [
            sentryVitePlugin({
              org: process.env.SENTRY_ORG,
              project: process.env.SENTRY_PROJECT,
              authToken,
              telemetry: false,
              release: {
                name: process.env.SENTRY_RELEASE,
              },
              sourcemaps: {
                assets: "./dist/**",
                filesToDeleteAfterUpload: ["./dist/**/*.map"],
              },
            }),
          ]
        : []),
    ],
    server: {
      port: 5173,
      strictPort: true,
      proxy: {
        "/v1": "http://127.0.0.1:8080",
        "/openapi.json": "http://127.0.0.1:8080",
      },
    },
  };
});
