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
	docker buildx build --platform linux/amd64,linux/arm64 -t modbus-exporter .

e2e:  ## Run E2E tests with docker-compose
	bash tests/e2e/run.sh

clean:
	cargo clean
