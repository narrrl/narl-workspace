.PHONY: dev dev-api dev-web build run check wiki screenshots clean

# Two terminals: `make dev-api` and `make dev-web` (http://127.0.0.1:4243).
dev-api:
	cargo run

dev-web:
	cd web && bun run dev

# Everything, as one self-contained release binary at target/release/narl-workspace.
build:
	cd web && bun install --frozen-lockfile && bun run build
	cargo build --release

run: build
	./target/release/narl-workspace

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test
	cd web && bun run check
	bun scripts/wiki.ts --check

# The generated wiki pages, from the code they describe: see docs/wiki/Development.md.
wiki:
	bun scripts/wiki.ts

# docs/images, taken again from a throwaway workspace in a headless Chromium.
screenshots:
	cd web && bun install --frozen-lockfile && bun run build
	cargo build
	bun scripts/screenshots.ts

clean:
	cargo clean
	rm -rf web/dist web/node_modules
