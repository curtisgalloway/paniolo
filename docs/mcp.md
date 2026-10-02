<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# MCP server (experimental)

`paniolo mcp` serves paniolo's runtime commands as
[Model Context Protocol](https://modelcontextprotocol.io) tools over stdio. An
agent harness that speaks MCP can then drive a target without a shell. For
example, a screenshot comes back as an image in the tool result instead of a
file the agent has to open.

It is a spike: two tools today, more to follow. The design and the plan for
the rest are in
[`notes/mcp-server.md`](https://github.com/curtisgalloway/paniolo/blob/main/notes/mcp-server.md).

## Set it up

Run it on the machine that holds your lab file, usually your own machine,
not a control host. Tools that touch a remote target dispatch over SSH
exactly as the CLI does.

Claude Code:

```bash
claude mcp add paniolo -- paniolo mcp
# a lab file other than the default:
claude mcp add paniolo -- paniolo --lab ~/lab.toml mcp
```

Any other harness: configure a stdio server whose command is `paniolo` with
arguments `["mcp"]`. In the common JSON form:

```json
{ "mcpServers": { "paniolo": { "command": "paniolo", "args": ["mcp"] } } }
```

The harness starts the server and keeps it running for the session. The
server holds no state of its own, so restarting it loses nothing.

## Tools

| Tool | Arguments | Returns |
|---|---|---|
| `target_list` | none | JSON: each target's name, description, hosts, and channels (kind, name, host). |
| `video_shot` | `target` (required); `changed_since` (a hash); `stable` (bool); `timeout_ms` (0–120000, default 2000) | The screen as a PNG image, plus a text line `signal=<s>  hash=<h>`. |

`video_shot` needs the target's video daemon to be running already
(`paniolo video watch <target>`). To wait for the screen to change after an
action, pass the previous result's hash as `changed_since`, the same loop the
`kvm-puppeting` skill teaches with `video shot --changed-since`.

A tool that runs and fails returns a result with `isError: true`. Its text
gives paniolo's exit code, the error message, and the one-line JSON error
object described in [Exit status and errors](errors.md). A malformed call
(unknown tool, missing `target`) is a JSON-RPC error instead.

## How it behaves

- **stdout is the protocol.** Each tool runs the matching `paniolo` command
  as a child process with stdin closed and stdout captured, so nothing a
  command prints can corrupt the stream. Diagnostics go to stderr, which
  harnesses usually log.
- **One call at a time.** A `video_shot` waiting on `changed_since` blocks
  other calls until it returns, for at most `timeout_ms`.
- **Dead links fail, not hang.** A remote call over an SSH connection that has
  gone silently dead fails as `unreachable` within about 15 seconds; see
  [Distributed control](distributed-control.md).
- MCP protocol revisions 2024-11-05 through 2025-11-25 are accepted.
