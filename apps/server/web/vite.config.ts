import { sentryVitePlugin } from "@sentry/bundler-plugins/vite";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv, type ViteDevServer } from "vite";

// Mirror the workspace routes served by Axum, without changing the browser URL.
function workspaceRoutes({ middlewares }: Pick<ViteDevServer, "middlewares">) {
  middlewares.use((req, res, next) => {
    const path = req.url?.split("?", 1)[0];
    if (path === "/sign" || path === "/pair" || (path && /^\/s\/[^/]+$/.test(path))) {
      req.url = "/workspace.html";
      res.setHeader("X-Robots-Tag", "noindex");
    }
    next();
  });
}

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
    appType: "mpa",
    build: {
      rolldownOptions: {
        input: ["index.html", "workspace.html", "terms/index.html", "privacy/index.html"],
      },
      // Keep fingerprinted output separate from public files with stable URLs.
      assetsDir: "assets/build",
      // Uninstrumented builds don't generate maps that could be served publicly.
      sourcemap: uploadSourceMaps ? "hidden" : false,
    },
    plugins: [
      {
        name: "apprelay-workspace-routes",
        configureServer: workspaceRoutes,
        configurePreviewServer: workspaceRoutes,
      },
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
