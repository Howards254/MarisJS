import { test, expect } from '@playwright/test';
import { spawn } from 'child_process';
import { cpSync, rmSync } from 'fs';
import { resolve, dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { tmpdir } from 'os';

const __dirname = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(__dirname, '../../..');
const PORT = 9880;

let devProcess;
let tempDir;
let tempOut;

test.beforeAll(async () => {
  tempDir = join(tmpdir(), `marisjs-routes-${Date.now()}`);
  tempOut = join(tempDir, 'dist');
  cpSync(join(projectRoot, 'examples/todo-app'), tempDir, { recursive: true });

  devProcess = spawn('cargo', [
    'run', '-p', 'cli', '--',
    'dev', tempDir,
    '--out', tempOut,
    '--port', String(PORT),
  ], { cwd: projectRoot, stdio: ['ignore', 'pipe', 'pipe'] });

  const ready = await pollReady(`http://localhost:${PORT}/__build_timestamp`, 20000);
  if (!ready) throw new Error('Dev server did not start');
  console.log(`Dev server ready on port ${PORT}`);
}, 35000);

test.afterAll(() => {
  if (devProcess) { devProcess.kill(); devProcess = null; }
  if (tempDir) { try { rmSync(tempDir, { recursive: true }); } catch(e) {} }
});

test('route / serves the main page', async ({ page }) => {
  await page.goto(`http://localhost:${PORT}/`);
  await page.waitForSelector('.app');
  await expect(page.locator('h1').first()).toHaveText('Todo App');
});

test('route /about serves the about page', async ({ page }) => {
  const resp = await page.goto(`http://localhost:${PORT}/about`);
  expect(resp.status()).toBe(200);
  await expect(page.locator('h1')).toHaveText('About This App');
  await expect(page.locator('p')).toHaveText('A simple todo application built with marisjs.');
});

test('route /nonexistent returns 404', async ({ page }) => {
  let status = null;
  page.on('response', r => { if (r.url().includes('/nonexistent')) status = r.status(); });
  await page.goto(`http://localhost:${PORT}/nonexistent`, { waitUntil: 'commit' }).catch(() => {});
  expect(status).toBe(404);
});

async function pollReady(url, timeoutMs) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try { const res = await fetch(url); if (res.ok) return true; } catch (e) {}
    await new Promise(r => setTimeout(r, 300));
  }
  return false;
}
