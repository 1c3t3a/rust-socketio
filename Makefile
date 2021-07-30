.PHONY: build test-fast test-all clippy format checks pipeline

build: 
	@cargo build --verbose

keys:
	@./ci/keygen.sh node-engine-io-secure 127.0.0.1

test-fast: keys
	@cargo test --verbose --package rust_socketio --lib -- engineio::packet && cargo test --verbose --package rust_socketio --lib -- socketio::packet

test-all: keys
	@cargo test --verbose 

clippy:
	@cargo clippy --verbose

format:
	@cargo fmt --all -- --check

checks: build test-fast clippy format
	@echo "### Don't forget to add untracked files! ###"
	@git status
	@echo "### Awesome work! 😍 ###"""

pipeline: build test-all clippy format
	@echo "### Don't forget to add untracked files! ###"
	@git status
	@echo "### Awesome work! 😍 ###"""
