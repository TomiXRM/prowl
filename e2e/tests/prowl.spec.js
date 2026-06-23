// @ts-check
const { test, expect } = require("@playwright/test");

test("ホストが出てポートスキャンできる", async ({ page }) => {
  await page.goto("/");

  // WebSocket 接続できている
  await expect(page.getByTestId("conn")).toHaveClass(/conn-on/, { timeout: 15_000 });

  // 発見でホスト行が出る（数秒かかる）
  const rows = page.getByTestId("host-row");
  await expect(rows.first()).toBeVisible({ timeout: 25_000 });
  expect(await rows.count()).toBeGreaterThan(0);

  // 先頭ホストをクリック → 詳細＋ポートスキャン
  await rows.first().click();
  await expect(page.getByTestId("detail-ip")).toBeVisible();

  // スキャン完了（開放0でも "開放ポート: N" が出る）
  await expect(page.getByTestId("scan-state")).toContainText("開放ポート", {
    timeout: 20_000,
  });
});

test("監視トグルでバッジが変わる", async ({ page }) => {
  await page.goto("/");
  const badge = page.getByTestId("monitor-badge");
  await expect(badge).toContainText("監視");

  const before = (await badge.textContent())?.trim();
  await page.getByTestId("btn-monitor").click();
  await expect(badge).not.toHaveText(before ?? "", { timeout: 8_000 });
});

test("絞り込みでホスト行が減る", async ({ page }) => {
  await page.goto("/");
  const rows = page.getByTestId("host-row");
  await expect(rows.first()).toBeVisible({ timeout: 25_000 });

  // 絶対に一致しない文字列で絞ると 0 件になる
  await page.getByTestId("filter").fill("zzzzz-no-match");
  await expect(rows).toHaveCount(0);
});
