# Install crapts

[crapts](README.md) ships as an npm package with a prebuilt binary, as release archives for `cargo binstall`, and as a crate.

## npm

Run it once without installing:

```bash
npx @minikin/crapts
```

Or add it to a project's dev dependencies, so CI and every developer run the same version:

```bash
npm install -D @minikin/crapts
```

The package needs Node.js 18 or later. `@minikin/crapts` holds a small launcher, and npm installs the one platform package that matches the machine:

| Package | Platform |
|---|---|
| `@minikin/crapts-darwin-arm64` | macOS on Apple silicon |
| `@minikin/crapts-darwin-x64` | macOS on Intel |
| `@minikin/crapts-linux-x64-gnu` | Linux x64, glibc |
| `@minikin/crapts-linux-arm64-gnu` | Linux arm64, glibc |
| `@minikin/crapts-linux-x64-musl` | Linux x64, musl (Alpine) |
| `@minikin/crapts-win32-x64-msvc` | Windows x64 |

An install that skips optional dependencies, such as `npm install --omit=optional`, leaves the launcher without a binary: it exits 2 and asks for a reinstall with optional dependencies.

## Cargo

`cargo binstall` downloads the release archive for the host from GitHub:

```bash
cargo binstall crapts
```

Building from source needs a Rust toolchain. The minimum Rust version is `1.98`.

```bash
cargo install --locked crapts
```

Check the install:

```bash
crapts --version
```
