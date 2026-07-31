# MarisJS MCP Server

Register the marisjs validator as an MCP tool so AI coding agents (Claude Code, opencode) can validate marisjs component files.

## Build

```bash
cargo build --release -p mcp-server
```

The binary is `target/release/marisjs-mcp`. It communicates over stdio using the [Model Context Protocol](https://modelcontextprotocol.io/).

## Tool

### `validate_component`

Accepts a file path or raw TSX source code. Returns the exact same structured JSON that `marisjs validate` produces:

```json
{
  "valid": true,
  "errors": [
    {
      "line": 2,
      "column": 0,
      "code": "FORBIDDEN_IMPORT",
      "message": "Import from 'react' is not allowed. Use marisjs primitives instead.",
      "fix_hint": "Remove this import and use <For>, signal(), or computed() from @marisjs/runtime."
    }
  ]
}
```

## Setup

### opencode

Add to your project's `opencode.json` or `opencode.jsonc`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "marisjs": {
      "type": "local",
      "command": ["marisjs-mcp"],
      "enabled": true
    }
  }
}
```

Or point directly at the release binary:

```json
{
  "mcp": {
    "marisjs": {
      "type": "local",
      "command": ["./target/release/marisjs-mcp"],
      "enabled": true
    }
  }
}
```

Restart opencode. The `validate_component` tool is now available to the agent.

### Claude Code (VS Code / Claude Desktop)

Add to `~/.claude/claude_desktop_config.json` or `.mcp.json` in your project:

```json
{
  "mcpServers": {
    "marisjs": {
      "command": "marisjs-mcp",
      "args": []
    }
  }
}
```

If the binary isn't on PATH, use the absolute path instead.

## Development

```bash
# Build
cargo build -p mcp-server

# Run tests (11 tests, same fixtures as CLI tests)
cargo test -p mcp-server

# Run the server manually (stdio MCP protocol)
cargo run -p mcp-server
```
