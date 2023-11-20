
.PHONY: build
build:
	cargo build --verbose

.PHONY:	test
test:
	cargo test -- --test-threads=1 --nocapture
