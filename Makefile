# mirsam — developer entry points.
# VERSION and RELEASE_NAME are the single source of truth for release metadata.

VERSION      := $(shell cat VERSION)
RELEASE_NAME := $(shell cat RELEASE_NAME)
THEME        := arabian_birds
MSRV         := $(shell grep -m1 '^rust-version' Cargo.toml | cut -d'"' -f2)

.DEFAULT_GOAL := help

help: ## Show available targets
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

build: ## Build the release binary
	@cargo build --release
	@echo "built mirsam $(VERSION) \"$(RELEASE_NAME)\" -> target/release/mirsam"

test: ## Run the full test suite
	@cargo test --all

fmt: ## Format sources
	@cargo fmt --all

fmt-check: ## Fail if rustfmt would change files
	@cargo fmt --all -- --check

lint: ## Clippy, warnings as errors
	@cargo clippy --all-targets --all-features -- -D warnings

doc: ## Build API docs, warnings fatal (as CI does)
	@RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace

clean: ## Remove build artifacts
	@cargo clean

version: ## Print release metadata
	@echo "version=$(VERSION) codename=$(RELEASE_NAME)"

check-version: ## Assert VERSION is the single source of truth
	@sh scripts/check-version.sh

codename: ## Generate a one-off codename (stdout only)
	@command -v tagtastic >/dev/null 2>&1 || { \
		echo "tagtastic not installed. Run: go install github.com/aenawi/tagtastic/cmd/tagtastic@latest"; \
		exit 1; \
	}
	@tagtastic generate --theme $(THEME)

release-name: ## Write RELEASE_NAME from tagtastic (slug form)
	@command -v tagtastic >/dev/null 2>&1 || { \
		echo "tagtastic not installed. Run: go install github.com/aenawi/tagtastic/cmd/tagtastic@latest"; \
		exit 1; \
	}
	@tagtastic generate --theme $(THEME) \
		| tr '[:upper:]' '[:lower:]' | tr -s ' ' '-' > RELEASE_NAME
	@echo "RELEASE_NAME=$$(cat RELEASE_NAME)"

msrv: ## Check the declared minimum supported Rust version still builds
	@RUSTUP_TOOLCHAIN=$(MSRV) cargo check --all \
		|| { echo "install it first: rustup toolchain install $(MSRV)"; exit 1; }
	@echo "msrv ok: $(MSRV)"

audit-deps: ## Report known vulnerabilities in dependencies
	@command -v cargo-audit >/dev/null 2>&1 || { \
		echo "cargo-audit not installed. Run: cargo install cargo-audit"; \
		exit 1; \
	}
	@cargo audit

golden: ## Regenerate the golden corpus reports; review the diff before committing
	@MIRSAM_UPDATE_GOLDEN=1 cargo test -p mirsam-cli --test golden --quiet
	@git --no-pager diff --stat -- tests/fixtures

fixtures: ## Regenerate the hand-built corpus decks, then their reports
	@python3 scripts/make-torture-fixture.py
	@$(MAKE) --no-print-directory golden

validate-fixtures: ## Validate every corpus deck against the ECMA-376 schemas (needs uv)
	@command -v uv >/dev/null 2>&1 || { \
		echo "uv not installed; alternatively: pip install lxml && python3 scripts/validate-ooxml.py"; \
		exit 1; \
	}
	@uv run --quiet --with lxml scripts/validate-ooxml.py

corpus: ## Regenerate the generated corpus decks (needs uv), then their reports
	@command -v uv >/dev/null 2>&1 || { \
		echo "uv not installed; alternatively: pip install python-pptx && python3 scripts/make-corpus.py"; \
		exit 1; \
	}
	@uv run --quiet --with python-pptx scripts/make-corpus.py
	@$(MAKE) --no-print-directory golden

verify: check-version fmt-check lint test doc build ## Full pre-PR check (mirrors CI)

pre-push: verify ## What the git pre-push hook runs

hooks-install: ## Enable repo .githooks
	@git config core.hooksPath .githooks
	@echo "hooks enabled: .githooks"

install: ## Install mirsam into ~/.cargo/bin
	@cargo install --path crates/mirsam-cli

.PHONY: help build test fmt fmt-check lint doc clean version check-version \
        codename release-name msrv audit-deps golden fixtures \
        validate-fixtures corpus verify pre-push hooks-install install
