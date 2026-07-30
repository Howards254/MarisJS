import { test, expect } from '@playwright/test';
import { execSync } from 'child_process';
import { createServer } from 'http';
import { readFileSync } from 'fs';
import { resolve, dirname, join, extname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../../..');
const appDir = resolve(__dirname, '..');
const distDir = join(appDir, 'dist-settings');

let server;

test.beforeAll(async () => {
  execSync('cargo run -p cli -- build examples/settings-app --out examples/settings-app/dist-settings', {
    cwd: projectRoot,
    stdio: 'inherit',
  });

  const port = 8767;
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
  console.log(`Settings test server on http://localhost:${port}`);
});

test.afterAll(() => {
  if (server) server.close();
});

test('character-by-character validation tracking: disabled state and error message', async ({ page }) => {
  await page.goto('http://localhost:8767/');
  await page.waitForSelector('.settings-form');

  const saveBtn = page.locator('.save-btn');
  const nameInput = page.locator('#name');
  const nameError = page.locator('.name-error');
  const termsCheckbox = page.locator('.terms-checkbox');
  const termsError = page.locator('.terms-error');

  // ── Initial state: form is invalid ────────────────────────────────
  // Name empty → nameValid is false → form is invalid → Save disabled
  await expect(saveBtn).toBeDisabled();
  // Error message visible
  await expect(nameError).toBeVisible();
  await expect(nameError).toHaveText('Name is required.');

  // ── Type one character ────────────────────────────────────────────
  await nameInput.press('A');
  await expect(nameError).not.toBeVisible();

  // But form is still invalid (terms not yet accepted) → Save still disabled
  await expect(saveBtn).toBeDisabled();
  // Name error should be gone (name is now non-empty)
  await expect(nameError).not.toBeVisible();

  // ── Clear the name back to empty ─────────────────────────────────
  await nameInput.press('Backspace');
  await expect(nameError).toBeVisible();
  await expect(saveBtn).toBeDisabled();

  // ── Type character by character, verify error stays hidden ────────
  const name = 'Alice';
  for (const ch of name) {
    await nameInput.press(ch);
    await expect(nameError).not.toBeVisible();
  }
  await expect(nameInput).toHaveValue('Alice');

  // Still disabled — terms not checked yet
  await expect(saveBtn).toBeDisabled();

  // ── Check the terms checkbox ──────────────────────────────────────
  await termsCheckbox.check();
  await expect(termsError).not.toBeVisible();

  // Now form should be valid — Save should be enabled
  await expect(saveBtn).toBeEnabled();

  // ── Uncheck terms — Save should go back to disabled ──────────────
  await termsCheckbox.uncheck();
  await expect(termsError).toBeVisible();
  await expect(saveBtn).toBeDisabled();

  // ── Re-check and verify Save becomes enabled ─────────────────────
  await termsCheckbox.check();
  await expect(saveBtn).toBeEnabled();
});

test('select dropdown and number input reactivity', async ({ page }) => {
  await page.goto('http://localhost:8767/');
  await page.waitForSelector('.settings-form');

  const themeSelect = page.locator('#theme');
  const ageInput = page.locator('#age');
  const ageError = page.locator('.age-error');

  // Initial theme is 'light'
  await expect(themeSelect).toHaveValue('light');

  // Change theme to 'dark'
  await themeSelect.selectOption('dark');
  await expect(themeSelect).toHaveValue('dark');

  // Change theme to 'system'
  await themeSelect.selectOption('system');
  await expect(themeSelect).toHaveValue('system');

  // Age is 30 initially — valid, no error
  await expect(ageError).not.toBeVisible();

  // Set age to 0 — invalid
  await ageInput.fill('0');
  await expect(ageError).toBeVisible();

  // Set age to 150 — invalid
  await ageInput.fill('150');
  await expect(ageError).toBeVisible();

  // Set age to valid range
  await ageInput.fill('25');
  await expect(ageError).not.toBeVisible();
});

test('named event handlers: all five fire correctly', async ({ page }) => {
  await page.goto('http://localhost:8767/');
  await page.waitForSelector('.settings-form');

  const nameInput = page.locator('#name');
  const ageInput = page.locator('#age');
  const termsCheckbox = page.locator('.terms-checkbox');
  const themeSelect = page.locator('#theme');
  const saveBtn = page.locator('.save-btn');

  // Fill the form via each input's handler
  await nameInput.fill('Bob');
  await expect(nameInput).toHaveValue('Bob');

  await ageInput.fill('42');
  await expect(ageInput).toHaveValue('42');

  await themeSelect.selectOption('dark');
  await expect(themeSelect).toHaveValue('dark');

  await termsCheckbox.check();
  await expect(termsCheckbox).toBeChecked();

  // Now Save should be enabled and submit should work
  await expect(saveBtn).toBeEnabled();

  // The success message should NOT be visible yet
  const successMsg = page.locator('.success-msg');
  await expect(successMsg).not.toBeVisible();

  // Click Save — named handler fires
  await saveBtn.click();
  await expect(successMsg).toBeVisible();
  await expect(successMsg).toHaveText('Settings saved!');
});

test('boolean attribute: checked syncs with reactive signal', async ({ page }) => {
  await page.goto('http://localhost:8767/');
  await page.waitForSelector('.settings-form');

  const termsCheckbox = page.locator('.terms-checkbox');
  const saveBtn = page.locator('.save-btn');
  const nameInput = page.locator('#name');

  // Initially unchecked
  await expect(termsCheckbox).not.toBeChecked();

  // Fill name so only terms gate keeps button disabled
  await nameInput.fill('Test');

  // Terms unchecked → Save disabled
  await expect(saveBtn).toBeDisabled();

  // Check terms
  await termsCheckbox.check();
  await expect(termsCheckbox).toBeChecked();
  await expect(saveBtn).toBeEnabled();

  // Uncheck terms
  await termsCheckbox.uncheck();
  await expect(termsCheckbox).not.toBeChecked();
  await expect(saveBtn).toBeDisabled();

  // Re-check
  await termsCheckbox.check();
  await expect(termsCheckbox).toBeChecked();
  await expect(saveBtn).toBeEnabled();
});
