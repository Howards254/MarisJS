import { test, expect } from '@playwright/test';
import { chromium } from 'playwright';
import { execSync, spawn } from 'child_process';
import http from 'http';
import path from 'path';
import fs from 'fs';
import os from 'os';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function createFixtureApp(dir) {
  // CSS: Base sets color: red, Override sets color: blue
  // Both target the .box class — the cascade should resolve to blue (last wins)
  fs.mkdirSync(path.join(dir, 'components'), { recursive: true });
  fs.mkdirSync(path.join(dir, 'pages'), { recursive: true });

  fs.writeFileSync(path.join(dir, 'components', 'base.css'), `.box { color: red; font-size: 20px; }`);
  fs.writeFileSync(path.join(dir, 'components', 'override.css'), `.box { color: blue; }`);

  // Leaf component with override.css (blue)
  fs.writeFileSync(path.join(dir, 'components', 'BlueBox.tsx'), `// @runsOn client
import "./override.css";
type BoxProps = { label: string; };
export function BlueBox(props: BoxProps) {
  return (<div class="box">{props.label}</div>);
}
`);

  // Middle component with base.css (red) — imports BlueBox
  fs.writeFileSync(path.join(dir, 'components', 'Wrapper.tsx'), `// @runsOn client
import "./base.css";
import { BlueBox } from './BlueBox';
type WrapperProps = {};
export function Wrapper(props: WrapperProps) {
  return (
    <div class="wrapper">
      <BlueBox label={'Hello World'} />
    </div>
  );
}
`);

  // Page: server component that wraps Wrapper
  fs.writeFileSync(path.join(dir, 'pages', 'Index.tsx'), `// @runsOn server
import { Wrapper } from '../components/Wrapper';
type IndexProps = {};
export function Index(props: IndexProps) {
  return (
    <div class="page">
      <Wrapper client:hydrate />
    </div>
  );
}
`);
}

// Find the CLI binary
function findCliBin() {
  // Walk up from __dirname (examples/todo-app/test) to workspace root
  let dir = path.resolve(__dirname, '../../..');
  const bin = path.join(dir, 'target', 'debug', 'cli');
  if (fs.existsSync(bin)) return bin;
  // Also try cargo-built binary
  const altBin = path.join(dir, 'target', 'debug', 'marisjs');
  if (fs.existsSync(altBin)) return altBin;
  throw new Error(`CLI binary not found, tried: ${bin}, ${altBin}`);
}

test('CSS cascade: later stylesheet wins for conflicting rules', async ({ page }) => {
  const fixtureDir = fs.mkdtempSync(path.join(os.tmpdir(), 'marisjs-css-'));
  const outDir = path.join(fixtureDir, 'dist');

  createFixtureApp(fixtureDir);

  // Build the app
  const cliBin = findCliBin();
  execSync(`"${cliBin}" build "${fixtureDir}" --out "${outDir}"`, {
    cwd: fixtureDir,
    stdio: 'pipe',
  });

  // Read the generated HTML to verify link tags exist in correct order
  const html = fs.readFileSync(path.join(outDir, 'index.html'), 'utf-8');
  expect(html).toContain('base.css');
  expect(html).toContain('override.css');
  const basePos = html.indexOf('base.css');
  const overridePos = html.indexOf('override.css');
  expect(basePos).toBeLessThan(overridePos);

  // Start a simple HTTP server
  const server = http.createServer((req, res) => {
    const url = req.url === '/' ? '/index.html' : req.url;
    const filePath = path.join(outDir, url);
    try {
      const content = fs.readFileSync(filePath);
      const ext = path.extname(filePath);
      const mime = { '.html': 'text/html', '.css': 'text/css', '.js': 'application/javascript', '.mjs': 'application/javascript', '.json': 'application/json' }[ext] || 'text/plain';
      res.writeHead(200, { 'Content-Type': mime, 'Access-Control-Allow-Origin': '*' });
      res.end(content);
    } catch (e) {
      res.writeHead(404);
      res.end();
    }
  });

  await new Promise(resolve => server.listen(0, resolve));
  const port = server.address().port;

  try {
    await page.goto(`http://localhost:${port}`, { waitUntil: 'networkidle' });

    // Verify the element exists and has the right text
    const box = page.locator('.box');
    await expect(box).toHaveText('Hello World');

    // getComputedStyle confirms the CASCADE behavior:
    // base.css { color: red } + override.css { color: blue }
    // → the element should be BLUE because override.css comes AFTER base.css in <link> order
    const color = await box.evaluate(el => getComputedStyle(el).color);
    expect(color).toBe('rgb(0, 0, 255)'); // blue

    // font-size should be 20px from base.css (override.css doesn't touch font-size)
    const fontSize = await box.evaluate(el => getComputedStyle(el).fontSize);
    expect(fontSize).toBe('20px');
  } finally {
    server.close();
    fs.rmSync(fixtureDir, { recursive: true, force: true });
  }
});
