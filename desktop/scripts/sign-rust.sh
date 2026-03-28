#!/bin/bash
set -euo pipefail
BINARY_PATH="${BUILT_PRODUCTS_DIR}/${CONTENTS_FOLDER_PATH}/Resources/byokey"

if [ ! -f "${BINARY_PATH}" ]; then
    echo "error: byokey binary not found at ${BINARY_PATH}" >&2
    exit 1
fi

APP_XCENT="${TARGET_TEMP_DIR}/${PRODUCT_NAME}.app.xcent"
if [ -f "${APP_XCENT}" ]; then
    codesign --force --options runtime --sign "${EXPANDED_CODE_SIGN_IDENTITY}" \
        --timestamp --identifier "io.byokey.cli" \
        --entitlements "${APP_XCENT}" "${BINARY_PATH}"
else
    codesign --force --options runtime --sign "${EXPANDED_CODE_SIGN_IDENTITY}" \
        --timestamp --identifier "io.byokey.cli" "${BINARY_PATH}"
fi
