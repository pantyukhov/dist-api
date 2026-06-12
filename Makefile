build:
	cargo build

test:
	cargo test

run:
	cargo run --bin dist-api -- --metadata-dir crates/metadata/tests/fixtures/metadata

claude:
	claude --dangerously-skip-permissions --teammate-mode tmux
