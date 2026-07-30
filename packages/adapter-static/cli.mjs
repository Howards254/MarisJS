#!/usr/bin/env node
import { readFileSync, existsSync, mkdirSync, cpSync, readdirSync, statSync, rmSync } from 'node:fs';
import { resolve, join, dirname, relative } from 'node:path';

const args = process.argv.slice(2);
if (args.length < 2) {
  console.error('Usage: marisjs-static <input-dist> <output-dir>');
  console.error('');
  console.error('  input-dist   Path to marisjs build output (contains routes.json)');
  console.error('  output-dir   Where to write the static-only deployment files');
  process.exit(1);
}

const INPUT = resolve(args[0]);
const OUTPUT = resolve(args[1]);

const manifestPath = join(INPUT, 'routes.json');
if (!existsSync(manifestPath)) {
  console.error(`Error: no routes.json found in ${INPUT}`);
  console.error("Run 'marisjs build' first.");
  process.exit(1);
}

const manifest = JSON.parse(readFileSync(manifestPath, 'utf-8'));

// ── Validate: no server routes ──────────────────────────────────────────

const serverRoutes = [];
for (const route of manifest.routes) {
  if (route.mode === 'server') {
    serverRoutes.push(route);
  }
}

if (serverRoutes.length > 0) {
  console.error(`Error: build contains ${serverRoutes.length} route(s) that require server-side execution.`);
  console.error('This adapter only produces fully static output. The following routes cannot be deployed statically:\n');
  for (const r of serverRoutes) {
    console.error(`  ${r.path} → ${r.file} (mode: server)`);
  }
  console.error(`\nThese routes use data() which re-executes per request.`);
  console.error('Use one of these options:');
  console.error('  1. Remove data() calls to make the route fully static.');
  console.error('  2. Use @maris/adapter-node for a server that handles both modes.');
  console.error('  3. Use a platform adapter (Vercel, Netlify) that supports SSR.');
  process.exit(1);
}

// ── Copy static files ───────────────────────────────────────────────────

console.log(`Validating static build: ${manifest.routes.length} route(s), all mode "static"`);

// Collect files to copy: route HTML files, CSS files listed in manifest,
// client component .mjs files, runtime.mjs, and any other static assets.
const filesToCopy = new Set();

// Route HTML files
for (const route of manifest.routes) {
  const htmlPath = join(INPUT, route.file);
  if (existsSync(htmlPath)) {
    filesToCopy.add(route.file);
  }
  // CSS files
  for (const css of (route.css || [])) {
    const cssPath = join(INPUT, css);
    if (existsSync(cssPath)) {
      filesToCopy.add(css);
    }
  }
  // Client module .mjs files
  for (const mod of (route.clientModules || [])) {
    const normalized = mod.path.replace(/^\.\//, '').replace(/[.][.]\//g, '');
    const modPath = join(INPUT, normalized);
    if (existsSync(modPath)) {
      filesToCopy.add(normalized);
    }
  }
}

// Runtime
filesToCopy.add('runtime.mjs');

// routes.json (for client-side routing if needed)
filesToCopy.add('routes.json');

// Walk for any other static files not in manifest (images, fonts, etc.)
function collectStaticFiles(dir, base = '') {
  if (!existsSync(dir)) return;
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const rel = base ? `${base}/${entry}` : entry;

    // Skip server-side and internal directories
    if (rel.startsWith('pages/')) continue;
    if (rel.startsWith('node_modules/')) continue;
    if (entry === '__build_timestamp.txt') continue;
    if (rel === 'routes.json') continue;

    if (statSync(full).isDirectory()) {
      collectStaticFiles(full, rel);
    } else {
      filesToCopy.add(rel);
    }
  }
}
collectStaticFiles(INPUT);

// ── Write output ─────────────────────────────────────────────────────────

rmSync(OUTPUT, { recursive: true, force: true });
mkdirSync(OUTPUT, { recursive: true });

let copied = 0;
for (const rel of filesToCopy) {
  const src = join(INPUT, rel);
  const dest = join(OUTPUT, rel);
  if (!existsSync(src)) continue;

  mkdirSync(dirname(dest), { recursive: true });
  cpSync(src, dest);
  copied++;
}

console.log(`Done: copied ${copied} file(s) to ${OUTPUT}`);
console.log('Ready for deployment to any static host (Netlify, Vercel, S3, GitHub Pages, etc.)');
