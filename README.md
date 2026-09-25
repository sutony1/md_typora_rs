# Markdown Renderer

A lightweight Windows Markdown viewer/editor built with Rust. It opens in preview mode, with an optional split editor, themes, local images, and MathJax formula rendering. The release executable is a GUI application and does not open a command prompt.

## Download and run on another Windows PC

1. Open [Releases](https://github.com/sutony1/md_typora_rs/releases) and download `md_typora_rs.exe` from the latest release. Do not download only the source-code ZIP if you just want to run the app.
2. Use Windows 10 or 11 with Microsoft Edge WebView2 Runtime. Most PCs already have it; otherwise install it from [Microsoft](https://developer.microsoft.com/en-us/microsoft-edge/webview2/).
3. Double-click the EXE. Choose **Open** or drag a `.md`, `.markdown`, or `.mdown` file into the window. You can also pass a file path on the command line.

中文：在 Releases 下载 `md_typora_rs.exe`，双击运行；点击 Open 打开 Markdown 文件，或直接拖入窗口。无需安装 Rust；若提示缺少 WebView2，请先安装微软 WebView2 Runtime。

## Features

- Preview and edit Markdown with local image support.
- Math formulas rendered using bundled MathJax (no network needed).
- UTF-8 and UTF-8 BOM files; GBK files are supported on Windows. Saving to the same path preserves the original encoding. **Save As** writes UTF-8.
- Light, Sepia, and Dark themes.

Shortcuts: `Ctrl+S` save, `Ctrl+O` open, `Ctrl+Shift+P` switch edit/preview, `Ctrl++`/`Ctrl+-` zoom, `Ctrl+0` reset zoom.

## Build from source

Install the [Rust MSVC toolchain](https://www.rust-lang.org/tools/install) and, if needed, [Visual Studio C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with Windows SDK. Then:

```powershell
git clone https://github.com/sutony1/md_typora_rs.git
cd md_typora_rs
cargo test
cargo build --release
```

The resulting file is `target\release\md_typora_rs.exe`. The first build downloads crates. The repository includes `src/mathjax_tex_svg.js`, which is required to compile the embedded MathJax bundle. Do not copy the `target` directory between computers.

## License

The application code is MIT licensed (see [LICENSE](LICENSE)). The bundled MathJax script in `src/mathjax_tex_svg.js` is from the [MathJax project](https://github.com/mathjax/MathJax) and is Apache-2.0 licensed; see [MATHJAX_LICENSE](MATHJAX_LICENSE). Other Rust dependencies have their own licenses.
