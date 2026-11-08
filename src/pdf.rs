use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use crate::error::{AppError, Result};

pub const MIME_TYPE: &str = "application/pdf";
const PDF_RENDER_ARGS: &[&str] = &[
    "--enable-local-file-access",
    "--print-media-type",
    "--enable-internal-links",
    "--enable-external-links",
    "--dpi",
    "300",
    "--zoom",
    "1.2",
    "--outline",
];

/// Converts a rendered HTML file into validated PDF bytes.
pub trait PdfRenderer {
    fn render(&self, html_path: &Path) -> Result<Vec<u8>>;
}

/// Adapter for the document templating system's wkhtmltopdf rendering contract.
pub struct WkhtmltopdfRenderer {
    renderer: PathBuf,
}

impl WkhtmltopdfRenderer {
    pub fn discover() -> Result<Self> {
        let root = env::current_dir().map_err(AppError::storage)?;
        Ok(Self {
            renderer: select_renderer(&root, env::var_os("DOCUMENT_TEMPLATING_MCP_WKHTMLTOPDF")),
        })
    }
}

fn candidates(root: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![
            root.join("tools/wkhtmltox/bin/wkhtmltopdf.exe"),
            root.join("tools/wkhtmltox/windows-x64/bin/wkhtmltopdf.exe"),
        ]
    } else if cfg!(target_os = "linux") {
        vec![
            root.join("tools/wkhtmltox/linux-x64/bin/wkhtmltopdf"),
            root.join("tools/wkhtmltox/bin/wkhtmltopdf"),
        ]
    } else {
        vec![root.join("tools/wkhtmltox/bin/wkhtmltopdf")]
    }
}

fn select_renderer(root: &Path, configured: Option<std::ffi::OsString>) -> PathBuf {
    if let Some(value) = configured {
        let path = PathBuf::from(value);
        return if path.is_relative() && path.components().count() > 1 {
            root.join(path)
        } else {
            path
        };
    }
    candidates(root)
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("wkhtmltopdf"))
}

impl PdfRenderer for WkhtmltopdfRenderer {
    fn render(&self, html_path: &Path) -> Result<Vec<u8>> {
        let root = env::current_dir().map_err(AppError::storage)?;
        let html_path = if html_path.is_absolute() {
            html_path.to_path_buf()
        } else {
            root.join(html_path)
        };
        let cwd = html_path.parent().ok_or_else(|| {
            AppError::Invalid("rendered HTML path must have a parent directory".into())
        })?;
        let temp = tempfile::Builder::new()
            .prefix("pdf-render-")
            .tempdir_in(cwd)
            .map_err(AppError::storage)?;
        let output_path = temp.path().join("document.pdf");
        let mut command = Command::new(&self.renderer);
        command
            .args(PDF_RENDER_ARGS)
            .arg(&html_path)
            .arg(&output_path)
            .current_dir(cwd);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let context = format!(
            "PDF renderer: {}\nPDF cwd: {}\nPDF input: {}\nPDF output: {}",
            self.renderer.display(),
            cwd.display(),
            html_path.display(),
            output_path.display()
        );
        let output = command.output().map_err(|error| AppError::storage(format!("failed to run PDF renderer: {error}\n{context}\nSet DOCUMENT_TEMPLATING_MCP_WKHTMLTOPDF or install wkhtmltopdf on PATH")))?;
        validate_output(&output, &output_path, &context)
    }
}

fn validate_output(output: &Output, output_path: &Path, context: &str) -> Result<Vec<u8>> {
    let diagnostics = format!(
        "{context}\nPDF exit: {}\nPDF stdout: {}\nPDF stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout).trim(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    if !output.status.success() {
        return Err(AppError::storage(format!(
            "wkhtmltopdf failed\n{diagnostics}"
        )));
    }
    let bytes = fs::read(output_path).map_err(|error| {
        AppError::storage(format!("failed to read PDF output: {error}\n{diagnostics}"))
    })?;
    if !bytes.starts_with(b"%PDF-") {
        return Err(AppError::storage(format!(
            "wkhtmltopdf completed without producing a valid PDF\n{diagnostics}"
        )));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn selects_override_then_local_then_path() {
        let root = tempfile::tempdir().unwrap();
        let paths = candidates(root.path());
        fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
        fs::write(&paths[0], b"renderer").unwrap();
        assert_eq!(select_renderer(root.path(), None), paths[0]);
        assert_eq!(
            select_renderer(root.path(), Some(OsString::from("bin/renderer"))),
            root.path().join("bin/renderer")
        );
        assert_eq!(
            select_renderer(root.path(), Some(OsString::from("custom-renderer"))),
            PathBuf::from("custom-renderer")
        );
        fs::remove_file(&paths[0]).unwrap();
        assert_eq!(
            select_renderer(root.path(), None),
            PathBuf::from("wkhtmltopdf")
        );
    }

    #[test]
    fn uses_source_system_flags_in_order() {
        assert_eq!(
            PDF_RENDER_ARGS,
            [
                "--enable-local-file-access",
                "--print-media-type",
                "--enable-internal-links",
                "--enable-external-links",
                "--dpi",
                "300",
                "--zoom",
                "1.2",
                "--outline"
            ]
        );
    }

    #[test]
    fn missing_renderer_reports_paths_and_remedy() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.html");
        fs::write(&input, "<p>test</p>").unwrap();
        let renderer = WkhtmltopdfRenderer {
            renderer: dir.path().join("missing-renderer"),
        };
        let error = renderer.render(&input).unwrap_err().to_string();
        assert!(error.contains("PDF renderer:"));
        assert!(error.contains("PDF cwd:"));
        assert!(error.contains("PDF input:"));
        assert!(error.contains("PDF output:"));
        assert!(error.contains("DOCUMENT_TEMPLATING_MCP_WKHTMLTOPDF"));
    }

    #[test]
    fn reports_converter_and_output_failures_with_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result.pdf");
        let (program, success_args, failure_args): (&str, &[&str], &[&str]) = if cfg!(windows) {
            (
                "cmd",
                &["/C", "echo renderer-output"],
                &["/C", "echo renderer-error & exit /b 7"],
            )
        } else {
            (
                "sh",
                &["-c", "echo renderer-output"],
                &["-c", "echo renderer-error >&2; exit 7"],
            )
        };
        let success = Command::new(program).args(success_args).output().unwrap();
        let missing = validate_output(&success, &path, "PDF cwd: workspace")
            .unwrap_err()
            .to_string();
        assert!(missing.contains("failed to read PDF output"));
        assert!(missing.contains("renderer-output"));
        fs::write(&path, b"invalid").unwrap();
        assert!(validate_output(&success, &path, "context")
            .unwrap_err()
            .to_string()
            .contains("valid PDF"));
        fs::write(&path, b"%PDF-test").unwrap();
        assert_eq!(
            validate_output(&success, &path, "context").unwrap(),
            b"%PDF-test"
        );
        let failure = Command::new(program).args(failure_args).output().unwrap();
        let error = validate_output(&failure, &path, "PDF cwd: workspace")
            .unwrap_err()
            .to_string();
        assert!(error.contains("PDF cwd: workspace"));
        assert!(error.contains("renderer-error"));
        assert!(error.contains('7'));
    }
}
