import { test, expect } from '@playwright/test';

const BASE_URL = 'http://localhost:3000';

test('three independent client islands coexist via adapter-node', async ({ page }) => {
  await page.goto(`${BASE_URL}/`);
  await page.waitForSelector('.search-island');
  await page.waitForSelector('.like-island');
  await page.waitForSelector('.theme-island');

  // ── Island 1: SearchBar ──────────────────────────────────────────
  const searchInput = page.locator('.search-input');
  const searchStatus = page.locator('.search-status');
  const clearBtn = page.locator('.clear-btn');

  await expect(searchStatus).not.toBeVisible();

  await searchInput.fill('phone');
  await expect(searchStatus).toBeVisible();
  await expect(searchStatus).toHaveText('Searching for: phone');

  await clearBtn.click();
  await expect(searchStatus).not.toBeVisible();

  await searchInput.fill('laptop');
  await expect(searchStatus).toHaveText('Searching for: laptop');

  // ── Island 2: LikeCounter ────────────────────────────────────────
  const likeCount = page.locator('.like-count');
  const likeBtn = page.locator('.like-btn');

  await expect(likeCount).toHaveText('❤️ 0 likes');

  await likeBtn.click();
  await likeBtn.click();
  await likeBtn.click();
  await expect(likeCount).toHaveText('❤️ 3 likes');

  // ── Island 3: ThemeToggle ─────────────────────────────────────────
  const themeIsland = page.locator('.theme-island');
  const themeLabel = page.locator('.theme-label');
  const themeBtn = page.locator('.theme-btn');

  await expect(themeIsland).toHaveClass(/theme-light/);
  await expect(themeLabel).toHaveText('Light mode');

  await themeBtn.click();
  await expect(themeIsland).toHaveClass(/theme-dark/);
  await expect(themeLabel).toHaveText('Dark mode');

  await themeBtn.click();
  await expect(themeIsland).toHaveClass(/theme-light/);
  await expect(themeLabel).toHaveText('Light mode');

  // ── Cross-check: islands don't interfere ──────────────────────────
  await expect(searchStatus).toHaveText('Searching for: laptop');
  await expect(likeCount).toHaveText('❤️ 3 likes');
});

test('adapter serves static HTML with correct content-type', async ({ page }) => {
  const response = await page.goto(`${BASE_URL}/`);
  expect(response.status()).toBe(200);
  expect(response.headers()['content-type']).toContain('text/html');
});

test('adapter serves runtime.mjs as JavaScript', async ({ page }) => {
  const response = await page.goto(`${BASE_URL}/runtime.mjs`);
  expect(response.status()).toBe(200);
  expect(response.headers()['content-type']).toContain('javascript');
});

test('adapter returns 404 for unknown routes', async ({ page }) => {
  const response = await page.goto(`${BASE_URL}/nonexistent`);
  expect(response.status()).toBe(404);
});

test('adapter does not expose node_modules to browser', async ({ page }) => {
  const response = await page.goto(`${BASE_URL}/node_modules/@marisjs/runtime/package.json`);
  expect(response.status()).toBe(404);
});
