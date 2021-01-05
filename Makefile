.PHONY: build test clippy format

build: 
	@cargo build --verbose

test:
	@cargo test --verbose --package rust_socketio --lib -- socketio::packet::test 

clippy:
	@cargo clippy --verbose

format:
	@cargo fmt --all -- --check

checks: build test clippy format
	@echo "### Don't forget to add untracked files! ###"
	@git status
	@echo "### Awesome work! 😍 ###"""