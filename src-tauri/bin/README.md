## Updating binaries

Run `scripts/update-pdu.sh [version]` from the repo root. It downloads pre-built
binaries from the GitHub release and builds `aarch64-apple-darwin` natively.

## PDU Version: 0.23.0

### Windows

Using the GNU toolchain build renamed as `pdu-x86_64-pc-windows-msvc.exe` (no
dependency on Visual C++ Redistributable).

### Mac / Linux

`x86_64-apple-darwin` and `x86_64-unknown-linux-gnu` are downloaded from the
GitHub release. `aarch64-apple-darwin` is built natively from source via
`cargo install`.
