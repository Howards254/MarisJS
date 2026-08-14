#!/usr/bin/env node
import { readFileSync, writeFileSync, existsSync, mkdirSync, cpSync, readdirSync, statSync, rmSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';

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

// ── Validate: no server routes, no API routes ───────────────────────────

const serverRoutes = [];
for (const route of manifest.routes) {
  if (route.mode === 'server') {
    serverRoutes.push(route);
  }
}

// §7b: fail-loud — an API route executes server code per request; a static
// host cannot run it. Same refusal pattern as server-mode routes.
const apiRoutes = manifest.apiRoutes || [];
if (apiRoutes.length > 0) {
  console.error(`Error: build contains ${apiRoutes.length} API route(s) that require server-side execution.`);
  console.error('This adapter only produces fully static output. The following API routes cannot be deployed statically:\n');
  for (const r of apiRoutes) {
    console.error(`  ${r.path} → ${r.file} (methods: ${r.methods.join(', ')})`);
  }
  console.error('\nThese routes execute handlers per request (e.g. env() reads, external fetches).');
  console.error('Use one of these options:');
  console.error('  1. Remove the api/ directory to make the build fully static.');
  console.error('  2. Use @marisjs/adapter-node for a server that handles both pages and API routes.');
  console.error('  3. Deploy the API routes separately (e.g. a platform adapter).');
  process.exit(1);
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
  console.error('  2. Use @marisjs/adapter-node for a server that handles both modes.');
  console.error('  3. Use a platform adapter (Vercel, Netlify) that supports SSR.');
  process.exit(1);
}

// ── Copy static files ───────────────────────────────────────────────────

console.log(`Validating static build: ${manifest.routes.length} route(s), all mode "static"`);

// Folder-URL convention: /docs → docs/index.html, /docs/api/signals →
// docs/api/signals/index.html. The page's HTML is emitted by the compiler
// with depth-aware relative references (../../../ prefix for a route 3
// segments deep), so moving it from docs.html to docs/index.html preserves
// the URL depth and every reference resolves unchanged. Root "/" stays
// index.html.
function folderUrlDest(route) {
  if (route.path === '/') return 'index.html';
  const rel = route.path.replace(/^\/+/, '').replace(/\/+$/, '');
  return `${rel}/index.html`;
}

// srcRel → destRel. Route HTML files go through the folder-URL mapping;
// everything else keeps its relative structure.
const filesToCopy = new Map();

// Route HTML files
for (const route of manifest.routes) {
  const htmlPath = join(INPUT, route.file);
  if (existsSync(htmlPath)) {
    filesToCopy.set(route.file, folderUrlDest(route));
  }
  // CSS files
  for (const css of (route.css || [])) {
    const cssPath = join(INPUT, css);
    if (existsSync(cssPath)) {
      filesToCopy.set(css, css);
    }
  }
  // Client module .mjs files
  for (const mod of (route.clientModules || [])) {
    const normalized = mod.path.replace(/^\.\//, '').replace(/[.][.]\//g, '');
    const modPath = join(INPUT, normalized);
    if (existsSync(modPath)) {
      filesToCopy.set(normalized, normalized);
    }
  }
}

// Runtime
filesToCopy.set('runtime.mjs', 'runtime.mjs');

// routes.json (rewritten below to match folder URLs)
filesToCopy.set('routes.json', 'routes.json');

// Walk for any other static files not in manifest (images, fonts, etc.)
function collectStaticFiles(dir, base = '') {
  if (!existsSync(dir)) return;
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const rel = base ? `${base}/${entry}` : entry;

    // Skip server-side and internal directories
    if (rel.startsWith('pages/')) continue;
    if (rel.startsWith('api/')) continue;
    if (rel.startsWith('node_modules/')) continue;
    if (entry === '__build_timestamp.txt') continue;
    if (rel === 'routes.json') continue;

    if (statSync(full).isDirectory()) {
      collectStaticFiles(full, rel);
    } else {
      // Don't clobber entries already mapped (e.g. route HTML files that
      // go through the folder-URL convention).
      if (!filesToCopy.has(rel)) {
        filesToCopy.set(rel, rel);
      }
    }
  }
}
collectStaticFiles(INPUT);

// ── Write output ─────────────────────────────────────────────────────────

rmSync(OUTPUT, { recursive: true, force: true });
mkdirSync(OUTPUT, { recursive: true });

let copied = 0;
for (const [srcRel, destRel] of filesToCopy) {
  const src = join(INPUT, srcRel);
  const dest = join(OUTPUT, destRel);
  if (!existsSync(src)) continue;

  mkdirSync(dirname(dest), { recursive: true });
  cpSync(src, dest);
  copied++;
}

// Rewrite routes.json in the output so its `file` entries match the
// folder-URL layout (client-side routing reads this manifest).
const outManifestPath = join(OUTPUT, 'routes.json');
if (existsSync(outManifestPath)) {
  const outManifest = JSON.parse(readFileSync(outManifestPath, 'utf-8'));
  for (const r of outManifest.routes) {
    r.file = folderUrlDest(r);
  }
  writeFileSync(outManifestPath, JSON.stringify(outManifest, null, 2));
}

console.log(`Done: copied ${copied} file(s) to ${OUTPUT}`);
console.log('Ready for deployment to any static host (Netlify, Vercel, S3, GitHub Pages, etc.)');
