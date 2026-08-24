//! Rasterizes PDF pages to PNG images so the Map stage can send them to
//! GPT-4o vision alongside CU's markdown text -- matching the reference
//! FNOL accelerator's `map_handler.py`, which does the identical thing via
//! Python's `pdf2image` (itself a wrapper around `pdftoppm`/poppler). CU's
//! own response never carries page images, only text/markdown/fields, so
//! this has to happen independently of the CU call, not as part of it.
//!
//! Shells out to `pdftoppm` (poppler-utils) rather than pulling in a Rust
//! PDF-rendering crate -- same approach the reference takes, and poppler is
//! a one-line `apt-get install` in the runtime image versus a heavier
//! dependency (pdfium bundles a prebuilt binary; mupdf needs system libs).

use std::process::Stdio;
use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Error)]
pub enum PdfRenderError {
    #[error("failed to write temp PDF: {0}")]
    TempWriteFailed(String),
    #[error("pdftoppm failed: {0}")]
    PdftoppmFailed(String),
    #[error("no pages rendered")]
    NoPages,
}

/// Renders every page of `pdf_bytes` to a PNG, in page order. 200 DPI
/// matches `pdf2image.convert_from_bytes()`'s own default -- not an
/// arbitrary choice, so payload size/quality tracks the reference.
pub async fn render_pdf_pages_to_png(pdf_bytes: &[u8]) -> Result<Vec<Vec<u8>>, PdfRenderError> {
    let work_id = uuid::Uuid::new_v4();
    let dir = std::env::temp_dir().join(format!("docpipe-pdf-{work_id}"));
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| PdfRenderError::TempWriteFailed(e.to_string()))?;
    let pdf_path = dir.join("input.pdf");
    let out_prefix = dir.join("page");

    let result = render_inner(pdf_bytes, &pdf_path, &out_prefix, &dir).await;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    result
}

async fn render_inner(
    pdf_bytes: &[u8],
    pdf_path: &std::path::Path,
    out_prefix: &std::path::Path,
    dir: &std::path::Path,
) -> Result<Vec<Vec<u8>>, PdfRenderError> {
    tokio::fs::write(pdf_path, pdf_bytes)
        .await
        .map_err(|e| PdfRenderError::TempWriteFailed(e.to_string()))?;

    let output = Command::new("pdftoppm")
        .arg("-png")
        .arg("-r")
        .arg("200")
        .arg(pdf_path)
        .arg(out_prefix)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| PdfRenderError::PdftoppmFailed(format!("spawn failed: {e}")))?;

    if !output.status.success() {
        return Err(PdfRenderError::PdftoppmFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|e| PdfRenderError::PdftoppmFailed(e.to_string()))?;
    let mut pages: Vec<(u32, std::path::PathBuf)> = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| PdfRenderError::PdftoppmFailed(e.to_string()))?
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        // pdftoppm names output "<prefix>-<N>.png" (or "-<NN>.png" etc,
        // zero-padded to the total page count's digit width).
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if let Some(num_str) = stem.rsplit('-').next() {
            if let Ok(num) = num_str.parse::<u32>() {
                pages.push((num, path));
            }
        }
    }
    pages.sort_by_key(|(n, _)| *n);

    if pages.is_empty() {
        return Err(PdfRenderError::NoPages);
    }

    let mut png_bytes = Vec::with_capacity(pages.len());
    for (_, path) in pages {
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| PdfRenderError::PdftoppmFailed(e.to_string()))?;
        png_bytes.push(bytes);
    }
    Ok(png_bytes)
}
