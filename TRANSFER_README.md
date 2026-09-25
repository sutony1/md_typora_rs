# Development on another Windows computer

For normal use, download the Windows executable from the [GitHub Releases page](https://github.com/sutony1/md_typora_rs/releases). It does not need Rust or a command prompt.

For development, clone the repository and follow [README.md](README.md). The build needs `Cargo.toml`, `Cargo.lock`, `src/main.rs`, and `src/mathjax_tex_svg.js`. Install the Rust MSVC toolchain, Visual Studio C++ Build Tools with Windows SDK if necessary, and Microsoft Edge WebView2 Runtime to run the application. Then run `cargo test` and `cargo build --release`.

The following are intentionally not part of the repository: `target/`, local backups, old ZIP archives, and local document files.
