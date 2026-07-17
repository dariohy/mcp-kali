# Changelog

## Unreleased

### Added

- Rust API server and stdio MCP bridge in one `mcp-kali` binary.
- Durable asynchronous jobs with bounded concurrency, timeout, cancellation, restart recovery, paged output, HTTPS completion webhooks, and a built-in job monitor.
- Scanner argument validation and structured process execution without a shell.

### Changed

- Tool calls now return HTTP `202 Accepted` with a job record instead of holding the request open until a scanner exits.
- Generic command execution uses an argument vector. The compatibility command-string endpoint tokenizes input but does not interpret shell operators.

