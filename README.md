# MCP Kali — Rust asynchronous port

This is a Rust reimplementation of the Python MCP Kali Server located in
[`mcp-kali-server/`](mcp-kali-server/). The two implementations are deliberately
separate: the nested directory remains the original upstream checkout, while the
workspace root contains the new Rust package.

The Rust service returns a job ID immediately instead of holding an MCP request
open until a scanner exits. It provides a durable queue, bounded workers,
cancellation, timeouts, restart recovery, paged output, completion webhooks, and
a small browser-based job monitor.

## Build and run

```bash
cargo build --release

./target/release/mcp-kali serve \
  --bind 127.0.0.1:5000 \
  --state-dir ./var/jobs \
  --max-concurrency 2 \
  --default-timeout 1800
```

Open `http://127.0.0.1:5000/` to view the monitor. Start the local stdio MCP
bridge with:

```bash
./target/release/mcp-kali mcp --server http://127.0.0.1:5000
```

Example MCP configuration:

```json
{
  "mcpServers": {
    "mcp-kali": {
      "command": "/absolute/path/to/mcp-kali",
      "args": ["mcp", "--server", "http://127.0.0.1:5000"]
    }
  }
}
```

See [RUST_PORT.md](RUST_PORT.md) for the API workflow, architecture, security
decisions, and remaining production migration work.

## Repository layout

```text
.
├── Cargo.toml             Rust package
├── src/                   Rust scheduler, API, MCP bridge, and tool modules
├── RUST_PORT.md           Architecture and migration notes
└── mcp-kali-server/       Original Python/upstream Git checkout
```

