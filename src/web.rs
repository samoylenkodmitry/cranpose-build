//! The web platform's page: served from a directory, and opened in headless
//! Chrome to record the pipelines its first screen draws with. A release build
//! embeds that record (`CRANPOSE_FIRST_SCREEN_PIPELINES`), and on WebGL the
//! page then builds those pipelines as it starts, so the browser compiles them
//! while the page composes its first screen.
use crate::{Cancellation, tools};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, ErrorKind, Write},
    net::{TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

/// A directory served over HTTP on a free port of this computer until dropped.
pub struct Site {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Site {
    pub fn serve(root: &Path) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let root = root.to_owned();
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let root = root.clone();
                        thread::spawn(move || {
                            let _ = respond(stream, &root);
                        });
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            port,
            stop,
            thread: Some(thread),
        })
    }

    /// The address of `page`, a path relative to the served directory.
    pub fn url(&self, page: &str) -> String {
        format!(
            "http://127.0.0.1:{}/{}",
            self.port,
            page.trim_start_matches('/')
        )
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn respond(stream: TcpStream, root: &Path) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
    }
    let target = request.split_whitespace().nth(1).unwrap_or("/");
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let file = served_file(root, path);
    let mut stream = stream;
    match file.and_then(|file| fs::read(&file).ok().map(|body| (file, body))) {
        Some((file, body)) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                content_type(&file),
                body.len()
            )?;
            stream.write_all(&body)?;
        }
        None => stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?,
    }
    stream.flush()?;
    Ok(())
}

/// The file under `root` that the request path `path` names; `None` for one
/// that leaves `root`. A directory serves its `index.html`.
pub fn served_file(root: &Path, path: &str) -> Option<PathBuf> {
    let relative = PathBuf::from(percent_decode(path.trim_start_matches('/'))?);
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return None;
    }
    let file = root.join(relative);
    if file.is_dir() {
        Some(file.join("index.html"))
    } else {
        Some(file)
    }
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn content_type(file: &Path) -> &'static str {
    match file.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("css") => "text/css",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Chrome or Chromium: `CHROME` when set, else a usual installation.
pub fn find_chrome() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CHROME").map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    let mut candidates = Vec::new();
    if cfg!(target_os = "macos") {
        for root in [Some(PathBuf::from("/")), tools::home()]
            .into_iter()
            .flatten()
        {
            candidates
                .push(root.join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome"));
            candidates.push(root.join("Applications/Chromium.app/Contents/MacOS/Chromium"));
        }
    }
    if cfg!(windows) {
        for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(variable).map(PathBuf::from) {
                candidates.push(root.join("Google/Chrome/Application/chrome.exe"));
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file()).or_else(|| {
        [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
        ]
        .into_iter()
        .find_map(tools::find)
    })
}

/// A viewport a first screen is recorded at.
struct Viewport {
    width: u32,
    height: u32,
    scale: f64,
    mobile: bool,
    user_agent: Option<&'static str>,
}

/// A phone's viewport, with its user agent, and a desktop window's.
const VIEWPORTS: [Viewport; 2] = [
    Viewport {
        width: 412,
        height: 915,
        scale: 2.625,
        mobile: true,
        user_agent: Some(
            "Mozilla/5.0 (Linux; Android 14; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36",
        ),
    },
    Viewport {
        width: 1280,
        height: 800,
        scale: 2.0,
        mobile: false,
        user_agent: None,
    },
];

/// How long a page may take to record its first screen.
const RECORD_TIMEOUT: Duration = Duration::from_secs(60);

/// Opens `page` of the site in `root` in headless Chrome at a phone's and a
/// desktop's viewport, on WebGL and on the backend the page picks itself,
/// and returns the first screens it recorded, one set after another, as the
/// page's build embeds them. A run that records what an earlier one did, as
/// one on a Chrome without WebGPU does, adds nothing.
pub fn record_first_screen(
    root: &Path,
    page: &str,
    cancel: &Cancellation,
    mut log: impl FnMut(&str),
) -> Result<Vec<u8>> {
    let chrome =
        find_chrome().context("Chrome or Chromium is missing; set CHROME to its executable")?;
    let site = Site::serve(root)?;
    let own = site.url(page);
    let gl = format!("{own}?backend=gl");
    let mut sets: Vec<Vec<u8>> = Vec::new();
    for viewport in &VIEWPORTS {
        for (url, backend) in [(gl.as_str(), "WebGL"), (own.as_str(), "its own backend")] {
            cancel.check()?;
            let mut browser = Browser::launch(&chrome, viewport)?;
            let set = browser.record(url, viewport, cancel)?;
            let recorded = sets.contains(&set);
            log(&format!(
                "First screen at {}x{} on {backend}: {} bytes of pipeline records{}",
                viewport.width,
                viewport.height,
                set.len(),
                if recorded { ", recorded already" } else { "" }
            ));
            if !recorded {
                sets.push(set);
            }
        }
    }
    Ok(sets.concat())
}

struct Browser {
    child: Child,
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    _profile: tempfile::TempDir,
}

impl Browser {
    fn launch(chrome: &Path, viewport: &Viewport) -> Result<Self> {
        let profile = tempfile::Builder::new()
            .prefix("cranpose-chrome-")
            .tempdir()?;
        let mut child = Command::new(chrome)
            .args([
                "--headless=new",
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                // Without these Chrome on a Linux machine waits for a desktop
                // keyring and never loads a page.
                "--password-store=basic",
                "--use-mock-keychain",
            ])
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .arg(format!(
                "--window-size={},{}",
                viewport.width, viewport.height
            ))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Start {}", chrome.display()))?;
        let socket = match page_socket(&mut child, profile.path()) {
            Ok(socket) => socket,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.context("Chrome did not answer on its debugging port"));
            }
        };
        let mut browser = Self {
            child,
            socket,
            next_id: 0,
            _profile: profile,
        };
        browser.call("Page.enable", json!({}))?;
        browser.call("Runtime.enable", json!({}))?;
        Ok(browser)
    }

    /// Sends one DevTools command and returns its result.
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.socket.send(Message::text(
            json!({ "id": id, "method": method, "params": params }).to_string(),
        ))?;
        loop {
            let Message::Text(text) = self.socket.read()? else {
                continue;
            };
            let message: Value = serde_json::from_str(text.as_str())?;
            if message["id"].as_u64() != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                bail!("{method}: {error}");
            }
            return Ok(message["result"].clone());
        }
    }

    fn evaluate(&mut self, expression: &str) -> Result<Value> {
        let result = self.call(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
        )?;
        ensure!(
            result.get("exceptionDetails").is_none(),
            "The page threw: {}",
            result["exceptionDetails"]
        );
        Ok(result["result"]["value"].clone())
    }

    fn record(&mut self, url: &str, viewport: &Viewport, cancel: &Cancellation) -> Result<Vec<u8>> {
        self.call(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": viewport.width,
                "height": viewport.height,
                "deviceScaleFactor": viewport.scale,
                "mobile": viewport.mobile,
            }),
        )?;
        if let Some(user_agent) = viewport.user_agent {
            self.call(
                "Emulation.setUserAgentOverride",
                json!({ "userAgent": user_agent }),
            )?;
        }
        self.call(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": "globalThis.__cranposeRecordFirstScreen = true;" }),
        )?;
        self.call("Page.navigate", json!({ "url": url }))?;
        let started = Instant::now();
        while self.evaluate("globalThis.__cranposeFirstScreenPipelines !== undefined")?
            != Value::Bool(true)
        {
            cancel.check()?;
            ensure!(
                started.elapsed() < RECORD_TIMEOUT,
                "The page left no first screen within {} s; is it a Cranpose page?",
                RECORD_TIMEOUT.as_secs()
            );
            thread::sleep(Duration::from_millis(100));
        }
        let bytes = self.evaluate("Array.from(globalThis.__cranposeFirstScreenPipelines)")?;
        bytes
            .as_array()
            .context("The page's first screen is not a byte array")?
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|byte| u8::try_from(byte).ok())
                    .context("The page's first screen holds a value that is not a byte")
            })
            .collect()
    }
}

/// The DevTools socket of the page Chrome `child` opened, once its profile
/// names the debugging port.
fn page_socket(child: &mut Child, profile: &Path) -> Result<WebSocket<MaybeTlsStream<TcpStream>>> {
    let started = Instant::now();
    let page = loop {
        if let Some(status) = child.try_wait()? {
            bail!("Chrome exited with {status}");
        }
        let port = fs::read_to_string(profile.join("DevToolsActivePort"))
            .ok()
            .and_then(|text| {
                text.lines()
                    .next()
                    .and_then(|line| line.parse::<u16>().ok())
            });
        if let Some(port) = port
            && let Ok(mut response) =
                ureq::get(&format!("http://127.0.0.1:{port}/json/list")).call()
            && let Ok(targets) = response.body_mut().read_json::<Vec<Value>>()
            && let Some(url) = targets
                .iter()
                .find(|target| target["type"] == "page")
                .and_then(|target| target["webSocketDebuggerUrl"].as_str())
        {
            break url.to_owned();
        }
        ensure!(
            started.elapsed() < Duration::from_secs(20),
            "no page within 20 s"
        );
        thread::sleep(Duration::from_millis(100));
    };
    let (socket, _) = tungstenite::connect(page.as_str())?;
    if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    }
    Ok(socket)
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
