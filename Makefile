SERVER_BIN := mcp-kali
CLIENT_BIN := mcp-kali-bridge
CARGO := cargo
VERSION := $(shell awk -F '"' '/^version = / { print $$2; exit }' Cargo.toml)
MCP_KALI_HOME ?= $(HOME)/.mcp-kali
INSTALL_DIR ?= $(MCP_KALI_HOME)/bin
CONFIG_DIR ?= $(MCP_KALI_HOME)/etc
STATE_DIR ?= $(MCP_KALI_HOME)/var/jobs
PLUGIN_DIR := $(CONFIG_DIR)/plugins
CONFIG_FILE := $(CONFIG_DIR)/mcp-kali.conf
LOCAL_BIN_DIR ?= $(HOME)/.local/bin
COMPLETION_DIR := target/completions
SECURITY_DIR := target/security
SYSTEM_PREFIX ?= /usr/local
SYSTEM_BIN_DIR ?= $(SYSTEM_PREFIX)/bin
SYSTEM_CONFIG_DIR ?= /etc/mcp-kali
SYSTEM_STATE_DIR ?= /var/lib/mcp-kali/jobs
SYSTEMD_UNIT_DIR ?= /etc/systemd/system
MCP_KALI_USER ?=
MCP_KALI_GROUP ?= $(MCP_KALI_USER)
SYSTEM_CONFIG_FILE := $(SYSTEM_CONFIG_DIR)/mcp-kali.conf
SYSTEM_UNIT_FILE := $(SYSTEMD_UNIT_DIR)/mcp-kali.service

.PHONY: help fmt fmt-check check clippy test build release verify run-server run-client \
	completions install-local checksum security sbom clean \
	install install-system systemd-reload enable-system disable-system status-system logs-system

help:
	@echo "MCP Kali $(VERSION) development and release targets"
	@echo "  fmt           Format Rust sources"
	@echo "  fmt-check     Verify Rust formatting without changes"
	@echo "  check         Compile all targets and features"
	@echo "  clippy        Run strict Clippy checks"
	@echo "  test          Run the full test suite"
	@echo "  build         Build debug binaries"
	@echo "  release       Build size-optimized release binaries"
	@echo "  verify        Run fmt, check, clippy, test, and release"
	@echo "  run-server    Run a local development server"
	@echo "  run-client    Run the stdio MCP client"
	@echo "  completions   Generate completion scripts for both binaries"
	@echo "  install       Install locally as a user, or system-wide as root"
	@echo "  install-local Create a self-contained per-user installation under ~/.mcp-kali"
	@echo "  install-system Install binaries, data, config template, and systemd unit (root; existing service user required)"
	@echo "  systemd-reload Reload systemd unit files after install-system"
	@echo "  enable-system  Enable and start mcp-kali.service"
	@echo "  disable-system Disable and stop mcp-kali.service"
	@echo "  status-system  Show mcp-kali.service status"
	@echo "  logs-system    Follow mcp-kali.service journal logs"
	@echo "  checksum      Generate target/release/SHA256SUMS"
	@echo "  security      Run audit, dependency policy, and secret scan"
	@echo "  sbom          Generate a CycloneDX JSON SBOM (cargo-cyclonedx required)"
	@echo "  clean         Remove Cargo build artifacts"

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

check:
	$(CARGO) check --all-targets --all-features

clippy:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

test:
	$(CARGO) test --all-features

build:
	$(CARGO) build

release:
	$(CARGO) build --release

verify: fmt-check check clippy test release

run-server:
	$(CARGO) run --bin $(SERVER_BIN) -- --state-dir ./var/jobs --system-data-dir ./share/mcp-kali --config-dir ./etc

run-client:
	$(CARGO) run --bin $(CLIENT_BIN) -- --server http://127.0.0.1:5000

completions: release
	mkdir -p "$(COMPLETION_DIR)"
	target/release/$(SERVER_BIN) completions bash > "$(COMPLETION_DIR)/$(SERVER_BIN).bash"
	target/release/$(SERVER_BIN) completions zsh > "$(COMPLETION_DIR)/_$(SERVER_BIN)"
	target/release/$(SERVER_BIN) completions fish > "$(COMPLETION_DIR)/$(SERVER_BIN).fish"
	target/release/$(SERVER_BIN) completions powershell > "$(COMPLETION_DIR)/$(SERVER_BIN).ps1"
	target/release/$(SERVER_BIN) completions elvish > "$(COMPLETION_DIR)/$(SERVER_BIN).elv"
	target/release/$(CLIENT_BIN) completions bash > "$(COMPLETION_DIR)/$(CLIENT_BIN).bash"
	target/release/$(CLIENT_BIN) completions zsh > "$(COMPLETION_DIR)/_$(CLIENT_BIN)"
	target/release/$(CLIENT_BIN) completions fish > "$(COMPLETION_DIR)/$(CLIENT_BIN).fish"
	target/release/$(CLIENT_BIN) completions powershell > "$(COMPLETION_DIR)/$(CLIENT_BIN).ps1"
	target/release/$(CLIENT_BIN) completions elvish > "$(COMPLETION_DIR)/$(CLIENT_BIN).elv"

install-local: release
	@test "$$(id -u)" -ne 0 || { echo "install-local is for a non-root user; use make install MCP_KALI_USER=<authorized-user> as root" >&2; exit 2; }
	mkdir -p "$(INSTALL_DIR)"
	mkdir -p "$(PLUGIN_DIR)"
	mkdir -p "$(STATE_DIR)"
	mkdir -p "$(LOCAL_BIN_DIR)"
	@test -e "$(CONFIG_FILE)" || install -m 0644 "examples/mcp-kali.conf.example" "$(CONFIG_FILE)"
	install -m 0755 "target/release/$(SERVER_BIN)" "$(INSTALL_DIR)/$(SERVER_BIN)"
	install -m 0755 "target/release/$(CLIENT_BIN)" "$(INSTALL_DIR)/$(CLIENT_BIN)"
	@for binary in "$(SERVER_BIN)" "$(CLIENT_BIN)"; do \
		link="$(LOCAL_BIN_DIR)/$$binary"; \
		if [ -e "$$link" ] && [ ! -L "$$link" ]; then \
			echo "refusing to replace non-symlink $$link" >&2; exit 2; \
		fi; \
	done; \
	for binary in "$(SERVER_BIN)" "$(CLIENT_BIN)"; do \
		link="$(LOCAL_BIN_DIR)/$$binary"; \
		ln -sfn "$(abspath $(INSTALL_DIR))/$$binary" "$$link"; \
	done
	cp -R share/mcp-kali/plugins/. "$(PLUGIN_DIR)/"

install:
	@if [ "$$(id -u)" -eq 0 ]; then \
		$(MAKE) install-system MCP_KALI_USER="$(MCP_KALI_USER)" MCP_KALI_GROUP="$(MCP_KALI_GROUP)"; \
	else \
		$(MAKE) install-local; \
	fi

install-system: release
	@test "$$(id -u)" -eq 0 || { echo "install-system must run as root" >&2; exit 2; }
	@test -n "$(MCP_KALI_USER)" || { echo "MCP_KALI_USER is required for a system install" >&2; exit 2; }
	@case "$(MCP_KALI_USER):$(MCP_KALI_GROUP)" in (*[!A-Za-z0-9_.-]*|:) echo "MCP_KALI_USER and MCP_KALI_GROUP must be simple account names" >&2; exit 2;; esac
	@id -u "$(MCP_KALI_USER)" >/dev/null 2>&1 || { echo "service user $(MCP_KALI_USER) does not exist; create or select an authorized account" >&2; exit 2; }
	@getent group "$(MCP_KALI_GROUP)" >/dev/null 2>&1 || { echo "service group $(MCP_KALI_GROUP) does not exist" >&2; exit 2; }
	install -d -m 0755 "$(SYSTEM_BIN_DIR)" "$(SYSTEM_CONFIG_DIR)/plugins" "$(SYSTEMD_UNIT_DIR)"
	install -d -o "$(MCP_KALI_USER)" -g "$(MCP_KALI_GROUP)" -m 0700 "$(SYSTEM_STATE_DIR)"
	install -m 0755 "target/release/$(SERVER_BIN)" "$(SYSTEM_BIN_DIR)/$(SERVER_BIN)"
	install -m 0755 "target/release/$(CLIENT_BIN)" "$(SYSTEM_BIN_DIR)/$(CLIENT_BIN)"
	cp -R share/mcp-kali/plugins/. "$(SYSTEM_CONFIG_DIR)/plugins/"
	@test -e "$(SYSTEM_CONFIG_FILE)" || install -m 0644 "examples/mcp-kali.system.conf.example" "$(SYSTEM_CONFIG_FILE)"
	sed -e 's|@MCP_KALI_USER@|$(MCP_KALI_USER)|g' -e 's|@MCP_KALI_GROUP@|$(MCP_KALI_GROUP)|g' -e 's|@MCP_KALI_BIN@|$(SYSTEM_BIN_DIR)/$(SERVER_BIN)|g' "systemd/mcp-kali.service.in" > "$(SYSTEM_UNIT_FILE)"
	chmod 0644 "$(SYSTEM_UNIT_FILE)"
	@echo "Installed $(SYSTEM_UNIT_FILE). Run: make systemd-reload enable-system"

systemd-reload:
	systemctl daemon-reload

enable-system:
	systemctl enable --now mcp-kali.service

disable-system:
	systemctl disable --now mcp-kali.service

status-system:
	systemctl status mcp-kali.service

logs-system:
	journalctl -u mcp-kali.service -f

checksum: release
	cd target/release && shasum -a 256 "$(SERVER_BIN)" "$(CLIENT_BIN)" > SHA256SUMS

security: check clippy test
	cargo audit
	cargo deny check
	gitleaks detect --source . --redact

sbom:
	mkdir -p "$(SECURITY_DIR)"
	cargo cyclonedx --format json --override-filename "mcp-kali-$(VERSION).cdx"
	mv "mcp-kali-$(VERSION).cdx.json" "$(SECURITY_DIR)/mcp-kali-$(VERSION).cdx.json"

clean:
	$(CARGO) clean
