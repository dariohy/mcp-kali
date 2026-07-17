BIN := mcp-kali
CARGO := cargo
INSTALL_DIR ?= $(HOME)/.local/bin

.PHONY: help fmt check clippy test build release run install-local clean

help:
	@echo "fmt check clippy test build release run install-local clean"

fmt:
	$(CARGO) fmt --all

check:
	$(CARGO) check

clippy:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

test:
	$(CARGO) test

build:
	$(CARGO) build

release:
	$(CARGO) build --release

run:
	$(CARGO) run -- serve --state-dir ./var/jobs

install-local: release
	mkdir -p "$(INSTALL_DIR)"
	cp "target/release/$(BIN)" "$(INSTALL_DIR)/$(BIN)"

clean:
	$(CARGO) clean

