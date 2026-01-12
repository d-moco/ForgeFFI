# ForgeFFI

[中文](README.md)

ForgeFFI is a set of general-purpose utilities implemented in Rust, shipped as FFI deliverables for C and other languages:

- Dynamic library / static library
- Auto-generated C header files
- A stable `dist/` output layout
- A single build front-end (`xtask`) with both interactive and non-interactive flows

If you want Rust-level performance and safety in a C/C++/C#/Java project without maintaining a pile of platform-specific scripts, this repository is built for that.

## Highlights

- One command to produce distributable artifacts: `.dll/.so/.dylib`, `.a/.lib`, and `.h`
- Easier cross-compiling: optional Zig-assisted toolchain via `cargo-zigbuild`
- CI-friendly outputs: artifacts are copied into a deterministic `dist/<target>/<profile>/...` structure
- Open-source ready: dual license + contribution and security docs

## Quick Start

### 1) Interactive build (recommended)

```bash
cargo xtask menu
```

The menu lets you select profile, build mode (module/aggregate, Rust/FFI), artifact type (dynamic/static), target triple (including `all`), whether to use zigbuild, and whether to generate C headers.

### 2) Non-interactive build

Module FFI (example: `net` as a dynamic library):

```bash
cargo xtask build --mode module-ffi --modules net --profile release --artifact cdylib --headers=true
```

Aggregate FFI (example: `full`):

```bash
cargo xtask build --mode aggregate-ffi --features full --profile release --artifact cdylib --headers=true
```

Download Zig and print its path (cached):

```bash
cargo xtask zig --version 0.12.0
```

## Artifacts (dist)

FFI builds copy artifacts to `dist/`:

```
dist/<target>/<debug|release>/<pkg>/<cdylib|staticlib>/...
dist/<target>/<debug|release>/<pkg>/include/<pkg>.h
```

On Windows, dynamic libraries also ship an import library for linking:

- MSVC: `.dll` + `.dll.lib`
- GNU/LLVM: `.dll` + `.dll.a`

Note: some targets (e.g. `*-unknown-linux-musl`) may not support `cdylib`. If a dynamic library is requested but not produced, the build tool automatically falls back to `staticlib` and prints a hint.

## C integration example

Build `forgeffi-net-ffi` (Windows MSVC, release):

```bash
cargo xtask build --mode module-ffi --modules net --profile release --target x86_64-pc-windows-msvc --zigbuild=false --headers=true
```

In your C project:

- include: `dist/x86_64-pc-windows-msvc/release/forgeffi-net-ffi/include/forgeffi-net-ffi.h`
- link: `dist/x86_64-pc-windows-msvc/release/forgeffi-net-ffi/cdylib/forgeffi_net_ffi.dll.lib`
- runtime: make sure `forgeffi_net_ffi.dll` is in a loadable path

## Cross-compiling and `all`

- `menu -> all` builds a curated list of target triples.
- If `rust-std` is missing for a target, the build tool runs `rustup target add <target>` automatically.
- Apple targets typically require Apple SDKs; they are skipped on non-macOS hosts.
- On Windows, `*-pc-windows-msvc` may be incompatible with zigbuild; the menu explicitly asks whether to keep MSVC (disable zigbuild) or switch to a zigbuild-friendly target.

## Development

```bash
cargo test
cargo clippy --workspace -- -D warnings
```

## Contributing & Security

- [CONTRIBUTING.md](CONTRIBUTING.md)
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- [SECURITY.md](SECURITY.md)

## License

Dual-licensed under Apache-2.0 OR MIT:

- [LICENSE-APACHE](LICENSE-APACHE)
- [LICENSE-MIT](LICENSE-MIT)
