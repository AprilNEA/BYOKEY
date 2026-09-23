.PHONY: build build-release clean

# Build the byokey binary (debug)
build:
	cargo build

# Build the byokey binary (release)
build-release:
	cargo build --release

clean:
	cargo clean
