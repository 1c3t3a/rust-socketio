.PHONY: build test-fast test-all clippy format

build: 
	@cargo build --verbose

test-fast:
	@cargo test --verbose --package rust_socketio --lib -- engineio::packet && cargo test --verbose --package rust_socketio --lib -- socketio::packet

test-all:
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
