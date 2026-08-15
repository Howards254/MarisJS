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
const apiRouteMap = new Map((manifest.apiRoutes || []).map(r => [r.path, r]));

function capitalizeFirst(s) {
  if (!s) return s;
  return s.charAt(0).toUpperCase() + s.slice(1);
}

// A page served at /docs/api/signals (3 segments) sits 3 directories below
// the dist root, so root-relative references (runtime.mjs, CSS, client
// modules) must be prefixed with ../../../ to resolve in the browser.
function depthPrefix(routePath) {
  const segs = routePath.split('/').filter(Boolean).length;
  return '../'.repeat(segs);
}

// Codegen emits the server html string entity-escaped (so it survives the
// JSON round trip during prerendering); the compiler's prerender path
// unescapes it before writing the file. SSR must do the same, or browsers
// render literal &lt;h1&gt; text. Mirrors unescape_html in crates/cli.
function unescapeHtml(s) {
  return s
    .replaceAll('&lt;', '<')
    .replaceAll('&gt;', '>')
    .replaceAll('&#39;', "'")
    .replaceAll('&quot;', '"')
    .replaceAll('&amp;', '&');
}

function htmlShell(serverHtml, route) {
  const prefix = depthPrefix(route.path);
  const runtime = prefix ? `${prefix}runtime.mjs` : './runtime.mjs';
  const cssLinks = (route.css || []).map(f => `  <link rel="stylesheet" href="${prefix}${f}">`).join('\n');
  const seenImports = new Set();
  const clientImports = (route.clientModules || []).filter(m => {
    if (seenImports.has(m.name)) return false; // exactly ONE import per island
    seenImports.add(m.name);
    return true;
  }).map(m =>
    // A relative module specifier MUST start with ./ ../ or / — at the
    // root (no depth prefix) that means an explicit "./".
    `    import { ${m.name} } from '${prefix ? prefix : './'}${m.path}';`
  ).join('\n');
  const clientMounts = (route.clientModules || []).map(m =>
    // Mount EVERY instance of the island (multiple uses / inside <For>);
    // each placeholder carries its own data-props from SSR render time.
    `    for (const el of document.querySelectorAll('[data-hydrate="${m.name}"]')) { mount(el, () => ${m.name}(el.dataset.props ? JSON.parse(el.dataset.props) : {})); }`
  ).join('\n');

  return `<!DOCTYPE html>
<html>
<head>
  <script type="importmap">
  {
    "imports": {
      "@marisjs/runtime": ${JSON.stringify(runtime)}
    }
  }
  </script>
${cssLinks}
</head>
<body>
  <div id="root">
  ${unescapeHtml(serverHtml)}
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

    // §7b: API routes dispatch FIRST — an /api/* path is never a static
    // file. The handler is a plain ESM function receiving the standard Web
    // Request object and returning a Web Response (or a promise of one) —
    // the same contract as marisjs dev.
    const apiRoute = apiRouteMap.get(url.pathname);
    if (apiRoute) {
      const method = req.method.toUpperCase();
      if (!apiRoute.methods.includes(method)) {
        res.writeHead(405, { Allow: apiRoute.methods.join(', ') });
        res.end();
        return;
      }
      const apiFile = join(DIST, apiRoute.file);
      if (existsSync(apiFile)) {
        const module = await import(apiFile);
        const handler = module[method];
        if (typeof handler === 'function') {
          // Pass the request body as a stream (async-iterable). Content-
          // length is derived by undici — forwarding the raw header with a
          // stream body makes the Request constructor throw, so drop framing
          // and hop-by-hop headers.
          const forwardedHeaders = { ...req.headers };
          delete forwardedHeaders['content-length'];
          delete forwardedHeaders['transfer-encoding'];
          delete forwardedHeaders.connection;
          const request = new Request(url, {
            method,
            headers: forwardedHeaders,
            body: ['GET', 'HEAD'].includes(method) ? undefined : req,
            // undici requires duplex for stream/async-iterable bodies.
            duplex: 'half',
          });
          const response = await handler(request);
          // E4-06: collect headers as an array per name — multiple Set-Cookie
          // headers must ALL be forwarded (an object would keep only the last
          // one and silently drop security cookies). Node emits array values
          // as repeated header lines.
          const outHeaders = {};
          response.headers.forEach((v, k) => {
            const name = k.toLowerCase() === 'set-cookie' ? 'Set-Cookie' : k;
            outHeaders[name] = (outHeaders[name] || []).concat(v);
          });
          res.writeHead(response.status, outHeaders);
          res.end(Buffer.from(await response.arrayBuffer()));
          return;
        }
      }
      res.writeHead(404);
      res.end('Not found');
      return;
    }

    if (route) {
      if (route.mode === 'server') {
        // Use the real compiled path from the manifest (source-preserved
        // casing) — reconstructing it from the route string with
        // capitalizeFirst breaks for nested routes like /docs/api/signals.
        const pageFile = join(DIST, route.mjs || route.file.replace('.html', '.mjs'));
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

    // Serve arbitrary files from dist/ for script/CSS imports. E4-01: the
    // server-side module tree (_server/) and api/ files are NEVER served
    // statically — they carry the baked env snapshot (SESSION_SECRET, API
    // keys) and are only reachable through the dispatchers above.
    const firstSeg = url.pathname.split('/').filter(Boolean)[0] || '';
    if (firstSeg === '_server' || firstSeg === 'api') {
      res.writeHead(404);
      res.end('Not found');
      return;
    }
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
  console.log(`API routes: ${(manifest.apiRoutes || []).length}`);
});
