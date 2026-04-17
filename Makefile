.PHONY: build build-release desktop desktop-build desktop-run clean openapi

# Build the byokey binary (debug)
build:
	cargo build

# Build the byokey binary (release)
build-release:
	cargo build --release

# Regenerate OpenAPI spec and Swift client from Rust utoipa annotations
openapi: build
	cargo run -- openapi 2>/dev/null | python3 -m json.tool > desktop/Byokey/openapi.json
	cd desktop/Byokey && swift-openapi-generator generate openapi.json \
		--config openapi-generator-config.yaml \
		--output-directory Generated/

# Build the binary then open the Xcode project
desktop: build openapi
	open desktop/Byokey.xcodeproj

# Build the desktop app without Xcode
desktop-build:
	xcodebuild -project desktop/Byokey.xcodeproj -scheme Byokey -configuration Debug build

# Build and launch the desktop app
desktop-run: desktop-build
	@open "$$(find ~/Library/Developer/Xcode/DerivedData/Byokey-*/Build/Products/Debug -name '*.app' -maxdepth 1 | head -1)"

clean:
	cargo clean
