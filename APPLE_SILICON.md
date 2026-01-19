# Apple Silicon Support

This document describes the Apple Silicon (M1/M2/M3) modernization changes made to SquirrelDisk.

## Overview

SquirrelDisk now fully supports native Apple Silicon (arm64/aarch64) builds alongside traditional Intel (x86_64) builds. This provides optimal performance on modern Mac hardware.

## What Changed

### 1. Dependencies Updated

**Tauri Framework:**
- Updated from `1.2.x` to `1.6.x` (latest v1.x stable)
- Includes improved Apple Silicon support and bug fixes

**Frontend Tooling:**
- Vite: `4.x` → `5.4.x`
- TypeScript: `4.6.x` → `5.6.x`
- React plugins updated for compatibility

### 2. CI/CD Modernization

The GitHub Actions workflow (`.github/workflows/main.yml`) now explicitly builds for both architectures:

**Build Matrix:**
- `macos-latest` → Apple Silicon (aarch64-apple-darwin)
- `macos-13` → Intel Mac (x86_64-apple-darwin)
- `ubuntu-20.04` → Linux x86_64
- `windows-latest` → Windows x86_64

**Key Improvements:**
- Explicit Rust target specification
- Separate DMG files for each Mac architecture
- Updated to modern GitHub Actions (v4)
- Optimized with `npm ci` instead of `npm i`

### 3. Sidecar Binary Support

The project includes pre-compiled `pdu` (parallel disk usage) binaries for both architectures:

```
src-tauri/bin/
├── pdu-aarch64-apple-darwin  (Apple Silicon)
├── pdu-x86_64-apple-darwin   (Intel Mac)
├── pdu-x86_64-unknown-linux-gnu
└── pdu-x86_64-pc-windows-msvc.exe
```

Tauri automatically selects the correct binary for the target architecture during build.

### 4. Code Signing

Code signing is configured via environment variables:

**CI/CD:** Set `APPLE_SIGNING_IDENTITY` in your workflow secrets
**Local Development:** Uses ad-hoc signing automatically

## Building Locally

### Prerequisites

Install Rust targets for both architectures:

```bash
rustup target add aarch64-apple-darwin
rustup target add x86_64-apple-darwin
```

### Build Commands

**Apple Silicon:**
```bash
npm run tauri build -- --target aarch64-apple-darwin
```

**Intel Mac:**
```bash
npm run tauri build -- --target x86_64-apple-darwin
```

**Current Architecture (auto-detect):**
```bash
npm run tauri build
```

### Verification

Run the architecture verification script:

```bash
./scripts/verify-arch.sh
```

This checks:
- Available sidecar binaries
- Installed Rust toolchains and targets
- Current system architecture

## Release Process

When you create a new release tag (e.g., `v0.3.5`), the workflow automatically:

1. Builds 4 separate artifacts:
   - `SquirrelDisk_<version>_aarch64.dmg` (Apple Silicon)
   - `SquirrelDisk_<version>_x64.dmg` (Intel Mac)
   - Linux AppImage/deb
   - Windows installer

2. Creates a draft release with all artifacts

3. Signs macOS builds (if credentials configured)

## Universal Binaries

Currently, the project builds separate binaries for each architecture. Universal binaries (combining both architectures) are possible but:

- Require double the disk space
- Add complexity with sidecar binaries
- Separate DMGs provide better download size optimization

If you need universal binaries, you would need to:
1. Build for both targets
2. Use `lipo` to combine the executables
3. Handle sidecar binaries separately

## Troubleshooting

### Build fails with "target not installed"

Install the required Rust target:
```bash
rustup target add aarch64-apple-darwin
```

### Wrong binary architecture in bundle

Verify sidecar binaries exist and are correctly named:
```bash
ls -lh src-tauri/bin/
```

### Code signing errors

For local builds, you can skip signing:
```bash
npm run tauri build -- --target aarch64-apple-darwin --config '{"bundle":{"macOS":{"signingIdentity":null}}}'
```

For CI, ensure `APPLE_SIGNING_IDENTITY`, `APPLE_CERTIFICATE`, and `APPLE_CERTIFICATE_PASSWORD` secrets are configured.

## Performance Benefits

Native Apple Silicon builds provide:
- **~2x faster execution** on M1/M2/M3 chips
- **Lower power consumption** (no Rosetta translation)
- **Better system integration** with macOS features
- **Smaller binary size** (no x86_64 code included)

## Migration Notes

For users upgrading from previous versions:

1. **Intel Mac users:** Continue using `_x64.dmg` builds
2. **Apple Silicon users:** Switch to `_aarch64.dmg` for best performance
3. **Auto-update:** The updater will automatically select the correct architecture

## References

- [Tauri v1.6 Documentation](https://v1.tauri.app/)
- [Rust Cross-Compilation Guide](https://rust-lang.github.io/rustup/cross-compilation.html)
- [Apple Silicon Developer Guide](https://developer.apple.com/documentation/apple-silicon)
