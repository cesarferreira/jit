.PHONY: install test release

install:
	cargo install --path .

test:
	cargo nextest run

release: test
	cargo release minor --execute --no-confirm
