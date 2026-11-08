//! HTML to PDF conversion with headless Chromium.
//!
//! Chromium is the same engine that displays the HTML preview, so the PDF keeps the
//! browser's layout (flexbox, grid, web fonts, `@page`), writes a correct Unicode text
//! layer, emits link annotations and produces a tagged PDF. The browser runs with an
//! unreachable proxy and a resolver that answers nothing: rendered documents are
//! self-contained, and a template can never make the server fetch remote URLs.

use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::error::{AppError, Result};

pub const MIME_TYPE: &str = "application/pdf";
pub const BROWSER_ENV: &str = "DOCUMENT_TEMPLATING_MCP_CHROMIUM";
pub const NO_SANDBOX_ENV: &str = "DOCUMENT_TEMPLATING_MCP_CHROMIUM_NO_SANDBOX";
const RENDER_TIMEOUT: Duration = Duration::from_secs(60);

/// Converts a rendered HTML file into validated PDF bytes.
pub trait PdfRenderer {
    fn render(&self, html_path: &Path) -> Result<Vec<u8>>;
}

pub struct ChromiumRenderer {
    browser: PathBuf,
    no_sandbox: bool,
    timeout: Duration,
}

impl ChromiumRenderer {
    /// Uses `DOCUMENT_TEMPLATING_MCP_CHROMIUM` when set, otherwise the first installed
    /// Chromium, Chrome or Edge, otherwise `chromium` on PATH.
    pub fn discover() -> Self {
        Self::with_browser(select_browser(env::var_os(BROWSER_ENV), &candidates()))
    }

    pub fn with_browser(browser: impl Into<PathBuf>) -> Self {
        Self {
            browser: browser.into(),
            no_sandbox: env::var(NO_SANDBOX_ENV).is_ok_and(|value| value == "1" || value == "true"),
            timeout: RENDER_TIMEOUT,
        }
    }

    /// True when the configured browser can be launched.
    pub fn is_available(&self) -> bool {
        self.browser.is_file() || which(&self.browser).is_some()
    }
}

fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if cfg!(windows) {
        for base in [
            env::var_os("ProgramFiles"),
            env::var_os("ProgramFiles(x86)"),
        ]
        .into_iter()
        .flatten()
        {
            let base = PathBuf::from(base);
            paths.push(base.join("Google/Chrome/Application/chrome.exe"));
            paths.push(base.join("Chromium/Application/chrome.exe"));
            paths.push(base.join("Microsoft/Edge/Application/msedge.exe"));
        }
    } else if cfg!(target_os = "macos") {
        paths.push("/Applications/Chromium.app/Contents/MacOS/Chromium".into());
        paths.push("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
    } else {
        for name in [
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/google-chrome",
        ] {
            paths.push(name.into());
        }
    }
    paths
}

fn select_browser(configured: Option<OsString>, candidates: &[PathBuf]) -> PathBuf {
    if let Some(value) = configured.filter(|value| !value.is_empty()) {
        return PathBuf::from(value);
    }
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .unwrap_or_else(|| PathBuf::from("chromium"))
}

pub(crate) fn which(program: &Path) -> Option<PathBuf> {
    if program.components().count() > 1 {
        return None;
    }
    let extensions: &[&str] = if cfg!(windows) { &["", ".exe"] } else { &[""] };
    env::split_paths(&env::var_os("PATH")?).find_map(|directory| {
        extensions.iter().find_map(|extension| {
            let mut candidate = directory.join(program).into_os_string();
            candidate.push(extension);
            let candidate = PathBuf::from(candidate);
            candidate.is_file().then_some(candidate)
        })
    })
}

/// Command-line arguments for one print job. Kept separate so tests can pin them.
pub fn chromium_args(
    profile: &Path,
    output: &Path,
    input_url: &str,
    no_sandbox: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "--headless=new",
        "--disable-gpu",
        "--disable-dev-shm-usage",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-extensions",
        "--disable-sync",
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-default-apps",
        "--disable-breakpad",
        "--disable-crash-reporter",
        "--mute-audio",
        "--hide-scrollbars",
        // No network: every request goes to a closed local port, and names never resolve.
        "--proxy-server=socks5://127.0.0.1:9",
        "--proxy-bypass-list=<-loopback>",
        "--host-resolver-rules=MAP * ~NOTFOUND",
        "--no-pdf-header-footer",
        "--print-to-pdf-no-header",
        "--generate-pdf-document-outline",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    if no_sandbox {
        args.push("--no-sandbox".into());
    }
    let mut profile_arg = OsString::from("--user-data-dir=");
    profile_arg.push(profile);
    args.push(profile_arg);
    let mut output_arg = OsString::from("--print-to-pdf=");
    output_arg.push(output);
    args.push(output_arg);
    args.push(input_url.into());
    args
}

/// Builds a `file://` URL for an absolute path, percent-encoding everything unsafe.
pub fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut url = String::from(if text.starts_with('/') {
        "file://"
    } else {
        "file:///"
    });
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            url.push(byte as char);
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

impl PdfRenderer for ChromiumRenderer {
    fn render(&self, html_path: &Path) -> Result<Vec<u8>> {
        let html_path = absolute(html_path)?;
        let temp = tempfile::Builder::new()
            .prefix("pdf-render-")
            .tempdir()
            .map_err(AppError::storage)?;
        let profile = temp.path().join("profile");
        let output_path = temp.path().join("document.pdf");
        let log_path = temp.path().join("chromium.log");
        let mut command = Command::new(&self.browser);
        command.args(chromium_args(
            &profile,
            &output_path,
            &file_url(&html_path),
            self.no_sandbox,
        ));
        let context = format!(
            "PDF renderer: {}\nPDF input: {}\nPDF output: {}",
            self.browser.display(),
            html_path.display(),
            output_path.display()
        );
        let started = Instant::now();
        let status = run_with_timeout(&mut command, &log_path, self.timeout).map_err(|error| {
            AppError::storage(format!(
                "{error}\n{context}\nSet {BROWSER_ENV} to a Chromium, Chrome or Edge executable"
            ))
        })?;
        if status.success() {
            // Chrome on Windows hands the job to a child process and exits before the
            // PDF is written; wait for a complete file within the same time budget.
            wait_for_complete_pdf(&output_path, self.timeout.saturating_sub(started.elapsed()));
        }
        let log = fs::read_to_string(&log_path).unwrap_or_default();
        validate_output(
            status,
            &output_path,
            &format!("{context}\nPDF log: {}", tail(&log)),
        )
    }
}

/// Returns once `path` holds a PDF that ends with `%%EOF` and has stopped growing.
fn wait_for_complete_pdf(path: &Path, budget: Duration) {
    let deadline = Instant::now() + budget;
    let mut last_size = None;
    while Instant::now() < deadline {
        if let Ok(bytes) = fs::read(path) {
            let tail = &bytes[bytes.len().saturating_sub(1024)..];
            let complete = tail.windows(5).any(|window| window == b"%%EOF");
            if complete && last_size == Some(bytes.len()) {
                return;
            }
            last_size = Some(bytes.len());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir().map_err(AppError::storage)?.join(path))
    }
}

fn tail(text: &str) -> &str {
    let text = text.trim();
    let start = text.len().saturating_sub(2000);
    let start = (start..text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(0);
    &text[start..]
}

/// Runs a command with stdout discarded and stderr captured to `log_path`, killing it
/// when it outlives `timeout`.
pub(crate) fn run_with_timeout(
    command: &mut Command,
    log_path: &Path,
    timeout: Duration,
) -> std::result::Result<ExitStatus, String> {
    let log = File::create(log_path).map_err(|error| format!("failed to create log: {error}"))?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start {:?}: {error}", command.get_program()))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{:?} timed out after {timeout:?}",
                    command.get_program()
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => return Err(format!("failed to wait for process: {error}")),
        }
    }
}

fn validate_output(status: ExitStatus, output_path: &Path, context: &str) -> Result<Vec<u8>> {
    let diagnostics = format!("{context}\nPDF exit: {status}");
    if !status.success() {
        return Err(AppError::storage(format!("Chromium failed\n{diagnostics}")));
    }
    let bytes = fs::read(output_path).map_err(|error| {
        AppError::storage(format!("failed to read PDF output: {error}\n{diagnostics}"))
    })?;
    if !bytes.starts_with(b"%PDF-") {
        return Err(AppError::storage(format!(
            "Chromium completed without producing a valid PDF\n{diagnostics}"
        )));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_override_then_installed_then_path() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("chrome");
        let missing = dir.path().join("missing");
        assert_eq!(
            select_browser(None, std::slice::from_ref(&missing)),
            PathBuf::from("chromium")
        );
        fs::write(&installed, b"browser").unwrap();
        assert_eq!(
            select_browser(None, &[missing, installed.clone()]),
            installed
        );
        assert_eq!(
            select_browser(Some("custom".into()), &[installed]),
            PathBuf::from("custom")
        );
    }

    #[test]
    fn print_arguments_disable_network_and_headers() {
        let args: Vec<String> = chromium_args(
            Path::new("/tmp/profile"),
            Path::new("/tmp/out.pdf"),
            "file:///tmp/in.html",
            true,
        )
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
        for expected in [
            "--headless=new",
            "--no-pdf-header-footer",
            "--generate-pdf-document-outline",
            "--proxy-server=socks5://127.0.0.1:9",
            "--proxy-bypass-list=<-loopback>",
            "--host-resolver-rules=MAP * ~NOTFOUND",
            "--no-sandbox",
            "--user-data-dir=/tmp/profile",
            "--print-to-pdf=/tmp/out.pdf",
        ] {
            assert!(args.iter().any(|arg| arg == expected), "missing {expected}");
        }
        assert_eq!(args.last().unwrap(), "file:///tmp/in.html");
        assert!(!args.iter().any(|arg| arg.starts_with("--zoom")));
    }

    #[test]
    fn file_urls_are_absolute_and_encoded() {
        assert_eq!(
            file_url(Path::new("/data/renders/my cv.html")),
            "file:///data/renders/my%20cv.html"
        );
        assert_eq!(
            file_url(Path::new(r"C:\data\renders\cv.html")),
            "file:///C:/data/renders/cv.html"
        );
    }

    #[test]
    fn missing_browser_reports_paths_and_remedy() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.html");
        fs::write(&input, "<p>test</p>").unwrap();
        let renderer = ChromiumRenderer::with_browser(dir.path().join("missing-browser"));
        assert!(!renderer.is_available());
        let error = renderer.render(&input).unwrap_err().to_string();
        assert!(error.contains("PDF renderer:"));
        assert!(error.contains("PDF input:"));
        assert!(error.contains(BROWSER_ENV));
    }

    #[test]
    fn rejects_failed_or_invalid_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.pdf");
        let log = dir.path().join("log");
        let (program, ok, fail): (&str, &[&str], &[&str]) = if cfg!(windows) {
            ("cmd", &["/C", "exit 0"], &["/C", "exit 7"])
        } else {
            ("sh", &["-c", "exit 0"], &["-c", "exit 7"])
        };
        let success =
            run_with_timeout(Command::new(program).args(ok), &log, RENDER_TIMEOUT).unwrap();
        let failure =
            run_with_timeout(Command::new(program).args(fail), &log, RENDER_TIMEOUT).unwrap();
        assert!(validate_output(success, &path, "ctx")
            .unwrap_err()
            .to_string()
            .contains("failed to read PDF output"));
        fs::write(&path, b"invalid").unwrap();
        assert!(validate_output(success, &path, "ctx")
            .unwrap_err()
            .to_string()
            .contains("valid PDF"));
        fs::write(&path, b"%PDF-test").unwrap();
        assert_eq!(
            validate_output(success, &path, "ctx").unwrap(),
            b"%PDF-test"
        );
        assert!(validate_output(failure, &path, "ctx")
            .unwrap_err()
            .to_string()
            .contains("Chromium failed"));
    }

    #[test]
    fn kills_processes_that_exceed_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("powershell", &["-NoProfile", "-Command", "Start-Sleep 30"])
        } else {
            ("sleep", &["30"])
        };
        let started = Instant::now();
        let error = run_with_timeout(
            Command::new(program).args(args),
            &dir.path().join("log"),
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(error.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
