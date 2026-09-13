# ==============================================================================
# llmman - Production OCI MicroVM & Workload Engine
# ==============================================================================

SHELL := /usr/bin/env bash
.DEFAULT_GOAL := help

# Colors
CYAN    := \033[36m
GREEN   := \033[32m
YELLOW  := \033[33m
BOLD    := \033[1m
RESET   := \033[0m

CARGO ?= cargo
MIX   ?= mix

.PHONY: help all build release build-all build-init build-bench \
        test test-lib test-bin test-dashboard test-all \
        check lint clippy fmt fmt-check ci \
        dashboard-setup dashboard-dev dashboard-server dashboard-stop dashboard-fmt \
        vm-ps vm-metering vm-gc clean clean-all

# ------------------------------------------------------------------------------
# Help & Information
# ------------------------------------------------------------------------------

##@ General
help: ## Display this help message with available targets
	@awk 'BEGIN {FS = ":.*##"; printf "\n$(BOLD)$(CYAN)llmman$(RESET) - Developer Makefile\n$(YELLOW)Usage:$(RESET) make $(GREEN)<target>$(RESET)\n"} /^[a-zA-Z0-9_-]+:.*?##/ { printf "  $(GREEN)%-20s$(RESET) %s\n", $$1, $$2 } /^##@/ { printf "\n$(BOLD)%s$(RESET)\n", substr($$0, 5) } ' $(MAKEFILE_LIST)
	@echo ""

all: build

# ------------------------------------------------------------------------------
# Build Targets
# ------------------------------------------------------------------------------

##@ Build Targets
build: ## Compile debug binaries (default: llmman)
	@echo -e "$(CYAN)--> Building llmman debug binary...$(RESET)"
	$(CARGO) build

release: ## Compile optimized production binaries
	@echo -e "$(CYAN)--> Building optimized release binaries...$(RESET)"
	$(CARGO) build --release

build-all: ## Compile all binaries (llmman, init, llmman-bench)
	@echo -e "$(CYAN)--> Building all repository binaries...$(RESET)"
	$(CARGO) build --bins

build-init: ## Compile guest PID 1 init supervisor binary
	@echo -e "$(CYAN)--> Building guest init supervisor...$(RESET)"
	$(CARGO) build --bin init

build-bench: ## Compile workload benchmarking utility
	@echo -e "$(CYAN)--> Building llmman-bench utility...$(RESET)"
	$(CARGO) build --bin llmman-bench

# ------------------------------------------------------------------------------
# Test & Quality Targets
# ------------------------------------------------------------------------------

##@ Test & Code Quality
test: ## Run Rust unit test suite
	@echo -e "$(CYAN)--> Running Rust test suite...$(RESET)"
	$(CARGO) test --lib

test-all: test test-dashboard ## Run both Rust test suite and Phoenix dashboard tests
	@echo -e "$(GREEN)✔ All test suites passed successfully!$(RESET)"

test-bin: ## Run tests for binary utilities (init parser)
	@echo -e "$(CYAN)--> Testing binary utilities...$(RESET)"
	$(CARGO) test --bin init

test-dashboard: ## Run Elixir Phoenix dashboard test suite
	@echo -e "$(CYAN)--> Running dashboard test suite...$(RESET)"
	@cd dashboard && $(MIX) test

check: ## Fast compilation check without code generation
	@echo -e "$(CYAN)--> Running fast cargo check...$(RESET)"
	$(CARGO) check --all-targets

lint: clippy ## Alias for clippy
clippy: ## Run strict Clippy linter (-D warnings)
	@echo -e "$(CYAN)--> Running Clippy (-D warnings)...$(RESET)"
	$(CARGO) clippy --all-targets -- -D warnings

fmt: ## Format Rust codebase according to rustfmt style
	@echo -e "$(CYAN)--> Formatting Rust code...$(RESET)"
	$(CARGO) fmt

fmt-check: ## Verify Rust code conforms to rustfmt guidelines
	@echo -e "$(CYAN)--> Checking formatting guidelines...$(RESET)"
	$(CARGO) fmt --check

ci: fmt-check clippy test-all ## Run full local CI pipeline (fmt, clippy, test-all)
	@echo -e "$(GREEN)✔ Local CI validation succeeded!$(RESET)"

# ------------------------------------------------------------------------------
# Dashboard Targets
# ------------------------------------------------------------------------------

##@ Phoenix LiveView Dashboard
dashboard-setup: ## Fetch and install Phoenix dashboard dependencies
	@echo -e "$(CYAN)--> Installing dashboard dependencies...$(RESET)"
	@cd dashboard && $(MIX) deps.get

dashboard-server: dashboard-dev ## Alias for dashboard-dev
dashboard-dev: ## Start Phoenix LiveView telemetry dashboard locally (port 4040)
	@echo -e "$(CYAN)--> Starting Phoenix Dashboard on http://localhost:4040...$(RESET)"
	@lsof -ti:4040 | xargs kill -9 2>/dev/null || true
	@cd dashboard && $(MIX) phx.server

dashboard-stop: ## Stop any running Phoenix Dashboard instance on port 4040
	@echo -e "$(YELLOW)--> Stopping dashboard on port 4040...$(RESET)"
	@lsof -ti:4040 | xargs kill -9 2>/dev/null || true
	@echo -e "$(GREEN)✔ Dashboard stopped.$(RESET)"

dashboard-fmt: ## Format Elixir dashboard codebase
	@echo -e "$(CYAN)--> Formatting dashboard code...$(RESET)"
	@cd dashboard && $(MIX) format

# ------------------------------------------------------------------------------
# MicroVM Runtime Operations
# ------------------------------------------------------------------------------

##@ MicroVM Runtime Operations
vm-ps: ## List active and hibernated MicroVM instances
	@$(CARGO) run --quiet -- microvm ps

vm-metering: ## Display compute metering and usage summary
	@$(CARGO) run --quiet -- microvm metering --summary

vm-gc: ## Reconcile orphaned resources and enforce snapshot retention
	@$(CARGO) run --quiet -- microvm gc

# ------------------------------------------------------------------------------
# Cleanup Targets
# ------------------------------------------------------------------------------

##@ Cleanup Targets
clean: ## Remove Rust target build artifacts
	@echo -e "$(YELLOW)--> Cleaning target directory...$(RESET)"
	$(CARGO) clean

clean-all: clean ## Clean target build, Phoenix artifacts, and local state
	@echo -e "$(YELLOW)--> Cleaning dashboard artifacts and local temp files...$(RESET)"
	@rm -rf dashboard/_build dashboard/deps
	@echo -e "$(GREEN)✔ Deep clean complete!$(RESET)"
