# Makefile for tailcat-node
# A small cross-platform daemon around Tailcat.

# --- Configuration ----------------------------------------------------------
CARGO      ?= cargo
BINARY     ?= tailcat-node
TARGET_DIR ?= target
RELEASE_BIN = $(TARGET_DIR)/release/$(BINARY)
DEBUG_BIN   = $(TARGET_DIR)/debug/$(BINARY)

# Default to whatever cargo picks up (debug). Override with `make PROFILE=release`.
PROFILE ?= debug

# Optional install prefix (used by `make install`).
PREFIX ?= /usr/local

# --- Phony targets ---------------------------------------------------------
.PHONY: all build release run test check clippy fmt lint clean doc install uninstall help

all: build

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

# --- Build -----------------------------------------------------------------
build: ## Build the binary in debug mode
	$(CARGO) build

release: ## Build the binary in release mode
	$(CARGO) build --release

run: ## Run the daemon (debug build) with any args via ARGS="..."
	$(CARGO) run -- $(ARGS)

run-release: ## Run the daemon (release build) with any args via ARGS="..."
	$(CARGO) run --release -- $(ARGS)

# --- Quality ---------------------------------------------------------------
check: ## Type-check without producing a binary
	$(CARGO) check --all-targets

clippy: ## Run clippy lints
	$(CARGO) clippy --all-targets -- -D warnings

fmt: ## Format the source tree
	$(CARGO) fmt

fmt-check: ## Verify formatting without changing files
	$(CARGO) fmt -- --check

lint: clippy fmt-check ## Run clippy + format check

# --- Testing ---------------------------------------------------------------
test: ## Run the test suite
	$(CARGO) test --all-targets

test-doc: ## Run doctests
	$(CARGO) test --doc

# --- Docs ------------------------------------------------------------------
doc: ## Generate and open the API documentation
	$(CARGO) doc --no-deps --open

# --- Install / Uninstall ---------------------------------------------------
install: release ## Install the release binary to PREFIX (default /usr/local)
	@mkdir -p $(DESTDIR)$(PREFIX)/bin
	@install -m 0755 $(RELEASE_BIN) $(DESTDIR)$(PREFIX)/bin/$(BINARY)
	@echo "Installed $(BINARY) -> $(DESTDIR)$(PREFIX)/bin/$(BINARY)"

uninstall: ## Remove the installed binary
	@rm -f $(DESTDIR)$(PREFIX)/bin/$(BINARY)
	@echo "Removed $(BINARY) from $(DESTDIR)$(PREFIX)/bin"

# --- Cleanup ---------------------------------------------------------------
clean: ## Remove the target directory
	$(CARGO) clean
