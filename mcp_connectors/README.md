# MCP connector packages

This directory contains source-only packaging for Codex and Claude Desktop on
Apple Silicon macOS. Compiled binaries and generated connector artifacts belong
under ignored `target/mcp_connectors/`; do not commit them.

## Prerequisites

Build and install the local stdio bridge on the Mac that runs the MCP host:

```bash
make client-install
```

The builders use `mcp-kali-bridge` from `PATH`. Set
`MCP_KALI_BRIDGE_PATH=/absolute/path/to/mcp-kali-bridge` when the executable is
elsewhere. They reject non-Apple-Silicon binaries and versions that differ from
`Cargo.toml`.

## Codex

Prepare a local marketplace whose MCP configuration contains the bridge's
absolute path:

```bash
make connector-codex
```

The result is `target/mcp_connectors/codex`. Add that directory as a local
Codex marketplace, install the `mcp-kali` plugin, and start a new task:

```bash
codex plugin marketplace add "$PWD/target/mcp_connectors/codex"
```

The plugin bundles the `use-mcp-kali` skill and uses the bridge's normal
config-file or environment precedence for the server URL.

## Claude Desktop

Install the MCPB CLI once:

```bash
npm install -g @anthropic-ai/mcpb
```

Then build the bundle:

```bash
make connector-claude-desktop
```

The resulting `target/mcp_connectors/mcp-kali-<version>-aarch64-apple-darwin.mcpb`
can be dragged onto Claude Desktop. Its setup form defaults to the loopback MCP
Kali server and requires an explicit opt-in for remote cleartext HTTP.

## Validation

```bash
make connectors-check
```

This always checks JSON syntax, version synchronization, and unfinished
placeholders. It also runs the official Codex validators when their scripts and
PyYAML are available, and validates the MCPB manifest when `mcpb` is installed.
