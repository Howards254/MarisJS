import { test, expect } from '@playwright/test';
import { execSync } from 'child_process';
import { createServer } from 'http';
import { readFileSync } from 'fs';
import { resolve, dirname, join, extname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../../..');
const appDir = resolve(__dirname, '..');
const distDir = join(appDir, 'dist-dashboard');

let server;

test.beforeAll(async () => {
  // Build the dashboard app
  execSync('cargo run -p cli -- build examples/dashboard-app --out examples/dashboard-app/dist-dashboard', {
    cwd: projectRoot,
    stdio: 'inherit',
  });

  const port = 8766;
  server = createServer((req, res) => {
    const url = new URL(req.url, `http://localhost:${port}`);
    let filePath = join(distDir, url.pathname === '/' ? 'index.html' : url.pathname);

    try {
      const content = readFileSync(filePath);
      const ext = extname(filePath).slice(1);
      const mime = {
        html: 'text/html', js: 'application/javascript',
        mjs: 'application/javascript', json: 'application/json',
        css: 'text/css',
      }[ext] || 'text/plain';
      res.writeHead(200, { 'Content-Type': mime, 'Access-Control-Allow-Origin': '*' });
      res.end(content);
    } catch (e) {
      res.writeHead(404);
      res.end('Not found: ' + filePath);
    }
  });

  await new Promise(r => server.listen(port, r));
  console.log(`Dashboard test server on http://localhost:${port}`);
});

test.afterAll(() => {
  if (server) server.close();
});

test('computed chains and style-driven bars update correctly', async ({ page }) => {
  await page.goto('http://localhost:8766/');
  await page.waitForSelector('.dashboard');

  // ── Initial state: stat cards ──────────────────────────────────────
  const statCards = page.locator('.stat-card');
  await expect(statCards).toHaveCount(5);

  // Level 1: totalRevenue = 450 + 720 + 300 = 1470
  await expect(statCards.nth(0).locator('.stat-value')).toHaveText('$1470');
  // Level 2: tax = round(1470 * 0.1) = 147
  await expect(statCards.nth(1).locator('.stat-value')).toHaveText('$147');
  // Level 3: net = 1470 - 147 = 1323
  await expect(statCards.nth(2).locator('.stat-value')).toHaveText('$1323');
  // Level 2: avg = round(1470 / 3) = 490
  await expect(statCards.nth(3).locator('.stat-value')).toHaveText('$490');
  // Level 3: percentChange — prior period 380+650+310=1340, (1470-1340)/1340 ≈ 10%
  await expect(statCards.nth(4).locator('.stat-value')).toHaveText('10%');

  // ── Initial state: bar chart ───────────────────────────────────────
  let bars = page.locator('.bar-wrapper');
  await expect(bars).toHaveCount(3);

  // Gadget (720) is the max → 100%. Widget (450) = 63%. Doodad (300) = 42%.
  // The bar-fill element should have a style="width: N%" attribute
  await expect(bars.nth(0).locator('.bar-pct')).toHaveText('63%');
  await expect(bars.nth(1).locator('.bar-pct')).toHaveText('100%');
  await expect(bars.nth(2).locator('.bar-pct')).toHaveText('42%');

  // Verify style attribute is actually set on the bar-fill elements
  const bar0Width = await bars.nth(0).locator('.bar-fill').getAttribute('style');
  expect(bar0Width).toContain('width:');
  expect(bar0Width).toContain('%');

  // Verify getComputedStyle on a bar-fill element
  const bar1Computed = await bars.nth(1).locator('.bar-fill').evaluate(el => getComputedStyle(el).width);
  expect(bar1Computed).not.toBe('0px');
  expect(bar1Computed).not.toBe('auto');

  // ── Mutate: add a new sale ────────────────────────────────────────
  await page.locator('.add-btn').click();
  await page.waitForTimeout(200);

  // Bar count should have increased
  bars = page.locator('.bar-wrapper');
  const newBarCount = await bars.count();
  expect(newBarCount).toBeGreaterThanOrEqual(4);

  // Stat cards should show updated values (total revenue increased)
  const updatedRevenueText = await statCards.nth(0).locator('.stat-value').textContent();
  const updatedRevenue = parseInt(updatedRevenueText.replace('$', ''), 10);
  expect(updatedRevenue).toBeGreaterThan(1470);

  // Net revenue should also have updated (Level 3 chain still working)
  const updatedNetText = await statCards.nth(2).locator('.stat-value').textContent();
  const updatedNet = parseInt(updatedNetText.replace('$', ''), 10);
  expect(updatedNet).toBeGreaterThan(1323);

  // Tax should be 10% of new total revenue
  const updatedTaxText = await statCards.nth(1).locator('.stat-value').textContent();
  const updatedTax = parseInt(updatedTaxText.replace('$', ''), 10);
  expect(updatedTax).toBe(Math.round(updatedRevenue * 0.1));

  // Net = revenue - tax (Level 3 computed chain)
  expect(updatedNet).toBe(updatedRevenue - updatedTax);

  // Avg = round(totalRevenue / new count)
  const updatedAvgText = await statCards.nth(3).locator('.stat-value').textContent();
  const updatedAvg = parseInt(updatedAvgText.replace('$', ''), 10);
  expect(updatedAvg).toBe(Math.round(updatedRevenue / newBarCount));

  // All new bars should have a style attribute on bar-fill
  const allBars = page.locator('.bar-wrapper');
  const barCount = await allBars.count();
  for (let i = 0; i < barCount; i++) {
    const style = await allBars.nth(i).locator('.bar-fill').getAttribute('style');
    expect(style).toContain('width:');
  }
});

test('deep computed chain: netRevenue reflects tax change from totalRevenue', async ({ page }) => {
  await page.goto('http://localhost:8766/');
  await page.waitForSelector('.dashboard');

  const statCards = page.locator('.stat-card');

  // Initial values
  const revText = await statCards.nth(0).locator('.stat-value').textContent();
  const taxText = await statCards.nth(1).locator('.stat-value').textContent();
  const netText = await statCards.nth(2).locator('.stat-value').textContent();

  const rev = parseInt(revText.replace('$', ''), 10);
  const tax = parseInt(taxText.replace('$', ''), 10);
  const net = parseInt(netText.replace('$', ''), 10);

  // net should equal revenue - tax (chain integrity)
  expect(net).toBe(rev - tax);

  // Average should be revenue / item count
  const avgText = await statCards.nth(3).locator('.stat-value').textContent();
  const avg = parseInt(avgText.replace('$', ''), 10);
  const barCount = await page.locator('.bar-wrapper').count();
  expect(avg).toBe(Math.round(rev / barCount));
});
