import { test, expect } from '@playwright/test';
import { execSync } from 'child_process';
import { createServer } from 'http';
import { readFileSync } from 'fs';
import { resolve, dirname, join, extname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../../..');
const appDir = resolve(__dirname, '..');
const distDir = join(appDir, 'dist-islands');

let server;

test.beforeAll(async () => {
  execSync('cargo run -p cli -- build examples/islands-app --out examples/islands-app/dist-islands', {
    cwd: projectRoot,
    stdio: 'inherit',
  });

  const port = 8768;
  server = createServer((req, res) => {
    const url = new URL(req.url, `http://localhost:${port}`);
    let filePath = join(distDir, url.pathname === '/' ? 'index.html' : url.pathname);
    try {
      const content = readFileSync(filePath);
      const ext = extname(filePath).slice(1);
      const mime = { html: 'text/html', js: 'application/javascript', mjs: 'application/javascript', json: 'application/json', css: 'text/css' }[ext] || 'text/plain';
      res.writeHead(200, { 'Content-Type': mime, 'Access-Control-Allow-Origin': '*' });
      res.end(content);
    } catch (e) { res.writeHead(404); res.end(); }
  });

  await new Promise(r => server.listen(port, r));
  console.log(`Islands test server on http://localhost:${port}`);
});

test.afterAll(() => { if (server) server.close(); });

test('three independent client islands coexist and operate in isolation', async ({ page }) => {
  await page.goto('http://localhost:8768/');
  await page.waitForSelector('.search-island');
  await page.waitForSelector('.like-island');
  await page.waitForSelector('.theme-island');

  // ── Island 1: SearchBar ──────────────────────────────────────────
  const searchInput = page.locator('.search-input');
  const searchStatus = page.locator('.search-status');
  const clearBtn = page.locator('.clear-btn');

  // Initially nothing shown
  await expect(searchStatus).not.toBeVisible();

  // Type in search
  await searchInput.fill('phone');
  await expect(searchStatus).toBeVisible();
  await expect(searchStatus).toHaveText('Searching for: phone');

  // Clear — status should disappear
  await clearBtn.click();
  await expect(searchStatus).not.toBeVisible();

  // Type again
  await searchInput.fill('laptop');
  await expect(searchStatus).toHaveText('Searching for: laptop');

  // ── Island 2: LikeCounter ────────────────────────────────────────
  const likeCount = page.locator('.like-count');
  const likeBtn = page.locator('.like-btn');

  // Initial counter
  await expect(likeCount).toHaveText('❤️ 0 likes');

  // Click like 3 times
  await likeBtn.click();
  await likeBtn.click();
  await likeBtn.click();
  await expect(likeCount).toHaveText('❤️ 3 likes');

  // ── Island 3: ThemeToggle ─────────────────────────────────────────
  const themeIsland = page.locator('.theme-island');
  const themeLabel = page.locator('.theme-label');
  const themeBtn = page.locator('.theme-btn');

  // Initial theme is light
  await expect(themeIsland).toHaveClass(/theme-light/);
  await expect(themeLabel).toHaveText('Light mode');

  // Toggle to dark
  await themeBtn.click();
  await expect(themeIsland).toHaveClass(/theme-dark/);
  await expect(themeLabel).toHaveText('Dark mode');

  // Toggle back to light
  await themeBtn.click();
  await expect(themeIsland).toHaveClass(/theme-light/);
  await expect(themeLabel).toHaveText('Light mode');

  // ── Cross-check: islands don't interfere ──────────────────────────
  // Search status should still show 'laptop' (unchanged by other islands)
  await expect(searchStatus).toHaveText('Searching for: laptop');

  // Like count should still be 3
  await expect(likeCount).toHaveText('❤️ 3 likes');
});
