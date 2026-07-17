# Rust port and asynchronous job design

The Python implementation ties every Flask request and MCP tool call to
`Popen.wait()`. A scan therefore occupies the request for up to five minutes,
buffers all output in memory, and encourages the model to poll until completion.
It also executes generic strings with `shell=True`, marks timed-out commands as
successful when they produced any output, uses one shared Metasploit resource
file, and provides no durable job identity or cancellation API.

The Rust implementation separates submission from execution:

```text
MCP tool -> HTTP 202 + job ID -> durable queue -> bounded workers -> output files
                                      |                              |
                                      +-> monitor/API <- final state +-> webhook
```

## Run it

```bash
cargo build --release
./target/release/mcp-kali serve \
  --bind 127.0.0.1:5000 \
  --state-dir ./var/jobs \
  --max-concurrency 2 \
  --default-timeout 1800
```

Open `http://127.0.0.1:5000/` for the job monitor. Run the MCP bridge locally:

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

## API workflow

Submit a scanner job:

```bash
curl -sS http://127.0.0.1:5000/api/tools/nmap \
  -H 'content-type: application/json' \
  -d '{"target":"127.0.0.1","scan_type":"-sV","timeout_seconds":600}'
```

Use the returned `id` with:

```text
GET  /api/jobs
GET  /api/jobs/{id}
GET  /api/jobs/{id}/output?stream=stdout&offset=0&limit=65536
POST /api/jobs/{id}/cancel
```

Add `"webhook_url":"https://listener.example/jobs"` to a submission to receive
the public job record once it reaches a terminal state. Delivery is best-effort
with a ten-second HTTP timeout. A production follow-up should add signed webhook
payloads and retry/backoff with a dead-letter view.

## Persistence and safety

Each job has a private directory containing `job.json`, `command.json`,
`stdout.log`, and `stderr.log`. On Unix, metadata and execution specs are mode
`600`, and directories are mode `700`. Execution arguments are omitted from API
and webhook serialization because they may contain passwords. Queued jobs resume
after restart; jobs that were running are marked `interrupted`, since adopting an
arbitrary orphan process safely requires a dedicated supervisor protocol.

Scanner processes receive an executable plus explicit arguments and never pass
through a shell. The old `/api/command` shape remains as a transition endpoint,
but shell operators are not interpreted. Bind to loopback and use an SSH tunnel;
there is intentionally no claim that an unauthenticated remote command runner is
safe to expose on a network.

## Migration gaps

- Debian packaging and systemd units still launch the Python scripts and need a
  follow-up package transition once binary paths and state ownership are agreed.
- The hand-written stdio MCP transport intentionally implements only the core
  lifecycle and tools methods used here. Moving to the official Rust SDK is a
  sensible follow-up after choosing a pinned protocol/SDK version.
- Webhook signing/retries, job retention, and per-user authorization should be
  completed before treating this as a multi-user service.
