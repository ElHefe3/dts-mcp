//! PNG previews of PDF pages, so an agent can look at the exact printed output.

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use crate::{
    error::{AppError, Result},
    pdf::{run_with_timeout, which},
};

pub const PDFTOPPM_ENV: &str = "DOCUMENT_TEMPLATING_MCP_PDFTOPPM";
pub const DEFAULT_DPI: u32 = 80;
pub const MAX_DPI: u32 = 150;
pub const MAX_PAGES: u32 = 10;

/// Rasterizes pages `1..=last_page` of a PDF into PNG bytes, in page order.
pub fn page_pngs(pdf_path: &Path, dpi: u32, last_page: u32) -> Result<Vec<Vec<u8>>> {
    let program = PathBuf::from(
        env::var_os(PDFTOPPM_ENV)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| OsString::from("pdftoppm")),
    );
    if !program.is_file() && which(&program).is_none() {
        return Err(AppError::storage(format!(
            "page previews need poppler's pdftoppm; install poppler-utils or set {PDFTOPPM_ENV}"
        )));
    }
    let temp = tempfile::tempdir().map_err(AppError::storage)?;
    let prefix = temp.path().join("page");
    let status = run_with_timeout(
        Command::new(&program)
            .arg("-png")
            .args(["-r", &dpi.to_string()])
            .args(["-f", "1", "-l", &last_page.to_string()])
            .arg(pdf_path)
            .arg(&prefix),
        &temp.path().join("log"),
        Duration::from_secs(60),
    )
    .map_err(AppError::storage)?;
    if !status.success() {
        let log = fs::read_to_string(temp.path().join("log")).unwrap_or_default();
        return Err(AppError::storage(format!(
            "pdftoppm failed ({status}): {}",
            log.trim()
        )));
    }
    let mut pages: Vec<(u32, PathBuf)> = fs::read_dir(temp.path())
        .map_err(AppError::storage)?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_stem()?.to_str()?;
            let number = name.strip_prefix("page-")?.parse().ok()?;
            (path.extension()? == "png").then_some((number, path))
        })
        .collect();
    pages.sort();
    pages
        .into_iter()
        .map(|(_, path)| fs::read(path).map_err(AppError::storage))
        .collect()
}
