PYTHON := .venv/bin/python
export MASONWING_TDD_CASE = $(CASE)

.PHONY: bootstrap generate check test test-integration test-e2e tdd dev-up dev-down dev-status dev-logs dev-seed dev-build
bootstrap:
	python3 -m venv .venv
	$(PYTHON) -m pip install -r requirements-dev.txt
	pnpm install --frozen-lockfile
	python3 scripts/dev.py preflight

generate:
	python3 scripts/sync_specs.py
	pnpm contracts:generate

check:
	python3 scripts/sync_specs.py --check
	node scripts/generate-types.mjs --check
	cargo fmt --all -- --check
	cargo check --locked --workspace --jobs 2
	cargo clippy --locked --workspace --all-targets --jobs 2 -- -D warnings
	pnpm typecheck

test:
	cargo test --locked --workspace --jobs 2
	$(PYTHON) -m pytest tests/scaffold -q
	pnpm test

test-integration:
	$(PYTHON) -m pytest tests/integration -q

test-e2e:
	pnpm test:e2e

# Example: make tdd CASE='MASONWING@1.0.1:TC-AC-088'
# Missing implementations intentionally exit nonzero.
tdd:
	$(PYTHON) scripts/tdd.py

dev-up:
	python3 scripts/dev.py up
	$(PYTHON) scripts/seed_dev.py

dev-seed:
	$(PYTHON) scripts/seed_dev.py

dev-build:
	python3 scripts/dev.py build

dev-down:
	python3 scripts/dev.py down

dev-status:
	python3 scripts/dev.py status

dev-logs:
	python3 scripts/dev.py logs
