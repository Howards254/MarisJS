import { test, expect } from '@playwright/test';
import { execSync } from 'child_process';
import { createServer } from 'http';
import { readFileSync, existsSync, mkdirSync } from 'fs';
import { resolve, dirname, join, extname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../../..');
const distDir = join(projectRoot, 'dist');

let server;

test.beforeAll(async () => {
  // Build the example app
  execSync('cargo run -p cli -- build examples/todo-app --out dist', {
    cwd: projectRoot,
    stdio: 'inherit',
  });

  // Serve dist/ directly — build-generated index.html is at dist/index.html
  const port = 8765;
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
  console.log(`Test server on http://localhost:${port} (serving dist/)`);
});

test.afterAll(() => {
  if (server) server.close();
});

test('sibling-to-sibling reactivity via shared signal', async ({ page }) => {
  await page.goto('http://localhost:8765/');
  await page.waitForSelector('.app');

  let items = page.locator('.todo-item');
  await expect(items).toHaveCount(0);

  await page.fill('.todo-input', 'Buy milk');
  await page.waitForTimeout(100);
  await page.locator('.add-form button[type="submit"]').click();
  await page.waitForTimeout(300);
  await expect(items).toHaveCount(1);
  await expect(page.locator('.todo-text').first()).toHaveText('Buy milk');

  // CSS: todo item should have the border from TodoItem.css
  const itemBorder = await items.first().evaluate(el => getComputedStyle(el).border);
  expect(itemBorder).toContain('1px');
  expect(itemBorder).toContain('solid');

  // CSS: add form should have the background from AddTodoForm.css
  const formBg = await page.locator('.add-form').evaluate(el => getComputedStyle(el).backgroundColor);
  expect(formBg).toBe('rgb(240, 240, 240)');

  await page.fill('.todo-input', 'Walk dog');
  await page.waitForTimeout(100);
  await page.locator('.add-form button[type="submit"]').click();
  await page.waitForTimeout(300);
  await expect(items).toHaveCount(2);

  const editBtns = page.locator('.edit-btn');
  await editBtns.first().click();
  await expect(page.locator('.todo-text').first()).toHaveText('Editing: Buy milk');

  await page.fill('.todo-input', 'Read book');
  await page.click('.add-form button[type="submit"]');
  await expect(items).toHaveCount(3);

  await expect(page.locator('.todo-text').first()).toHaveText('Editing: Buy milk');
});

test('node identity survives across list mutations', async ({ page }) => {
  await page.goto('http://localhost:8765/');
  await page.waitForSelector('.app');

  await page.fill('.todo-input', 'Item A');
  await page.click('.add-form button[type="submit"]');
  await page.fill('.todo-input', 'Item B');
  await page.click('.add-form button[type="submit"]');

  await page.locator('.edit-btn').first().click();
  await expect(page.locator('.todo-text').first()).toHaveText('Editing: Item A');

  const firstId = await page.locator('.todo-item').first().getAttribute('data-id');

  await page.fill('.todo-input', 'Item C');
  await page.click('.add-form button[type="submit"]');

  const editedItem = page.locator(`.todo-item[data-id="${firstId}"]`);
  await expect(editedItem.locator('.todo-text')).toHaveText('Editing: Item A');
});

test('two independent client islands hydrate and operate in isolation', async ({ page }) => {
  await page.goto('http://localhost:8765/');
  await page.waitForSelector('.app');
  await page.waitForSelector('.newsletter');

  await page.fill('.todo-input', 'First task');
  await page.click('.add-form button[type="submit"]');
  await expect(page.locator('.todo-item')).toHaveCount(1);

  await expect(page.locator('.success-msg')).toHaveCount(0);
  await page.fill('.email-input', 'test@example.com');
  await page.click('.subscribe-btn');
  await expect(page.locator('.success-msg')).toHaveCount(1);
  await expect(page.locator('.success-msg')).toHaveText('Subscribed!');

  await expect(page.locator('.todo-item')).toHaveCount(1);
  await expect(page.locator('.todo-text').first()).toHaveText('First task');
});
