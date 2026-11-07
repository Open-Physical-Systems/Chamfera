# The MCP server

`chamfera-cli mcp` lets AI agents (Claude Code and other MCP clients) drive Chamfera: list and run commands, build parts in batches, check the design and look at a rendered snapshot. It speaks the [Model Context Protocol](https://modelcontextprotocol.io) over stdio: newline-delimited JSON-RPC 2.0, one message per line, logs on stderr. The code is `crates/mcp` (layer 6); the modelling guide agents get from it is `skills/chamfera-modeling/SKILL.md`.

## Adding it to Claude Code

```sh
cargo install --path apps/chamfera-cli          # or use a built chamfera-cli
claude mcp add chamfera -- chamfera-cli mcp --allow-path ~/cad
```

`--allow-path DIR` (repeatable) names a folder the server may read and write designs and meshes in. Without it the server can only use its scratch folder. Other clients: run `chamfera-cli mcp [--allow-path DIR]...` as a stdio server.

## Protocol versions
The server follows revision 2026-07-28, where every request carries `io.modelcontextprotocol/protocolVersion` and `io.modelcontextprotocol/clientCapabilities` in `params._meta` and there is no handshake (`server/discover` describes the server). It also accepts clients of 2025-11-25, 2025-06-18, 2025-03-26 and 2024-11-05, which open with `initialize`; after that, requests without `_meta` are served under the agreed legacy revision. A request with neither gets an error naming both ways.

The process holds one open design, like an app window: all tools work on it, from any client on the connection. Restarting the server starts an empty design (save first).

## Tools
| Tool | Arguments | Does | Annotations |
|---|---|---|---|
| `commands` | `prefix` (optional, e.g. `"sketch."`) | command ids, labels, parameter docs, `mutates` | read-only |
| `run` | `command`, `params` (object, default `{}`) | runs one command; the reply as JSON text, or the error as a tool error | destructive |
| `batch` | `steps`: up to 1000 `{command, params}` | runs in order, stops at the first error; every reply, and the error with its step | destructive |
| `snapshot` | `view` (iso, front, back, left, right, top, bottom), `projection`, `theme` (light, dark), `width`, `height` (16 to 2048, default 800 x 600) | a PNG image of bodies and sketches, fitted to the content; the session's camera is put back | read-only |
| `design_summary` | none | parameters; sketches with counts, DOF, status, profile areas; bodies with volume and bounds; the allowed folders | read-only |
| `skill_reference` | none | the modelling guide | read-only |

`run` and `batch` are marked destructive because commands can delete geometry, replace the design or overwrite a file; none of the tools reach the network (`openWorldHint: false`). Bad arguments and failing commands come back as tool results with `isError: true` and a message saying what was wrong; an unknown tool or a malformed request is a JSON-RPC error. A `batch` checks the shape of every step before running any. Without a graphics adapter `snapshot` returns an error saying so (a software Vulkan driver such as lavapipe, in `mesa-vulkan-drivers`, is enough).

## Path rules
The file commands (`design.open`, `design.save`, `body.import_stl`) may only use the scratch folder and the `--allow-path` folders. Before such a command runs, the server resolves its `path`:
1. An empty path, one longer than 4096 bytes, or one containing NUL is refused.
2. Any `..` component is refused, even one that would stay inside.
3. A relative path is taken relative to the scratch folder.
4. The path is canonicalized, following every symlink: for reading the file itself, for writing the file if it exists (a dangling symlink is refused) or else its folder. The result must lie inside an allowed folder, compared by whole path components.
5. The command runs with the canonical path.

Anything else is refused with a message listing the allowed folders. A command whose parameters name a path but which is not in the guarded list is refused outright, so a new file command stays closed until it is added (`PATH_COMMANDS` in `crates/mcp/src/paths.rs`; a test fails until it is).

The scratch folder is `chamfera-mcp-<pid>` in the system temp folder, private to the user (mode 0700 on Unix), and is kept when the server exits so saved files survive. The server prints the folders on stderr at start and in its instructions; `design_summary` lists them too.

Limit: a symlink swapped in between the check and the command's own open is not caught. The allowed folders are the user's own, so that needs someone who can already write there.
