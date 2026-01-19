#!/bin/bash

# Verification script for Apple Silicon support
# This script checks that the correct pdu binary is being bundled for each architecture

echo "=== Apple Silicon Support Verification ==="
echo ""

# Check if we're on macOS
if [[ "$OSTYPE" == "darwin"* ]]; then
    echo "Platform: macOS"

    # Detect architecture
    ARCH=$(uname -m)
    echo "Architecture: $ARCH"

    # Check for sidecar binaries
    echo ""
    echo "Checking sidecar binaries..."

    if [ -f "src-tauri/bin/pdu-aarch64-apple-darwin" ]; then
        echo "✓ Apple Silicon binary (aarch64) found"
        file src-tauri/bin/pdu-aarch64-apple-darwin
    else
        echo "✗ Apple Silicon binary (aarch64) NOT found"
    fi

    if [ -f "src-tauri/bin/pdu-x86_64-apple-darwin" ]; then
        echo "✓ Intel binary (x86_64) found"
        file src-tauri/bin/pdu-x86_64-apple-darwin
    else
        echo "✗ Intel binary (x86_64) NOT found"
    fi

    # Check Rust toolchain
    echo ""
    echo "Checking Rust toolchain..."
    if command -v rustc &> /dev/null; then
        echo "Rust version: $(rustc --version)"
        echo ""
        echo "Installed targets:"
        rustup target list --installed | grep apple-darwin
    else
        echo "✗ Rust not found"
    fi

    # Check if targets are installed
    echo ""
    echo "Recommended targets:"
    echo "  - aarch64-apple-darwin (Apple Silicon)"
    echo "  - x86_64-apple-darwin (Intel Mac)"

    if ! rustup target list --installed | grep -q "aarch64-apple-darwin"; then
        echo ""
        echo "To add Apple Silicon target, run:"
        echo "  rustup target add aarch64-apple-darwin"
    fi

    if ! rustup target list --installed | grep -q "x86_64-apple-darwin"; then
        echo ""
        echo "To add Intel target, run:"
        echo "  rustup target add x86_64-apple-darwin"
    fi
else
    echo "Platform: $OSTYPE"
    echo "This verification script is optimized for macOS."
    echo "On CI, the GitHub Actions workflow handles architecture targeting."
fi

echo ""
echo "=== Verification Complete ==="
