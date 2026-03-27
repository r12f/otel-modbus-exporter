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
	docker buildx build --platform linux/amd64,linux/arm64 -t otel-modbus-exporter .

e2e:
	docker compose -f docker-compose.test.yml up --build --abort-on-container-exit

clean:
	cargo clean
