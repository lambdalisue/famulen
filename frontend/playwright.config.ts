import { cpSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { defineConfig, devices } from "@playwright/test";
import { newAgeKey } from "./e2e/age-key";

// Every run starts from an empty backend, so tests see only what they make.
const data = mkdtempSync(join(tmpdir(), "famulen-e2e-"));
const config = join(data, "famulen.toml");
writeFileSync(config, `[claude]\nbin = ${JSON.stringify(resolve("e2e/fake-claude.pl"))}\n`);
cpSync(resolve("e2e/services"), join(data, "data", "gateway", "services"), { recursive: true });
const adroit = join(data, "adroit");
mkdirSync(adroit);
// Set here, as the workers that run the tests take it over.
process.env.E2E_GATES ??= join(data, "gates");
mkdirSync(process.env.E2E_GATES, { recursive: true });

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
        // The programs tests stand in for come before any on this machine.
        PATH: `${resolve("e2e/bin")}${delimiter}${process.env.PATH ?? ""}`,
        E2E_ADROIT_DIR: adroit,
        E2E_GATES: process.env.E2E_GATES,
      },
      reuseExistingServer: false,
      timeout: 60_000,
    },
    {
      command: "pnpm build && node .output/server/index.mjs",
      url: console,
      // Not the browser's locale, as the owner's server and browser need not share one.
      env: { PORT: "13000", HOST: "127.0.0.1", FAMULEN_API_URL: `http://${api}`, LC_ALL: "en_US.UTF-8" },
      reuseExistingServer: false,
      timeout: 180_000,
    },
  ],
});
