# scripts/flatten-sync/Makefile
#
# Portable Makefile for the flatten + watch sync tools.
# Travels with the flatten-sync/ directory when injected into other repos.
#
# From repo root (via delegation):
#   make project-export
#   make project-export PROFILE=bench-code
#   make project-export-discover
#   make project-export-test
#   make project-watch
#   make project-watch PROFILE=work-laptop
#   make project-watch-test
#
# Standalone (from this directory):
#   make
#   make PROFILE=bench-code
#   make discover
#   make test
#   make test-watch
#   make watch
#   make watch PROFILE=work-laptop

PROFILE ?= default
PROJECT_ROOT ?= $(shell git rev-parse --show-toplevel)

.PHONY: run discover dry-run test test-watch watch

run:
	uv run flatten_for_project.py run --profile $(PROFILE) --root $(PROJECT_ROOT)

discover:
	uv run flatten_for_project.py discover --profile $(PROFILE) --root $(PROJECT_ROOT)

dry-run:
	uv run flatten_for_project.py run --profile $(PROFILE) --root $(PROJECT_ROOT) --dry-run

test:
	uv run --with pytest --with pyyaml pytest tests/ -q

test-watch:
	bash tests/test-watch.sh

watch:
	uv run watch.py --profile $(PROFILE) -v
