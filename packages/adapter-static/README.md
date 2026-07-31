# @marisjs/adapter-static

Produces a fully static deployment from marisjs build output — zero server, zero runtime
process. The output directory can be deployed to any static host.

```bash
npm install @marisjs/adapter-static
npx marisjs-static ./dist ./static-out
```

## What it does

1. **Validates** that every route in `routes.json` has `mode: "static"`. If any route
   requires server-side execution (`mode: "server"`, meaning it uses `data()`), the
   adapter fails with a clear error naming every affected route and suggesting fixes.

2. **Produces** a clean output directory containing only the files needed for a
   static deployment:
   - Route HTML files
   - Client component `.mjs` bundles
   - `runtime.mjs` (reactive runtime, needed for client hydration)
   - CSS files
   - Any additional static assets

3. **Excludes** server-only artifacts:
   - `node_modules/` directory (SSR resolution shim)
   - `pages/` directory (server page modules)
   - `__build_timestamp.txt` (dev artifact)

## Usage

```
marisjs-static <input-dist> <output-dir>
```

| Argument | Description |
|----------|-------------|
| `<input-dist>` | Path to marisjs build output (contains `routes.json`) |
| `<output-dir>` | Where to write the static-only deployment files |

## Error: server routes detected

If your build contains pages with `data()` calls, the adapter will reject them:

```
Error: build contains 1 route(s) that require server-side execution.
This adapter only produces fully static output. The following routes cannot be deployed statically:

  / → index.html (mode: server)

These routes use data() which re-executes per request.
Use one of these options:
  1. Remove data() calls to make the route fully static.
  2. Use @marisjs/adapter-node for a server that handles both modes.
  3. Use a platform adapter (Vercel, Netlify) that supports SSR.
```

## Platform targets

The output is a plain directory of HTML/CSS/JS files — no server process, no runtime
dependencies. Works on literally any static host:

- Netlify (drag-and-drop or CLI deploy)
- Vercel (static deployment)
- GitHub Pages
- Amazon S3 + CloudFront
- Cloudflare Pages
- Any CDN or object storage service
