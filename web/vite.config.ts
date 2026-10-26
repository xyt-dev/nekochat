import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Dev server proxies the API and custom themes to a running nekochat server.
const target = process.env.NEKOCHAT_SERVER ?? "http://127.0.0.1:10000";

export default defineConfig({
  plugins: [solid()],
  worker: { format: "es" },
  build: { target: "es2022" },
  server: {
    proxy: {
      "/api": { target },
      "/themes": { target },
    },
  },
});
