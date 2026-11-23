import { cpSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { defineConfig, devices } from "@playwright/test";
import { newAgeKey } from "./e2e/age-key";

// Every run starts from an empty backend, so tests see only what they make.
const data = mkdtempSync(join(tmpdir(), "famulen-e2e-"));
const config = join(data, "famulen.toml");
writeFileSync(config, `[claude]\nbin = ${JSON.stringify(resolve("e2e/fake-claude.pl"))}\n`);
cpSync(resolve("e2e/services"), join(data, "data", "gateway", "services"), { recursive: true });

const api = "127.0.0.1:17790";
const console = "http://127.0.0.1:13000";

export default defineConfig({
  testDir: "e2e",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: process.env.CI ? "github" : "list",
  use: { baseURL: console, trace: "retain-on-failure", locale: "ja-JP" },
  projects: [
    {
      name: "desktop",
      use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } },
      testIgnore: /mobile\.spec\.ts/,
    },
    { name: "mobile", use: { ...devices["Pixel 7"] }, testMatch: /mobile\.spec\.ts/ },
  ],
  webServer: [
    {
      command: `${resolve("../target/debug/famulen")} ${config}`,
      url: `http://${api}/health`,
      env: {
        FAMULEN_DATA_DIR: join(data, "data"),
        FAMULEN_LISTEN: api,
        FAMULEN_TIMEZONE: "Asia/Tokyo",
        FAMULEN_VAULT_KEY: newAgeKey(),
        FAMULEN_E2E_CLIENT_ID: "e2e",
      },
      reuseExistingServer: false,
      timeout: 60_000,
    },
    {
      command: "pnpm build && node .output/server/index.mjs",
      url: console,
      env: { PORT: "13000", HOST: "127.0.0.1", FAMULEN_API_URL: `http://${api}` },
      reuseExistingServer: false,
      timeout: 180_000,
    },
  ],
});
