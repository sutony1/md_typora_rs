#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pulldown_cmark::{Options, Parser, html};
use serde::Deserialize;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::http::Response;
use wry::{DragDropEvent, WebView, WebViewBuilder};

#[derive(Debug)]
enum UserEvent {
    Ipc(String),
    OpenPath(PathBuf),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum IpcCommand {
    Ready,
    Render { markdown: String },
    Open,
    Save { markdown: String },
    SaveAs { markdown: String },
    NewDocument,
}

#[derive(Default)]
struct AppState {
    current_path: Option<PathBuf>,
    current_encoding: DocumentEncoding,
    pending_open: Option<PathBuf>,
    ui_ready: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DocumentEncoding {
    #[default]
    Utf8,
    Utf8Bom,
    Gbk,
}

fn decode_document(bytes: &[u8]) -> Result<(String, DocumentEncoding)> {
    if let Some(bytes) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return Ok((
            std::str::from_utf8(bytes)?.to_owned(),
            DocumentEncoding::Utf8Bom,
        ));
    }
    if let Ok(markdown) = std::str::from_utf8(bytes) {
        return Ok((markdown.to_owned(), DocumentEncoding::Utf8));
    }
    Ok((gbk::decode(bytes)?, DocumentEncoding::Gbk))
}

fn encode_document(markdown: &str, encoding: DocumentEncoding) -> Result<Vec<u8>> {
    match encoding {
        DocumentEncoding::Utf8 => Ok(markdown.as_bytes().to_vec()),
        DocumentEncoding::Utf8Bom => {
            let mut bytes = vec![0xef, 0xbb, 0xbf];
            bytes.extend_from_slice(markdown.as_bytes());
            Ok(bytes)
        }
        DocumentEncoding::Gbk => gbk::encode(markdown),
    }
}

#[cfg(windows)]
mod gbk {
    use anyhow::{Result, bail};
    use std::ptr;

    const CP_GBK: u32 = 936;
    const MB_ERR_INVALID_CHARS: u32 = 0x0000_0008;
    const WC_NO_BEST_FIT_CHARS: u32 = 0x0000_0400;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MultiByteToWideChar(
            code_page: u32,
            flags: u32,
            input: *const u8,
            input_len: i32,
            output: *mut u16,
            output_len: i32,
        ) -> i32;
        fn WideCharToMultiByte(
            code_page: u32,
            flags: u32,
            input: *const u16,
            input_len: i32,
            output: *mut u8,
            output_len: i32,
            default_char: *const u8,
            used_default_char: *mut i32,
        ) -> i32;
    }

    pub fn decode(bytes: &[u8]) -> Result<String> {
        if bytes.is_empty() {
            return Ok(String::new());
        }
        let len = i32::try_from(bytes.len())?;
        let needed = unsafe {
            MultiByteToWideChar(
                CP_GBK,
                MB_ERR_INVALID_CHARS,
                bytes.as_ptr(),
                len,
                ptr::null_mut(),
                0,
            )
        };
        if needed == 0 {
            bail!(
                "file is neither valid UTF-8 nor valid GBK: {}",
                std::io::Error::last_os_error()
            );
        }
        let mut wide = vec![0u16; needed as usize];
        let written = unsafe {
            MultiByteToWideChar(
                CP_GBK,
                MB_ERR_INVALID_CHARS,
                bytes.as_ptr(),
                len,
                wide.as_mut_ptr(),
                needed,
            )
        };
        if written != needed {
            bail!("GBK decode failed: {}", std::io::Error::last_os_error());
        }
        Ok(String::from_utf16(&wide)?)
    }

    pub fn encode(text: &str) -> Result<Vec<u8>> {
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let wide: Vec<u16> = text.encode_utf16().collect();
        let len = i32::try_from(wide.len())?;
        let mut used_default = 0;
        let needed = unsafe {
            WideCharToMultiByte(
                CP_GBK,
                WC_NO_BEST_FIT_CHARS,
                wide.as_ptr(),
                len,
                ptr::null_mut(),
                0,
                ptr::null(),
                &mut used_default,
            )
        };
        if needed == 0 {
            bail!("GBK encode failed: {}", std::io::Error::last_os_error());
        }
        if used_default != 0 {
            bail!(
                "This document uses GBK; the new text contains characters GBK cannot represent. Use Save As to create a UTF-8 copy."
            );
        }
        let mut bytes = vec![0u8; needed as usize];
        let written = unsafe {
            WideCharToMultiByte(
                CP_GBK,
                WC_NO_BEST_FIT_CHARS,
                wide.as_ptr(),
                len,
                bytes.as_mut_ptr(),
                needed,
                ptr::null(),
                &mut used_default,
            )
        };
        if written != needed || used_default != 0 {
            bail!("GBK encode failed without writing the file");
        }
        Ok(bytes)
    }
}

#[cfg(not(windows))]
mod gbk {
    use anyhow::{Result, bail};
    pub fn decode(_: &[u8]) -> Result<String> {
        bail!("GBK support is available on Windows only")
    }
    pub fn encode(_: &str) -> Result<Vec<u8>> {
        bail!("GBK support is available on Windows only")
    }
}
#[derive(Debug, PartialEq)]
enum StartupAction {
    Open(PathBuf),
    NewDocument,
}

impl AppState {
    fn on_ready(&mut self) -> Option<StartupAction> {
        if self.ui_ready {
            return None;
        }
        self.ui_ready = true;
        Some(match self.pending_open.take() {
            Some(path) => StartupAction::Open(path),
            None => StartupAction::NewDocument,
        })
    }
}

fn main() -> Result<()> {
    let mut event_loop_builder = EventLoopBuilder::<UserEvent>::with_user_event();
    let event_loop = event_loop_builder.build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("Markdown Renderer")
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .build(&event_loop)
        .context("failed to create application window")?;

    let ipc_proxy = proxy.clone();
    let drop_proxy = proxy.clone();
    let webview = WebViewBuilder::new()
        .with_url("local://localhost/index.html")
        .with_custom_protocol("local".into(), |_id, request| {
            let path = request.uri().path().to_string();
            let (mime, body): (&str, Cow<'static, [u8]>) = match path.as_str() {
                "/" | "/index.html" => (
                    "text/html; charset=utf-8",
                    Cow::Owned(app_html().into_bytes()),
                ),
                "/mathjax/tex-svg.js" => (
                    "application/javascript; charset=utf-8",
                    Cow::Borrowed(MATHJAX_SRC.as_bytes()),
                ),
                _ => (
                    "text/plain; charset=utf-8",
                    Cow::Borrowed(b"not found".as_slice()),
                ),
            };
            Response::builder()
                .header("Content-Type", mime)
                .body(body)
                .expect("static response cannot fail")
        })
        .with_ipc_handler(move |request| {
            let _ = ipc_proxy.send_event(UserEvent::Ipc(request.body().to_string()));
        })
        .with_drag_drop_handler(move |event| {
            if let DragDropEvent::Drop { paths, .. } = event {
                if let Some(path) = paths.into_iter().find(|path| is_markdown_path(path)) {
                    let _ = drop_proxy.send_event(UserEvent::OpenPath(path));
                }
                return true;
            }
            false
        })
        .build(&window)
        .context("failed to create WebView")?;

    // WebView navigation is asynchronous: wait for the frontend handshake.
    let mut state = AppState {
        pending_open: std::env::args_os().nth(1).map(PathBuf::from),
        ..AppState::default()
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(UserEvent::Ipc(raw)) => {
                if let Err(error) = handle_ipc(&webview, &mut state, &raw) {
                    send_error(&webview, &error.to_string());
                }
            }
            Event::UserEvent(UserEvent::OpenPath(path)) => {
                if !state.ui_ready {
                    state.pending_open = Some(path);
                } else if let Err(error) = open_path(&webview, &mut state, path) {
                    send_error(&webview, &error.to_string());
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

fn handle_ipc(webview: &WebView, state: &mut AppState, raw: &str) -> Result<()> {
    let command: IpcCommand =
        serde_json::from_str(raw).with_context(|| format!("invalid IPC message: {raw}"))?;

    match command {
        IpcCommand::Ready => match state.on_ready() {
            Some(StartupAction::Open(path)) => open_path(webview, state, path)?,
            Some(StartupAction::NewDocument) => new_document(webview, state)?,
            None => {}
        },
        IpcCommand::Render { markdown } => {
            let base = state.current_path.as_deref().and_then(Path::parent);
            let rendered = render_markdown(&markdown, base);
            call_js(webview, "window.app.setPreview", &[json_arg(&rendered)?])?;
        }
        IpcCommand::Open => {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Markdown", &["md", "markdown", "mdown"])
                .add_filter("Text", &["txt"])
                .pick_file()
            {
                open_path(webview, state, path)?;
            }
        }
        IpcCommand::Save { markdown } => {
            if let Some(path) = state.current_path.clone() {
                save_to_path(webview, state, path, &markdown, state.current_encoding)?;
            } else {
                save_as(webview, state, &markdown)?;
            }
        }
        IpcCommand::SaveAs { markdown } => {
            save_as(webview, state, &markdown)?;
        }
        IpcCommand::NewDocument => {
            new_document(webview, state)?;
        }
    }

    Ok(())
}

fn new_document(webview: &WebView, state: &mut AppState) -> Result<()> {
    state.current_path = None;
    state.current_encoding = DocumentEncoding::Utf8;
    let markdown = starter_markdown();
    let html = render_markdown(markdown, None);
    call_js(
        webview,
        "window.app.loadDocument",
        &[json_arg("")?, json_arg(markdown)?, json_arg(&html)?],
    )
}

fn open_path(webview: &WebView, state: &mut AppState, path: PathBuf) -> Result<()> {
    if !is_markdown_path(&path) {
        anyhow::bail!("only .md/.markdown files are supported in this first version");
    }

    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let (markdown, encoding) =
        decode_document(&bytes).with_context(|| format!("failed to decode {}", path.display()))?;
    let display_path = path.to_string_lossy().to_string();
    let rendered = render_markdown(&markdown, path.parent());
    state.current_path = Some(path);
    state.current_encoding = encoding;
    call_js(
        webview,
        "window.app.loadDocument",
        &[
            json_arg(&display_path)?,
            json_arg(&markdown)?,
            json_arg(&rendered)?,
        ],
    )
}

fn save_as(webview: &WebView, state: &mut AppState, markdown: &str) -> Result<()> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Markdown", &["md"])
        .set_file_name("untitled.md")
        .save_file()
    else {
        return Ok(());
    };

    save_to_path(webview, state, path, markdown, DocumentEncoding::Utf8)
}

fn save_to_path(
    webview: &WebView,
    state: &mut AppState,
    path: PathBuf,
    markdown: &str,
    encoding: DocumentEncoding,
) -> Result<()> {
    let bytes = encode_document(markdown, encoding)?;
    fs::write(&path, bytes).with_context(|| format!("failed to save {}", path.display()))?;
    let display_path = path.to_string_lossy().to_string();
    state.current_path = Some(path);
    state.current_encoding = encoding;
    call_js(webview, "window.app.markSaved", &[json_arg(&display_path)?])
}

fn render_markdown(markdown: &str, base_dir: Option<&Path>) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    let parser = Parser::new_ext(markdown, options);
    let mut rendered = String::new();
    html::push_html(&mut rendered, parser);
    let rendered = rewrite_image_srcs(&rendered, base_dir);
    if let Ok(dump_path) = std::env::var("MD_DUMP_HTML") {
        let _ = std::fs::write(&dump_path, &rendered);
    }
    rendered
}

/// The WebView is created with `with_html()` and has no real base URL, so a
/// relative `<img src="pic.png">` would resolve against `about:blank` and fail.
/// Rewrite every image `src` to an absolute `file:///` URL resolved against the
/// markdown document's directory (when one is known).
fn rewrite_image_srcs(html: &str, base_dir: Option<&Path>) -> String {
    let mut out = String::with_capacity(html.len() + 64);
    let mut rest = html;
    while let Some(img_start) = rest.find("<img") {
        out.push_str(&rest[..img_start]);
        rest = &rest[img_start..];
        let Some(tag_end) = rest.find('>') else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rewrite_img_tag(&rest[..=tag_end], base_dir));
        rest = &rest[tag_end + 1..];
    }
    out.push_str(rest);
    out
}

fn rewrite_img_tag(tag: &str, base_dir: Option<&Path>) -> String {
    let Some(src_start) = tag.find("src=\"") else {
        return tag.to_string();
    };
    let value_start = src_start + "src=\"".len();
    let Some(relative_end) = tag[value_start..].find('"') else {
        return tag.to_string();
    };
    let value_end = value_start + relative_end;
    let original = &tag[value_start..value_end];
    let resolved = resolve_image_src(original, base_dir);
    if resolved == original {
        return tag.to_string();
    }
    let mut out = String::with_capacity(tag.len() + 16);
    out.push_str(&tag[..value_start]);
    out.push_str(&resolved);
    out.push_str(&tag[value_end..]);
    out
}

fn resolve_image_src(src: &str, base_dir: Option<&Path>) -> String {
    let trimmed = src.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("data:")
        || trimmed.starts_with("file:")
        || trimmed.starts_with("//")
        || trimmed.starts_with('#')
    {
        return src.to_string();
    }

    let normalized = percent_decode(trimmed).replace('\\', "/");
    let drive_letter = normalized
        .as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic())
        && normalized.as_bytes().get(1) == Some(&b':');
    let absolute = normalized.starts_with('/') || drive_letter;

    let path = if absolute {
        PathBuf::from(&normalized)
    } else if let Some(base) = base_dir {
        base.join(&normalized)
    } else {
        return src.to_string();
    };

    embed_image_as_data_url(&path).unwrap_or_else(|| src.to_string())
}

/// Read a local image and return it as a `data:` URL so it renders regardless of
/// the WebView's base URL or file-access policy. Returns `None` when the file is
/// missing or too large, leaving the original markdown `src` untouched so the
/// broken state stays visible for debugging.
fn embed_image_as_data_url(path: &Path) -> Option<String> {
    const MAX_BYTES: u64 = 8 * 1024 * 1024;
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let mut url = String::with_capacity(bytes.len() / 3 * 4 + 32);
    url.push_str("data:");
    url.push_str(mime_for_path(path));
    url.push_str(";base64,");
    url.push_str(&to_base64(&bytes));
    Some(url)
}

/// pulldown-cmark percent-encodes non-ASCII characters in link/image
/// destinations (e.g. `机械臂.png` becomes `%E6%9C%BA%E8%87%82.png`), so decode
/// before resolving the path on disk.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("avif") => "image/avif",
        _ => "image/png",
    }
}

fn to_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "mdown"
            )
        })
        .unwrap_or(false)
}

fn call_js(webview: &WebView, function_name: &str, args: &[String]) -> Result<()> {
    let script = format!("{}({});", function_name, args.join(","));
    webview
        .evaluate_script(&script)
        .context("failed to evaluate JavaScript")
}

fn json_arg(value: &str) -> Result<String> {
    serde_json::to_string(value).context("failed to encode JavaScript argument")
}

fn send_error(webview: &WebView, message: &str) {
    if let Ok(message) = json_arg(message) {
        let _ = webview.evaluate_script(&format!("window.app.showError({message});"));
    }
}

fn starter_markdown() -> &'static str {
    "# Untitled\n\nStart writing on the left. The preview updates on the right.\n\n- Open or drag a `.md` file\n- Save with Ctrl+S\n- Switch the theme from the toolbar\n"
}

fn app_html() -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Markdown Renderer</title>
  <style>{}</style>
</head>
<body class="preview-only">
  <div class="app-shell">
    <header class="toolbar">
      <div class="file-meta">
        <strong id="document-title">Untitled</strong>
        <span id="document-path">No file opened</span>
      </div>
      <div class="actions">
        <button id="new-button" type="button">New</button>
        <button id="open-button" type="button">Open</button>
        <button id="save-button" type="button">Save</button>
        <button id="save-as-button" type="button">Save As</button>
        <button id="preview-mode-button" type="button" aria-pressed="true">Edit</button>
        <button id="source-button" type="button">源码</button>
        <select id="theme-select" aria-label="Theme">
          <option value="light">Light</option>
          <option value="sepia">Sepia</option>
          <option value="dark">Dark</option>
        </select>
      </div>
    </header>
    <main class="workspace">
      <section class="pane editor-pane">
        <textarea id="editor" spellcheck="false"></textarea>
      </section>
      <section class="pane preview-pane">
        <article id="preview" class="markdown-body"></article>
      </section>
    </main>
    <div class="source-overlay" id="source-overlay" tabindex="-1">
      <div class="source-panel">
        <div class="source-header">
          <strong>Markdown Source</strong>
          <button id="source-close-button" type="button">Close</button>
        </div>
        <pre class="source-body" id="source-view"></pre>
      </div>
    </div>
    <footer class="statusbar">
      <span id="status">Ready</span>
      <span id="drop-hint">Drop a Markdown file anywhere in the window</span>
    </footer>
  </div>
  <!--MATHJAX-->
  <script>{}</script>
</body>
</html>"#,
        app_css(),
        app_js()
    )
    .replace(
        "<!--MATHJAX-->",
        &format!(
            "<script>window.MathJax = {MATHJAX_CONFIG};</script><script src=\"mathjax/tex-svg.js\"></script>"
        ),
    )
}

/// MathJax config: `$...$` / `\(...\)` inline math, `$$...$$` / `\[...\]` display math.
/// SVG output keeps the formulas theme-aware (currentColor) and needs no font files.
const MATHJAX_CONFIG: &str = r#"{ tex: { inlineMath: [['$','$'], ['\\(','\\)']], displayMath: [['$$','$$'], ['\\[','\\]']] }, svg: { fontCache: 'global' } }"#;

/// Bundled MathJax v3 (tex input + SVG output), embedded so rendering works fully offline.
const MATHJAX_SRC: &str = include_str!("mathjax_tex_svg.js");

fn app_css() -> &'static str {
    r#"
:root {
  color-scheme: light;
  --bg: #f6f7f9;
  --panel: #ffffff;
  --panel-soft: #eef1f5;
  --text: #20242c;
  --muted: #657184;
  --border: #d9dee7;
  --accent: #2563eb;
  --code-bg: #f0f3f7;
  --shadow: rgba(15, 23, 42, 0.08);
  --editor-font: Consolas, 'Cascadia Mono', 'SFMono-Regular', monospace;
  --body-font: 'Segoe UI', system-ui, sans-serif;
  --preview-zoom: 1;
}

body.theme-dark {
  color-scheme: dark;
  --bg: #17191f;
  --panel: #20242b;
  --panel-soft: #2a3039;
  --text: #e7eaf0;
  --muted: #9aa4b2;
  --border: #343b46;
  --accent: #7aa2ff;
  --code-bg: #15181e;
  --shadow: rgba(0, 0, 0, 0.22);
}

body.theme-sepia {
  --bg: #f4efe6;
  --panel: #fffaf1;
  --panel-soft: #ece2d1;
  --text: #2d2820;
  --muted: #756a5a;
  --border: #d9ccb8;
  --accent: #9a5b22;
  --code-bg: #eee4d3;
}

* {
  box-sizing: border-box;
}

html,
body {
  height: 100%;
  margin: 0;
}

body {
  background: var(--bg);
  color: var(--text);
  font-family: var(--body-font);
}

button,
select {
  font: inherit;
}

.app-shell {
  display: grid;
  grid-template-rows: 54px minmax(0, 1fr) 28px;
  height: 100vh;
}

.toolbar,
.statusbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: var(--panel);
  border-color: var(--border);
  box-shadow: 0 1px 10px var(--shadow);
}

.toolbar {
  padding: 0 14px;
  border-bottom: 1px solid var(--border);
}

.statusbar {
  padding: 0 12px;
  color: var(--muted);
  border-top: 1px solid var(--border);
  font-size: 12px;
}

.file-meta {
  min-width: 0;
}

#document-title {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 45vw;
}

#document-path {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--muted);
  font-size: 12px;
  max-width: 45vw;
}

.actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.actions button,
.actions select {
  height: 32px;
  padding: 0 10px;
  color: var(--text);
  background: var(--panel-soft);
  border: 1px solid var(--border);
  border-radius: 6px;
}

.actions button:hover,
.actions select:hover {
  border-color: var(--accent);
}

.actions button[aria-pressed="true"] {
  color: #ffffff;
  background: var(--accent);
  border-color: var(--accent);
}

.workspace {
  display: grid;
  grid-template-columns: minmax(320px, 1fr) minmax(320px, 1fr);
  min-height: 0;
}

.pane {
  min-width: 0;
  min-height: 0;
}

.editor-pane {
  border-right: 1px solid var(--border);
}

body.preview-only .workspace {
  grid-template-columns: 1fr;
}

body.preview-only .editor-pane {
  display: none;
}

body.preview-only .markdown-body {
  max-width: 980px;
  padding-top: 34px;
}

#editor {
  width: 100%;
  height: 100%;
  padding: 28px;
  resize: none;
  outline: none;
  border: 0;
  background: var(--panel);
  color: var(--text);
  font-family: var(--editor-font);
  font-size: 15px;
  line-height: 1.65;
  tab-size: 2;
}

.preview-pane {
  overflow: auto;
  background: var(--panel);
}

.markdown-body {
  max-width: 860px;
  margin: 0 auto;
  padding: 28px 36px 80px;
  line-height: 1.68;
  font-size: 16px;
  zoom: var(--preview-zoom);
}

.markdown-body h1,
.markdown-body h2,
.markdown-body h3 {
  line-height: 1.25;
  margin: 1.35em 0 0.55em;
}

.markdown-body h1 {
  padding-bottom: 0.28em;
  border-bottom: 1px solid var(--border);
  font-size: 2rem;
}

.markdown-body h2 {
  padding-bottom: 0.22em;
  border-bottom: 1px solid var(--border);
  font-size: 1.45rem;
}

.markdown-body p,
.markdown-body ul,
.markdown-body ol,
.markdown-body blockquote,
.markdown-body pre,
.markdown-body table {
  margin: 0.85em 0;
}

.markdown-body code {
  padding: 0.15em 0.35em;
  background: var(--code-bg);
  border-radius: 4px;
  font-family: var(--editor-font);
  font-size: 0.9em;
}

.markdown-body pre {
  overflow: auto;
  padding: 14px 16px;
  background: var(--code-bg);
  border: 1px solid var(--border);
  border-radius: 7px;
}

.markdown-body pre code {
  padding: 0;
  background: transparent;
}

.markdown-body blockquote {
  padding: 0.15em 1em;
  color: var(--muted);
  border-left: 4px solid var(--border);
}

.markdown-body table {
  width: 100%;
  border-collapse: collapse;
}

.markdown-body th,
.markdown-body td {
  padding: 8px 10px;
  border: 1px solid var(--border);
}

.markdown-body th {
  background: var(--panel-soft);
}

.markdown-body img {
  max-width: 100%;
}

.source-overlay {
  position: fixed;
  inset: 0;
  z-index: 10;
  display: none;
  align-items: stretch;
  justify-content: center;
  padding: 44px;
  background: rgba(0, 0, 0, 0.45);
}

body.source-open .source-overlay {
  display: flex;
}

.source-panel {
  display: flex;
  flex-direction: column;
  width: min(900px, 100%);
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 10px;
  box-shadow: 0 18px 50px rgba(0, 0, 0, 0.35);
  overflow: hidden;
}

.source-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
}

.source-header strong {
  font-size: 13px;
  color: var(--muted);
  text-transform: uppercase;
  letter-spacing: 0.06em;
}

.source-header button {
  height: 28px;
  padding: 0 12px;
  color: var(--text);
  background: var(--panel-soft);
  border: 1px solid var(--border);
  border-radius: 6px;
}

.source-header button:hover {
  border-color: var(--accent);
}

.source-body {
  flex: 1;
  margin: 0;
  padding: 16px 18px;
  overflow: auto;
  background: var(--code-bg);
  color: var(--text);
  font-family: var(--editor-font);
  font-size: 13.5px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
}

.markdown-body a {
  color: var(--accent);
}

.dirty #document-title::after {
  content: ' *';
  color: var(--accent);
}

@media (max-width: 780px) {
  .app-shell {
    grid-template-rows: auto minmax(0, 1fr) 28px;
  }

  .toolbar {
    align-items: stretch;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
  }

  .actions {
    flex-wrap: wrap;
  }

  .workspace {
    grid-template-columns: 1fr;
    grid-template-rows: minmax(260px, 1fr) minmax(260px, 1fr);
  }

  .editor-pane {
    border-right: 0;
    border-bottom: 1px solid var(--border);
  }
}
"#
}

fn app_js() -> &'static str {
    r#"
const editor = document.getElementById('editor');
const preview = document.getElementById('preview');
const statusLabel = document.getElementById('status');
const titleLabel = document.getElementById('document-title');
const pathLabel = document.getElementById('document-path');
const themeSelect = document.getElementById('theme-select');
const previewModeButton = document.getElementById('preview-mode-button');

let currentPath = '';
let renderTimer = 0;
let dirty = false;
let previewOnly = true;
let previewZoom = 1;

function post(message) {
  window.ipc.postMessage(JSON.stringify(message));
}

function setDirty(value) {
  dirty = value;
  document.body.classList.toggle('dirty', dirty);
}

function fileName(path) {
  if (!path) return 'Untitled';
  const normalized = path.replaceAll('\\', '/');
  return normalized.slice(normalized.lastIndexOf('/') + 1) || path;
}

function queueRender() {
  window.clearTimeout(renderTimer);
  renderTimer = window.setTimeout(() => {
    post({ cmd: 'render', markdown: editor.value });
  }, 120);
}

function typesetMath() {
  if (window.MathJax && MathJax.typesetPromise) {
    MathJax.typesetPromise([preview]).catch(() => {});
  }
}

function setPreviewOnly(value) {
  previewOnly = value;
  document.body.classList.toggle('preview-only', previewOnly);
  previewModeButton.setAttribute('aria-pressed', String(previewOnly));
  previewModeButton.textContent = previewOnly ? 'Edit' : 'Preview';
  statusLabel.textContent = previewOnly ? 'Preview only' : (dirty ? 'Editing' : 'Ready');
  if (!previewOnly) {
    editor.focus();
  }
}

function setPreviewZoom(value) {
  previewZoom = Math.min(2.5, Math.max(0.5, value));
  document.documentElement.style.setProperty('--preview-zoom', previewZoom.toFixed(2));
  localStorage.setItem('md-preview-zoom', String(previewZoom));
  statusLabel.textContent = `Preview zoom ${Math.round(previewZoom * 100)}%`;
}

function changePreviewZoom(delta) {
  const nextZoom = Math.round((previewZoom + delta) * 100) / 100;
  setPreviewZoom(nextZoom);
}

editor.addEventListener('input', () => {
  setDirty(true);
  statusLabel.textContent = 'Editing';
  queueRender();
});

editor.addEventListener('scroll', () => {
  const ratio = editor.scrollTop / Math.max(1, editor.scrollHeight - editor.clientHeight);
  const previewPane = document.querySelector('.preview-pane');
  previewPane.scrollTop = ratio * Math.max(1, previewPane.scrollHeight - previewPane.clientHeight);
});

document.getElementById('new-button').addEventListener('click', () => {
  post({ cmd: 'new_document' });
});

document.getElementById('open-button').addEventListener('click', () => {
  post({ cmd: 'open' });
});

document.getElementById('save-button').addEventListener('click', () => {
  post({ cmd: 'save', markdown: editor.value });
});

document.getElementById('save-as-button').addEventListener('click', () => {
  post({ cmd: 'save_as', markdown: editor.value });
});

previewModeButton.addEventListener('click', () => {
  setPreviewOnly(!previewOnly);
});

themeSelect.addEventListener('change', () => {
  document.body.classList.remove('theme-light', 'theme-dark', 'theme-sepia');
  document.body.classList.add(`theme-${themeSelect.value}`);
  localStorage.setItem('md-theme', themeSelect.value);
});

const sourceOverlay = document.getElementById('source-overlay');
const sourceView = document.getElementById('source-view');

function openSource() {
  sourceView.textContent = editor.value;
  document.body.classList.add('source-open');
}

function closeSource() {
  document.body.classList.remove('source-open');
}

document.getElementById('source-button').addEventListener('click', openSource);

document.getElementById('source-close-button').addEventListener('click', closeSource);

sourceOverlay.addEventListener('click', (event) => {
  if (event.target === sourceOverlay) {
    closeSource();
  }
});

document.addEventListener('keydown', (event) => {
  const hasModifier = event.ctrlKey || event.metaKey;
  const key = event.key.toLowerCase();

  if (event.key === 'Escape') {
    closeSource();
    return;
  }

  if (hasModifier && (event.key === '+' || event.key === '=')) {
    event.preventDefault();
    changePreviewZoom(0.1);
    return;
  }
  if (hasModifier && (event.key === '-' || event.key === '_')) {
    event.preventDefault();
    changePreviewZoom(-0.1);
    return;
  }
  if (hasModifier && event.key === '0') {
    event.preventDefault();
    setPreviewZoom(1);
    return;
  }

  if (hasModifier && key === 's') {
    event.preventDefault();
    post({ cmd: 'save', markdown: editor.value });
  }
  if (hasModifier && key === 'o') {
    event.preventDefault();
    post({ cmd: 'open' });
  }
  if (hasModifier && event.shiftKey && key === 'p') {
    event.preventDefault();
    setPreviewOnly(!previewOnly);
  }
});

window.addEventListener('dragover', (event) => {
  event.preventDefault();
  statusLabel.textContent = 'Drop to open Markdown';
});

window.addEventListener('dragleave', () => {
  statusLabel.textContent = dirty ? 'Editing' : 'Ready';
});

window.app = {
  setPreview(html) {
    preview.innerHTML = html;
    typesetMath();
  },
  loadDocument(path, markdown, html) {
    currentPath = path;
    editor.value = markdown;
    preview.innerHTML = html;
    typesetMath();
    titleLabel.textContent = fileName(path);
    pathLabel.textContent = path || 'No file opened';
    setDirty(false);
    statusLabel.textContent = path ? 'Opened' : 'New document';
    if (!previewOnly) {
      editor.focus();
    }
  },
  markSaved(path) {
    currentPath = path;
    titleLabel.textContent = fileName(path);
    pathLabel.textContent = path || 'No file opened';
    setDirty(false);
    statusLabel.textContent = 'Saved';
  },
  showError(message) {
    statusLabel.textContent = message;
  }
};

const savedTheme = localStorage.getItem('md-theme') || 'light';
themeSelect.value = savedTheme;
document.body.classList.add(`theme-${savedTheme}`);

const savedZoom = Number(localStorage.getItem('md-preview-zoom') || '1');
setPreviewZoom(Number.isFinite(savedZoom) ? savedZoom : 1);

setPreviewOnly(true);
// Rust chooses the startup file or welcome document after this handshake.
// Do not queue a default render here: it could overwrite an opened file.
post({ cmd: 'ready' });
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_and_bom_round_trip() {
        for (bytes, expected) in [
            ("中文".as_bytes().to_vec(), DocumentEncoding::Utf8),
            (
                [&[0xef, 0xbb, 0xbf][..], "中文".as_bytes()].concat(),
                DocumentEncoding::Utf8Bom,
            ),
        ] {
            let (text, encoding) = decode_document(&bytes).unwrap();
            assert_eq!(text, "中文");
            assert_eq!(encoding, expected);
            assert_eq!(encode_document(&text, encoding).unwrap(), bytes);
        }
    }

    #[cfg(windows)]
    #[test]
    fn gbk_round_trip_and_rejects_unrepresentable_edits() {
        let bytes = [0xd6, 0xd0, 0xce, 0xc4];
        let (text, encoding) = decode_document(&bytes).unwrap();
        assert_eq!(text, "中文");
        assert_eq!(encoding, DocumentEncoding::Gbk);
        assert_eq!(encode_document(&text, encoding).unwrap(), bytes);
        assert!(encode_document("😀", encoding).is_err());
    }
    #[test]
    fn startup_file_waits_for_ready_and_is_consumed_once() {
        let path = PathBuf::from("C:/资料/有 空格的文件.md");
        let mut state = AppState {
            pending_open: Some(path.clone()),
            ..AppState::default()
        };
        assert!(!state.ui_ready);
        assert_eq!(state.on_ready(), Some(StartupAction::Open(path)));
        assert!(state.pending_open.is_none());
        assert_eq!(state.on_ready(), None);
    }

    #[test]
    fn startup_without_file_creates_welcome_only_once() {
        let mut state = AppState::default();
        assert_eq!(state.on_ready(), Some(StartupAction::NewDocument));
        assert_eq!(state.on_ready(), None);
    }

    #[test]
    fn frontend_bootstrap_sends_ready_not_a_default_document() {
        assert!(matches!(
            serde_json::from_str::<IpcCommand>(r#"{"cmd":"ready"}"#).unwrap(),
            IpcCommand::Ready
        ));
        let html = app_html();
        assert!(html.contains("post({ cmd: 'ready' });"));
        assert!(!html.contains("window.app.loadDocument('', `# Untitled"));
    }

    #[test]
    fn embeds_local_images_as_data_urls() {
        let dir = std::env::temp_dir().join(format!("md_typora_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = [0x89u8, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        std::fs::write(dir.join("pic.png"), png).unwrap();
        let html = r#"<p><img src="pic.png" alt="图" /></p>"#;
        let out = rewrite_image_srcs(html, Some(&dir));
        assert!(
            out.starts_with(r#"<p><img src="data:image/png;base64,"#),
            "{out}"
        );
        assert!(out.contains("iVBORw0KGgo="), "{out}");
        assert!(out.ends_with(r#"" alt="图" /></p>"#), "{out}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn embeds_absolute_windows_paths() {
        let dir = std::env::temp_dir().join(format!("md_typora_test_abs_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), [1u8, 2, 3, 4, 5]).unwrap();
        let abs = dir.join("a.png").to_string_lossy().replace('/', "\\");
        let out = rewrite_image_srcs(&format!(r#"<img src="{abs}" />"#), None);
        assert!(
            out.starts_with(r#"<img src="data:image/png;base64,"#),
            "{out}"
        );
        assert!(out.contains("AQIDBAU="), "{out}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_keeps_original_src() {
        let base = Path::new("C:/does/not/exist");
        let html = r#"<img src="nope.png" />"#;
        assert_eq!(rewrite_image_srcs(html, Some(base)), html);
    }

    #[test]
    fn keeps_remote_and_data_srcs() {
        let html = r#"<p><img src="https://example.com/a.png" /><img src="data:image/png;base64,AAA" /></p>"#;
        let out = rewrite_image_srcs(html, Some(Path::new("C:/base")));
        assert_eq!(out, html);
    }

    #[test]
    fn no_base_keeps_src() {
        let html = r#"<img src="a.png" />"#;
        assert_eq!(rewrite_image_srcs(html, None), html);
    }

    #[test]
    fn decodes_percent_encoded_names() {
        let dir = std::env::temp_dir().join(format!("md_typora_test_pct_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("示意图.png"), [1u8, 2, 3]).unwrap();
        // pulldown-cmark percent-encodes destinations: 示意图.png -> %E7%A4%BA%E6%84%8F%E5%9B%BE.png
        let html = r#"<img src="%E7%A4%BA%E6%84%8F%E5%9B%BE.png" />"#;
        let out = rewrite_image_srcs(html, Some(&dir));
        assert!(
            out.starts_with(r#"<img src="data:image/png;base64,"#),
            "{out}"
        );
        assert!(out.ends_with(r#"" />"#), "{out}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
