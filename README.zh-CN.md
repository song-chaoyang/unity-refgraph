# Unity RefGraph

**独立 Unity 项目资产关系与引用分析器 — 使用 Rust 构建。**

[![CI](https://github.com/song-chaoyang/unity-refgraph/actions/workflows/ci.yml/badge.svg)](https://github.com/song-chaoyang/unity-refgraph/actions/workflows/ci.yml)
[![Release](https://github.com/song-chaoyang/unity-refgraph/actions/workflows/release.yml/badge.svg)](https://github.com/song-chaoyang/unity-refgraph/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[English](README.md)

将 Unity 项目索引到 SQLite 数据库，提供快速的资产引用查询、场景/预制体层级浏览、以及 C# 脚本绑定分析。同时提供 CLI 命令行工具和带有交互式依赖图的可视化 Web UI。

## 功能特性

- **资产引用追踪** — 查找谁引用了某个材质、纹理、预制体或任何资产（通过 GUID）
- **场景/预制体层级** — 浏览 GameObject 树、组件绑定、Transform 父子关系
- **C# 脚本分析** — 通过 tree-sitter 提取类/方法/属性声明，将 MonoBehaviour 组件链接到其脚本类
- **虚拟文件系统（VFS）** — 以虚拟路径查询资产，支持 glob 匹配、grep 搜索和树形浏览
- **交互式 Web UI** — 使用 vis.js 的可视化依赖图，点击探索导航
- **增量同步** — 基于内容哈希仅重新索引变更的文件
- **单一二进制** — 无运行时依赖，发布为单个静态二进制文件

## 快速上手

```bash
# 构建
cargo build --release

# 为 Unity 项目建立索引
unity-refgraph index build /path/to/your/unity/project

# 查询引用 — 谁引用了这个材质？
unity-refgraph refs Assets/Materials/Enemy.mat -p /path/to/project

# 这个场景引用了什么？
unity-refgraph refs Assets/Scenes/Game.unity -p /path/to/project --direction out

# 浏览 VFS
unity-refgraph ls Assets -p /path/to/project --depth 2

# 启动 Web UI
unity-refgraph serve /path/to/project --port 8089
```

## 命令行参考

```
unity-refgraph <COMMAND>

命令:
  index   索引管理 (build 构建 / sync 同步 / status 状态)
  refs    查询某个 VFS 路径的引用关系
  ls      列出某路径下的 VFS 条目
  glob    按通配符匹配 VFS 条目
  grep    在 VFS 条目内容中搜索
  read    读取 VFS 条目内容
  serve   启动 Web 服务器（交互式图谱 UI）
  mcp     启动 MCP 服务器（JSON-RPC over stdio）
  help    打印帮助信息
```

## REST API

| 接口 | 说明 |
|------|------|
| `GET /api/status` | 索引统计信息 |
| `GET /api/ls?path=...&depth=N` | 列出 VFS 子条目 |
| `GET /api/refs?path=...&direction=in` | 引用查询 |
| `GET /api/glob?pattern=...` | 通配符搜索 |
| `GET /api/grep?pattern=...` | 内容搜索 |
| `GET /api/read?path=...` | 读取 VFS 条目 |
| `GET /api/graph?path=...` | 可视化子图数据 |

## 架构

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

### VFS 边类型

| 边类型 | 含义 |
|--------|------|
| `child_of` | 目录→文件，父节点→子节点 |
| `defined_in` | 节点→定义它的文件 |
| `depends_on` | 文件→被引用文件（GUID） |
| `instance_of` | 预制体实例→源预制体 |
| `binds_to` | 组件→C# 脚本类 |

## 支持的文件类型

| 扩展名 | 类型 | 解析器 |
|--------|------|--------|
| `.unity`, `.scene` | 场景 | Unity YAML |
| `.prefab` | 预制体 | Unity YAML |
| `.mat` | 材质 | Unity YAML |
| `.asset` | 资产 | Unity YAML |
| `.cs` | C# 脚本 | tree-sitter |
| `.shader` | 着色器 | 文本 |
| `.fbx`, `.png`, ... | 二进制 | 仅哈希 |

## 技术栈

| 用途 | Crate |
|---------|-------|
| SQLite | `rusqlite` (bundled) |
| C# 解析 | `tree-sitter` + `tree-sitter-c-sharp` |
| YAML 解析 | `serde_yaml` |
| 命令行 | `clap` |
| Web 服务器 | `axum` + `tokio` |
| 并行 | `rayon` |
| 图可视化 | vis.js |

## MCP 服务器

`unity-refgraph` 可作为 MCP（模型上下文协议）服务器运行，让 MCP 兼容的工具直接查询 Unity 项目的资产引用关系。

### 启动 MCP 服务器

```bash
unity-refgraph mcp
```

服务器从 stdin 读取 JSON-RPC 请求，将响应写入 stdout。

### MCP 客户端配置

添加到 MCP 客户端配置文件中（如 macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`，Windows: `%APPDATA%\Claude\claude_desktop_config.json`）：

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

### 可用 MCP 工具

| 工具 | 说明 |
|------|------|
| `index_build` | 为 Unity 项目构建索引 |
| `index_status` | 显示索引统计 |
| `refs` | 查询引用图（入/出） |
| `ls` | 列出 VFS 条目 |
| `glob` | 通配符搜索 |
| `grep` | 内容搜索 |
| `read` | 读取 VFS 条目内容 |

### 通过 MCP 查询引用示例

MCP 客户端可调用：
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

响应：
```
Assets/Materials/Enemy.mat (2 referenced by)
────────────────────────────────────────────────
  ← Assets/Scenes/Game.unity [scene] via depends_on (guid-file)
  ← Assets/Scenes/Game.unity [scene] via instance_of

2 reference(s) total
```

## 开发

```bash
cargo test      # 运行全部 32 个测试
cargo clippy    # 代码检查
cargo fmt       # 格式化
```

## 下载

预编译二进制文件可在 [Releases](https://github.com/song-chaoyang/unity-refgraph/releases) 页面下载：

- macOS (arm64, x86_64)
- Linux (x86_64)
- Windows (x86_64)

## 许可证

[MIT](LICENSE)
