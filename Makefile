.PHONY: build test-fast run-test-servers test-all clippy format checks pipeline

build:
	@cargo build --verbose --all-features

keys:
	@./ci/keygen.sh node-engine-io-secure 127.0.0.1

test-fast: keys
	@cargo test --verbose --package rust_socketio --lib -- engineio::packet && cargo test --verbose --package rust_socketio --lib -- socketio::packet

run-test-servers:
	cd ci && docker build -t test_suite:latest . && cd ..
	docker run -d -p 4200:4200 -p 4201:4201 -p 4202:4202 -p 4203:4203 -p 4204:4204 -p 4205:4205 -p 4206:4206  --name socketio_test test_suite:latest

test-all: keys run-test-servers
	@cargo test --verbose --all-features
	docker stop socketio_test

clippy:
	@cargo clippy --verbose --all-features

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
