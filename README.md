# Unity RefGraph

**Standalone Unity project asset relationship and reference analyzer — built in Rust.**

[![CI](https://github.com/song-chaoyang/unity-refgraph/actions/workflows/ci.yml/badge.svg)](https://github.com/song-chaoyang/unity-refgraph/actions/workflows/ci.yml)
[![Release](https://github.com/song-chaoyang/unity-refgraph/actions/workflows/release.yml/badge.svg)](https://github.com/song-chaoyang/unity-refgraph/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[中文文档](README.zh-CN.md)

Indexes Unity projects into a SQLite database and provides fast queries for asset references, scene/prefab hierarchy, and C# script bindings. Includes both a CLI tool and a Web UI with an interactive dependency graph.

## Features

- **Asset Reference Tracking** — Find who references a material, texture, prefab, or any asset (by GUID)
- **Scene/Prefab Hierarchy** — Browse GameObject trees, component bindings, Transform parent/child relationships
- **C# Script Analysis** — Extract class/method/property declarations via tree-sitter, link MonoBehaviour components to their script classes
- **Virtual File System (VFS)** — Query assets as virtual paths with glob, grep, and tree listing
- **Interactive Web UI** — Visual dependency graph with vis.js, click-to-explore navigation
- **Incremental Sync** — Re-index only changed files based on content hash
- **Single Binary** — No runtime dependencies, ships as one static binary

## Quick Start

```bash
# Build
cargo build --release

# Index a Unity project
unity-refgraph index build /path/to/your/unity/project

# Query references — who uses this material?
unity-refgraph refs Assets/Materials/Enemy.mat -p /path/to/project

# What does this scene reference?
unity-refgraph refs Assets/Scenes/Game.unity -p /path/to/project --direction out

# Browse the VFS
unity-refgraph ls Assets -p /path/to/project --depth 2

# Launch Web UI
unity-refgraph serve /path/to/project --port 8089
```

## CLI Reference

```
unity-refgraph <COMMAND>

Commands:
  index   Index management (build / sync / status)
  refs    Query references for a VFS path
  ls      List VFS entries under a path
  glob    Find VFS entries matching a glob pattern
  grep    Search content within VFS entries
  read    Read content of a VFS entry
  serve   Start web server with interactive graph UI
  mcp     Start MCP server (JSON-RPC over stdio)
  help    Print this message
```

## REST API

| Endpoint | Description |
|----------|-------------|
| `GET /api/status` | Index statistics |
| `GET /api/ls?path=...&depth=N` | List VFS children |
| `GET /api/refs?path=...&direction=in` | Reference query |
| `GET /api/glob?pattern=...` | Glob search |
| `GET /api/grep?pattern=...` | Content search |
| `GET /api/read?path=...` | Read VFS entry |
| `GET /api/graph?path=...` | Subgraph for visualization |

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                 unity-refgraph (Rust binary)                 │
│                                                               │
│  ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐ │
│  │ Discovery │→ │ Extract  │→ │ Resolve  │→ │Materialize│ │
│  │ (walkdir) │   │(yaml+TS) │   │ (graph)  │   │  (VFS)   │ │
│  └──────────┘   └──────────┘   └──────────┘   └──────────┘  │
│        ↓              ↓             ↓              ↓          │
│  ┌──────────────────────────────────────────────────────┐    │
│  │              SQLite Index (rusqlite)                 │    │
│  └──────────────────────────────────────────────────────┘    │
│                          ↑                                   │
│  ┌────────────────┐    ┌────────────────┐                    │
│  │  CLI (clap)    │    │  Web UI (axum) │                    │
│  └────────────────┘    └────────────────┘                    │
└─────────────────────────────────────────────────────────────┘
```

### VFS Edge Types

| Edge Kind | Meaning |
|-----------|---------|
| `child_of` | Directory → file, parent node → child node |
| `defined_in` | Node → file where it's defined |
| `depends_on` | File → referenced file (GUID-based) |
| `instance_of` | Prefab instance → source prefab |
| `binds_to` | Component → C# script class |

## Supported File Types

| Extension | Kind | Parser |
|-----------|------|--------|
| `.unity`, `.scene` | scene | Unity YAML |
| `.prefab` | prefab | Unity YAML |
| `.mat` | material | Unity YAML |
| `.asset` | asset | Unity YAML |
| `.cs` | C# script | tree-sitter |
| `.shader` | shader | text |
| `.fbx`, `.png`, ... | binary | hash only |

## Tech Stack

| Concern | Crate |
|---------|-------|
| SQLite | `rusqlite` (bundled) |
| C# parsing | `tree-sitter` + `tree-sitter-c-sharp` |
| YAML parsing | `serde_yaml` |
| CLI | `clap` |
| Web server | `axum` + `tokio` |
| Parallelism | `rayon` |
| Graph visualization | vis.js |

## MCP Server

`unity-refgraph` can run as an MCP (Model Context Protocol) server, allowing MCP-compatible tools to query your Unity project's asset references directly.

### Start MCP Server

```bash
unity-refgraph mcp
```

The server reads JSON-RPC requests from stdin and writes responses to stdout.

### MCP Client Configuration

Add to your MCP client configuration file (e.g. `claude_desktop_config.json` on macOS: `~/Library/Application Support/Claude/`, on Windows: `%APPDATA%\Claude\`):

```json
{
  "mcpServers": {
    "unity-refgraph": {
      "command": "/path/to/unity-refgraph",
      "args": ["mcp"]
    }
  }
}
```

### Available MCP Tools

| Tool | Description |
|------|-------------|
| `index_build` | Build index for a Unity project |
| `index_status` | Show index statistics |
| `refs` | Query reference graph (in/out) |
| `ls` | List VFS entries |
| `glob` | Glob pattern search |
| `grep` | Content search |
| `read` | Read VFS entry content |

### Example: Query References via MCP

MCP client can call:
```json
{
  "name": "refs",
  "arguments": {
    "project_path": "/path/to/unity/project",
    "path": "Assets/Materials/Enemy.mat",
    "direction": "in"
  }
}
```

Response:
```
Assets/Materials/Enemy.mat (2 referenced by)
────────────────────────────────────────────────
  ← Assets/Scenes/Game.unity [scene] via depends_on (guid-file)
  ← Assets/Scenes/Game.unity [scene] via instance_of

2 reference(s) total
```

## Development

```bash
cargo test      # Run all 32 tests
cargo clippy    # Lint
cargo fmt       # Format
```

## Download

Pre-built binaries are available on the [Releases](https://github.com/song-chaoyang/unity-refgraph/releases) page for:

- macOS (arm64, x86_64)
- Linux (x86_64)
- Windows (x86_64)

## License

[MIT](LICENSE)
