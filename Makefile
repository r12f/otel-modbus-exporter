.PHONY: build run fmt lint test docker e2e clean

build:
	cargo build --release

run:
	cargo run -- --config config/example.yaml

fmt:
	cargo fmt

lint:
	cargo clippy -- -D warnings

test:
	cargo test

docker:
	docker buildx build --platform linux/amd64,linux/arm64 -t bus-exporter .

e2e:  ## Run native E2E tests (Rust-based Modbus simulator, no Docker needed)
	cargo test --test e2e_modbus -- --nocapture

clean:
	cargo clean
