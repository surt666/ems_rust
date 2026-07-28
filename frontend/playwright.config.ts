import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  use: { baseURL: "http://localhost:4321" },
  webServer: {
    command: "bun run build && bun run preview -- --port 4321",
    url: "http://localhost:4321/dev/schema-designer",
    timeout: 120_000,
    reuseExistingServer: true,
  },
});
