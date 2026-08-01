# @marisjs/adapter-node

A plain Node.js HTTP server for marisjs build output. Zero framework dependencies.

```bash
npm install @marisjs/adapter-node
npx @marisjs/adapter-node ./dist      # serve the build output
PORT=8080 npx @marisjs/adapter-node ./dist  # custom port via PORT= env
```

## What it does

- Reads the `routes.json` manifest produced by `marisjs build`
- Serves `mode: "static"` routes as prerendered HTML files directly from disk
- Re-executes `mode: "server"` routes (pages with `data()`) by dynamically importing the page module, calling the component, and wrapping the result in the standard HTML shell
- Serves arbitrary files from the output directory for script/CSS asset requests
- Blocks browser access to the internal `node_modules/` shim

## Usage

```
npx @marisjs/adapter-node <dist-directory>
```

After installation the same server is on your PATH as `marisjs-serve`.

| Argument | Default | Description |
|----------|---------|-------------|
| `<dist-dir>` | `./dist` | Path to the marisjs build output directory |
| `PORT` env | `3000` | HTTP port |

## Adapter contract

This adapter implements the [marisjs adapter interface](../../docs/adapter-interface.md).

- Reads `routes.json` from the build output
- Static routes: serves the HTML file directly
- Server routes: `import()`s the page `.mjs` module, calls the component, generates the HTML shell with import map, CSS links, and client hydration scripts
- No knowledge of marisjs language rules or component structure

## Platform targets

Works on any platform that can run Node.js:

- Self-hosted VPS / bare metal
- Docker containers
- Railway / Render / Fly.io
- Local production testing
