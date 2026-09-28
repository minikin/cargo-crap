# Install

**Via `cargo binstall`** (downloads the right pre-built binary automatically):

```bash
cargo binstall cargo-crap
```

**From source** (requires Rust stable ≥ 1.88):

```bash
cargo install cargo-crap
```

**From the AUR**:

```bash
paru -S cargo-crap
```

**Pre-built binary** (manual download):

```bash
# macOS (Apple Silicon)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-aarch64-apple-darwin.tar.gz | tar xz -C ~/.cargo/bin

# macOS (Intel)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-x86_64-apple-darwin.tar.gz | tar xz -C ~/.cargo/bin

# Linux (x86_64)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-x86_64-unknown-linux-gnu.tar.gz | tar xz -C ~/.cargo/bin

# Linux (aarch64)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-aarch64-unknown-linux-gnu.tar.gz | tar xz -C ~/.cargo/bin
```

Windows: download `cargo-crap-x86_64-pc-windows-msvc.zip` from the [latest release](https://github.com/minikin/cargo-crap/releases/latest) and extract `cargo-crap.exe` into a directory on your `PATH`.
