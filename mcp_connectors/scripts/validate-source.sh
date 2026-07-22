#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd -P)
PYTHON_BIN=${PYTHON_BIN:-python3}
PLUGIN_CREATOR_DIR=${PLUGIN_CREATOR_DIR:-$HOME/.codex/skills/.system/plugin-creator}
SKILL_CREATOR_DIR=${SKILL_CREATOR_DIR:-$HOME/.codex/skills/.system/skill-creator}
PLUGIN_ROOT="$REPO_ROOT/mcp_connectors/codex/plugins/mcp-kali"
SKILL_ROOT="$PLUGIN_ROOT/skills/use-mcp-kali"

expected_version=$(awk -F '"' '/^version = / { print $2; exit }' "$REPO_ROOT/Cargo.toml")
plugin_version=$(sed -n 's/.*"version": "\([^"]*\)".*/\1/p' "$PLUGIN_ROOT/.codex-plugin/plugin.json" | head -n 1)
manifest_version=$(sed -n 's/.*"version": "\([^"]*\)".*/\1/p' "$REPO_ROOT/mcp_connectors/claude-desktop/mcp-kali/manifest.json" | head -n 1)

if [ "$plugin_version" != "$expected_version" ] || [ "$manifest_version" != "$expected_version" ]; then
  echo "connector versions must match Cargo.toml version $expected_version" >&2
  exit 2
fi

"$PYTHON_BIN" -m json.tool "$PLUGIN_ROOT/.codex-plugin/plugin.json" >/dev/null
"$PYTHON_BIN" -m json.tool "$PLUGIN_ROOT/.mcp.json" >/dev/null
"$PYTHON_BIN" -m json.tool "$REPO_ROOT/mcp_connectors/codex/.agents/plugins/marketplace.json" >/dev/null
"$PYTHON_BIN" -m json.tool "$REPO_ROOT/mcp_connectors/claude-desktop/mcp-kali/manifest.json" >/dev/null

if grep -R '\[TODO:' "$PLUGIN_ROOT" >/dev/null 2>&1; then
  echo "Codex connector contains an unfinished TODO placeholder" >&2
  exit 2
fi

if "$PYTHON_BIN" -c 'import yaml' >/dev/null 2>&1 \
  && [ -f "$PLUGIN_CREATOR_DIR/scripts/validate_plugin.py" ] \
  && [ -f "$SKILL_CREATOR_DIR/scripts/quick_validate.py" ]; then
  "$PYTHON_BIN" "$PLUGIN_CREATOR_DIR/scripts/validate_plugin.py" "$PLUGIN_ROOT"
  "$PYTHON_BIN" "$SKILL_CREATOR_DIR/scripts/quick_validate.py" "$SKILL_ROOT"
else
  echo "PyYAML or Codex validator scripts not found; skipped official plugin and skill validation" >&2
fi

if command -v mcpb >/dev/null 2>&1; then
  mcpb validate "$REPO_ROOT/mcp_connectors/claude-desktop/mcp-kali/manifest.json"
else
  echo "mcpb not found; skipped MCPB schema validation" >&2
fi

echo "Connector sources are valid for version $expected_version"
