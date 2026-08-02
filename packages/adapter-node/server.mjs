#!/usr/bin/env node
import { readFileSync, existsSync } from 'node:fs';
import { resolve, join, extname, basename } from 'node:path';
import { createServer } from 'node:http';

const PORT = parseInt(process.env.PORT || '3000', 10);
const DIST = resolve(process.argv[2] || process.env.MARISJS_DIST || './dist');

if (!existsSync(join(DIST, 'routes.json'))) {
  console.error(`No routes.json found in ${DIST}. Run 'marisjs build' first.`);
  process.exit(1);
}

const manifest = JSON.parse(readFileSync(join(DIST, 'routes.json'), 'utf-8'));

const mimeTypes = {
  '.html': 'text/html',
  '.mjs': 'application/javascript',
  '.js': 'application/javascript',
  '.css': 'text/css',
  '.json': 'application/json',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
  '.webp': 'image/webp',
  '.gif': 'image/gif',
};

const routeMap = new Map(manifest.routes.map(r => [r.path, r]));

function capitalizeFirst(s) {
  if (!s) return s;
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function htmlShell(serverHtml, route) {
  const cssLinks = (route.css || []).map(f => `  <link rel="stylesheet" href="/${f}">`).join('\n');
  const clientImports = (route.clientModules || []).map(m =>
    `    import { ${m.name} } from '${m.path}';`
  ).join('\n');
  const clientMounts = (route.clientModules || []).map(m =>
    `    mount(document.querySelector('[data-hydrate="${m.name}"]'), () => ${m.name}({}));`
  ).join('\n');

  return `<!DOCTYPE html>
<html>
<head>
  <script type="importmap">
  {
    "imports": {
      "@marisjs/runtime": "./runtime.mjs"
    }
  }
  </script>
${cssLinks}
</head>
<body>
  <div id="root">
  ${serverHtml}
  </div>
  <script type="module">
    import { mount } from '@marisjs/runtime';
${clientImports}
${clientMounts}
  </script>
</body>
</html>`;
}

const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, `http://localhost:${PORT}`);
    const route = routeMap.get(url.pathname);

    if (route) {
      if (route.mode === 'server') {
        const pageFile = join(DIST, route.file.replace('.html', '.mjs'));
        if (existsSync(pageFile)) {
          try {
            const stem = basename(pageFile, '.mjs');
            const exportName = capitalizeFirst(stem);
            const module = await import(pageFile);
            const PageComponent = module[exportName];
            if (typeof PageComponent === 'function') {
              const result = await PageComponent({});
              const html = typeof result === 'string' ? result : (result?.html || '');
              res.writeHead(200, { 'Content-Type': 'text/html' });
              res.end(htmlShell(html, route));
              return;
            }
          } catch (err) {
            console.error(`SSR failed for ${route.path}:`, err.message);
          }
        }
      }

      const filePath = join(DIST, route.file);
      if (existsSync(filePath)) {
        const content = readFileSync(filePath);
        const ext = extname(filePath);
        res.writeHead(200, {
          'Content-Type': mimeTypes[ext] || 'application/octet-stream',
        });
        res.end(content);
        return;
      }
    }

    // Serve arbitrary files from dist/ for script/CSS imports
    const filePath = join(DIST, url.pathname);
    if (existsSync(filePath) && !filePath.includes('/node_modules/') && !filePath.includes('\\node_modules\\')) {
      const ext = extname(filePath);
      res.writeHead(200, {
        'Content-Type': mimeTypes[ext] || 'application/octet-stream',
        'Access-Control-Allow-Origin': '*',
      });
      res.end(readFileSync(filePath));
      return;
    }

    res.writeHead(404);
    res.end('Not found');
  } catch (err) {
    console.error('Request error:', err);
    res.writeHead(500);
    res.end('Internal error');
  }
});

server.listen(PORT, () => {
  console.log(`marisjs adapter-node → http://localhost:${PORT}`);
  console.log(`Serving from: ${DIST}`);
  console.log(`Routes: ${manifest.routes.length}`);
});
