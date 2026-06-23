// @ts-check
const { defineConfig, devices } = require("@playwright/test");
const path = require("path");

// prowl --web --mock を起動して DOM フロントを Playwright で検証する。
// --mock は実ネットワークに触れず固定データを返す（決定論的＝CI/再現可能）。
module.exports = defineConfig({
  testDir: "./tests",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  use: {
    baseURL: "http://127.0.0.1:7878",
    trace: "on-first-retry",
  },
  webServer: {
    command: "target/debug/prowl --web --mock --port 7878",
    cwd: path.resolve(__dirname, ".."),
    url: "http://127.0.0.1:7878",
    reuseExistingServer: false,
    timeout: 60_000,
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
