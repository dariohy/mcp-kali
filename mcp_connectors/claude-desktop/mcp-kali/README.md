# MCP Kali for Claude Desktop

This source template is staged into an Apple Silicon MCP Bundle by
`mcp_connectors/scripts/build-claude-desktop.sh`. The generated bundle contains
the locally compiled `mcp-kali-bridge`; the binary is never stored in this
source directory.

The bridge connects to an existing MCP Kali server. Keep the default loopback
URL when using an SSH tunnel, or configure an authenticated HTTPS origin.
