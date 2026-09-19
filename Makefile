# Makefile
#
# Flatten PM root Makefile.
# Delegates to scripts/flatten-sync/ for project export and watch.

PROFILE ?= default
FLATTEN_DIR = scripts/flatten-sync

.DEFAULT_GOAL := help

# -- Help ----------------------------------------------------------------

.PHONY: help

help:
	@echo "flatten-pm"
	@echo ""
	@echo "  Development:"
	@echo "    npm run tauri dev            Run Tauri app in dev mode"
	@echo "    cargo test --workspace       Run all Rust tests"
	@echo "    make fixtures                Generate test fixtures (node_modules)"
	@echo ""
	@echo "  Project sync:"
	@echo "    make project-export              Export to Claude Project (default profile)"
	@echo "    make project-export PROFILE=x    Export with named profile"
	@echo "    make project-export-dry-run      Preview without copying"
	@echo "    make project-watch               Watch Downloads, auto-place files"
	@echo ""
	@echo "  Flatten-sync tests:"
	@echo "    make project-test                Run flatten-sync unit tests"

# -- Fixtures ------------------------------------------------------------

.PHONY: fixtures

fixtures:
	@echo "Generating fixtures/repo-a/node_modules/ (1000 files)..."
	@mkdir -p fixtures/repo-a/node_modules
	@for i in $$(seq 1 1000); do \
		echo "// dummy module $$i" > fixtures/repo-a/node_modules/mod_$$i.js; \
	done
	@echo "Done: 1000 files in fixtures/repo-a/node_modules/"

# -- Project sync --------------------------------------------------------

.PHONY: project-export project-export-dry-run project-watch project-test

project-export:
	$(MAKE) -C $(FLATTEN_DIR) run PROFILE=$(PROFILE) PROJECT_ROOT=$(CURDIR)

project-export-dry-run:
	$(MAKE) -C $(FLATTEN_DIR) dry-run PROFILE=$(PROFILE) PROJECT_ROOT=$(CURDIR)

project-watch:
	$(MAKE) -C $(FLATTEN_DIR) watch PROFILE=$(PROFILE)

project-test:
	$(MAKE) -C $(FLATTEN_DIR) test
