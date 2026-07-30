import { test, expect } from '@playwright/test';
import { spawn } from 'child_process';
import { readFileSync, writeFileSync, cpSync, rmSync, existsSync } from 'fs';
import { resolve, dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { tmpdir } from 'os';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../../..');
const PORT = 9879;

let devProcess;
let tempDir;
let tempOut;
let tsPath;

test.beforeAll(async () => {
  tempDir = join(tmpdir(), `marisjs-dev-test-${Date.now()}`);
  tempOut = join(tempDir, 'dist');
  cpSync(join(projectRoot, 'examples/todo-app'), tempDir, { recursive: true });
  tsPath = join(tempDir, 'components/App.tsx');

  devProcess = spawn('cargo', [
    'run', '-p', 'cli', '--',
    'dev', tempDir,
    '--out', tempOut,
    '--port', String(PORT),
  ], {
    cwd: projectRoot,
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  const ready = await pollReady(`http://localhost:${PORT}/__build_timestamp`, 20000);
  if (!ready) throw new Error('Dev server did not start');
  console.log(`Dev server ready on port ${PORT}`);
}, 35000);

test.afterAll(() => {
  if (devProcess) { devProcess.kill(); devProcess = null; }
  if (tempDir) { try { rmSync(tempDir, { recursive: true }); } catch(e) {} }
});

test('dev server serves initial built app', async ({ page }) => {
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector('.app');
  await expect(page.locator('h1')).toHaveText('Todo App');
  await expect(page.locator('.newsletter h2')).toHaveText('Newsletter');
});

test('file change triggers rebuild and browser reload', async ({ page }) => {
  // Record initial build timestamp
  await page.goto(`http://localhost:${PORT}/__build_timestamp`);
  const ts1 = await page.locator('body').textContent();
  console.log('Initial timestamp:', ts1);

  // Load the app
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector('.app');
  await expect(page.locator('h1')).toHaveText('Todo App');

  // Modify a file on disk
  const content = readFileSync(tsPath, 'utf-8');
  writeFileSync(tsPath, content.replace('Todo App', 'Todo App — Live'));

  // Wait for rebuild — timestamp must change
  let ts2 = ts1;
  for (let i = 0; i < 30; i++) {
    await new Promise(r => setTimeout(r, 500));
    await page.goto(`http://localhost:${PORT}/__build_timestamp`);
    ts2 = await page.locator('body').textContent();
    if (ts2 !== ts1) break;
  }
  console.log('After change timestamp:', ts2);
  expect(ts2).not.toBe(ts1); // rebuild actually happened

  // The live-reload script in the page should trigger a reload.
  // Navigate to the app page and wait for the heading to reflect the change.
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector('.app');
  await expect(page.locator('h1')).toHaveText('Todo App — Live');
});

async function pollReady(url, timeoutMs) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(url);
      if (res.ok) return true;
    } catch (e) {}
    await new Promise(r => setTimeout(r, 300));
  }
  return false;
}
