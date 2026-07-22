#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd -P)
SOURCE_ROOT="$REPO_ROOT/mcp_connectors/codex"
OUTPUT_ROOT="$REPO_ROOT/target/mcp_connectors/codex"
SOURCE_MCP="$SOURCE_ROOT/plugins/mcp-kali/.mcp.json"
OUTPUT_MCP="$OUTPUT_ROOT/plugins/mcp-kali/.mcp.json"

bridge_path=${MCP_KALI_BRIDGE_PATH:-}
if [ -z "$bridge_path" ]; then
  bridge_path=$(command -v mcp-kali-bridge || true)
fi
if [ -z "$bridge_path" ] || [ ! -x "$bridge_path" ]; then
  echo "mcp-kali-bridge was not found; run 'make client-install' or set MCP_KALI_BRIDGE_PATH" >&2
  exit 2
fi

bridge_dir=$(CDPATH= cd -- "$(dirname -- "$bridge_path")" && pwd -P)
bridge_path="$bridge_dir/$(basename -- "$bridge_path")"
case "$bridge_path" in
  *'"'*|*'\'*|*'|'*)
    echo "bridge path contains characters unsupported by the connector renderer" >&2
    exit 2
    ;;
esac

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
  echo "Codex connector builds are supported only on Apple Silicon macOS" >&2
  exit 2
fi
if ! file "$bridge_path" | grep -q 'Mach-O 64-bit executable arm64'; then
  echo "bridge is not an Apple Silicon Mach-O executable: $bridge_path" >&2
  exit 2
fi

expected_version=$(awk -F '"' '/^version = / { print $2; exit }' "$REPO_ROOT/Cargo.toml")
actual_version=$("$bridge_path" --version | awk '{ print $2 }')
if [ "$actual_version" != "$expected_version" ]; then
  echo "bridge version $actual_version does not match Cargo.toml version $expected_version" >&2
  exit 2
fi

source_version=$(sed -n 's/.*"version": "\([^"]*\)".*/\1/p' "$SOURCE_ROOT/plugins/mcp-kali/.codex-plugin/plugin.json" | head -n 1)
if [ "$source_version" != "$expected_version" ]; then
  echo "Codex plugin version $source_version does not match Cargo.toml version $expected_version" >&2
  exit 2
fi

mkdir -p "$REPO_ROOT/target/mcp_connectors"
rm -rf "$OUTPUT_ROOT"
cp -R "$SOURCE_ROOT" "$OUTPUT_ROOT"
escaped_bridge=$(printf '%s' "$bridge_path" | sed 's/[&|]/\\&/g')
sed "s|\"command\": \"mcp-kali-bridge\"|\"command\": \"$escaped_bridge\"|" "$SOURCE_MCP" > "$OUTPUT_MCP"

echo "Prepared Codex connector marketplace: $OUTPUT_ROOT"
