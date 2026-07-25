use anyhow::Context;
use domain::insurance::{Currency, Customer, NationalIdType, Policy, Product};
use infra::blob::BlobClient;
use printpdf::*;

/// Generates policy PDF documents and uploads them to Azure Blob Storage.
pub struct DocumentGenerator {
    blob: BlobClient,
    /// Base URL prefix for constructing public blob URLs,
    /// e.g. `https://mystorage.blob.core.windows.net/policy-documents`.
    base_url: String,
}

impl DocumentGenerator {
    pub fn new(blob: BlobClient, container: String, storage_account: &str) -> Self {
        let base_url = format!(
            "https://{}.blob.core.windows.net/{}",
            storage_account, container
        );
        Self { blob, base_url }
    }

    /// Generate a policy declaration page PDF, upload it to Blob Storage,
    /// and return the public URL.
    pub async fn generate_policy_document(
        &self,
        policy: &Policy,
        customer: &Customer,
        product: &Product,
    ) -> anyhow::Result<String> {
        let pdf_bytes = self.render_policy_document(policy, customer, product)?;

        // Blob name: {policy_number}.pdf
        let blob_name = format!("{}.pdf", policy.policy_number);

        self.blob
            .upload(&blob_name, pdf_bytes, "application/pdf")
            .await
            .context("Failed to upload policy PDF to blob storage")?;

        let url = format!("{}/{}", self.base_url, blob_name);
        Ok(url)
    }

    /// Render a policy PDF without choosing a persistence backend. The
    /// standalone local runtime stores these bytes on disk; production keeps
    /// using Azure Blob Storage through `generate_policy_document`.
    pub fn render_policy_document(
        &self,
        policy: &Policy,
        customer: &Customer,
        product: &Product,
    ) -> anyhow::Result<Vec<u8>> {
        if customer.country.eq_ignore_ascii_case("IN") {
            generate_india_schedule(policy, customer, product)
        } else {
            generate_us_declaration(policy, customer, product)
        }
    }
}

// ── shared helper ─────────────────────────────────────────────────────────────

/// Serialise a printpdf document to a Vec<u8>.
fn to_bytes(doc: PdfDocumentReference) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    doc.save(&mut std::io::BufWriter::new(std::io::Cursor::new(&mut buf)))?;
    Ok(buf)
}

// ── US declaration page ───────────────────────────────────────────────────────

/// US auto / personal lines declaration page (US Letter, portrait).
fn generate_us_declaration(
    policy: &Policy,
    customer: &Customer,
    product: &Product,
) -> anyhow::Result<Vec<u8>> {
    let (doc, page1, layer1) = PdfDocument::new(
        "Policy Declaration",
        Mm(215.9_f32),
        Mm(279.4_f32),
        "Layer 1",
    );

    let page = doc.get_page(page1);
    let layer = page.get_layer(layer1);
    let font = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;

    let currency_sym = match policy.currency {
        Currency::Usd => "$",
        Currency::Inr => "Rs",
        Currency::Aed => "AED",
    };

    // ── Header ───────────────────────────────────────────────────────────────
    layer.use_text("SageSure Insurance", 22.0, Mm(20.0), Mm(258.0), &font_bold);
    layer.use_text("DECLARATIONS PAGE", 14.0, Mm(20.0), Mm(248.0), &font_bold);
    layer.use_text(
        &format!("Policy Number: {}", policy.policy_number),
        11.0,
        Mm(20.0),
        Mm(238.0),
        &font_bold,
    );

    // ── Named insured ─────────────────────────────────────────────────────────
    let mut y = 225.0_f32;
    layer.use_text("Named Insured:", 10.0, Mm(20.0), Mm(y), &font_bold);
    layer.use_text(&customer.name, 10.0, Mm(72.0), Mm(y), &font);
    y -= 8.0;

    if let Some(addr) = &customer.address {
        layer.use_text("Address:", 10.0, Mm(20.0), Mm(y), &font_bold);
        layer.use_text(addr, 10.0, Mm(72.0), Mm(y), &font);
        y -= 8.0;
    }

    // ── Product & coverage period ─────────────────────────────────────────────
    layer.use_text("Product:", 10.0, Mm(20.0), Mm(y), &font_bold);
    layer.use_text(&product.name, 10.0, Mm(72.0), Mm(y), &font);
    y -= 8.0;

    let eff = policy.start_date.format("%m/%d/%Y").to_string();
    let exp = policy.end_date.format("%m/%d/%Y").to_string();
    layer.use_text("Policy Period:", 10.0, Mm(20.0), Mm(y), &font_bold);
    layer.use_text(&format!("{eff}  –  {exp}"), 10.0, Mm(72.0), Mm(y), &font);
    y -= 16.0;

    // ── Coverage table ────────────────────────────────────────────────────────
    layer.use_text("Coverage Detail", 11.0, Mm(20.0), Mm(y), &font_bold);
    layer.use_text("Limit / Amount", 11.0, Mm(130.0), Mm(y), &font_bold);
    y -= 8.0;

    if let Some(obj) = policy.coverage.as_object() {
        for (key, val) in obj {
            let label = key.replace('_', " ");
            let value_str = match val {
                serde_json::Value::Number(n) => format!("{currency_sym}{n}"),
                serde_json::Value::String(s) => s.clone(),
                _ => val.to_string(),
            };
            layer.use_text(&label, 10.0, Mm(20.0), Mm(y), &font);
            layer.use_text(&value_str, 10.0, Mm(130.0), Mm(y), &font);
            y -= 7.0;
        }
    }

    // ── Premium ───────────────────────────────────────────────────────────────
    y -= 8.0;
    layer.use_text("Annual Premium:", 12.0, Mm(20.0), Mm(y), &font_bold);
    layer.use_text(
        &format!("{currency_sym}{:.2}", policy.premium),
        12.0,
        Mm(85.0),
        Mm(y),
        &font_bold,
    );
    y -= 20.0;

    // ── Signature line ────────────────────────────────────────────────────────
    layer.use_text(
        "Authorized Representative: ____________________________",
        10.0,
        Mm(20.0),
        Mm(y),
        &font,
    );
    y -= 8.0;
    layer.use_text(
        &format!("Date: {}", policy.created_at.format("%m/%d/%Y")),
        9.0,
        Mm(20.0),
        Mm(y),
        &font,
    );

    // ── Footer ────────────────────────────────────────────────────────────────
    layer.use_text(
        "SageSure Insurance | info@sagesure.io | This document is computer-generated.",
        8.0,
        Mm(20.0),
        Mm(12.0),
        &font,
    );

    to_bytes(doc)
}

// ── India policy schedule ─────────────────────────────────────────────────────

/// India motor / life policy schedule (A4, portrait).
fn generate_india_schedule(
    policy: &Policy,
    customer: &Customer,
    product: &Product,
) -> anyhow::Result<Vec<u8>> {
    let (doc, page1, layer1) =
        PdfDocument::new("Policy Schedule", Mm(210.0_f32), Mm(297.0_f32), "Layer 1");

    let page = doc.get_page(page1);
    let layer = page.get_layer(layer1);
    let font = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;

    // ── Header ────────────────────────────────────────────────────────────────
    layer.use_text(
        "SageSure Insurance India",
        20.0,
        Mm(20.0),
        Mm(273.0),
        &font_bold,
    );
    layer.use_text("POLICY SCHEDULE", 14.0, Mm(20.0), Mm(263.0), &font_bold);
    layer.use_text(
        &format!("Policy No: {}", policy.policy_number),
        11.0,
        Mm(20.0),
        Mm(253.0),
        &font_bold,
    );

    // ── Policyholder details ──────────────────────────────────────────────────
    let mut y = 240.0_f32;
    layer.use_text("Policyholder:", 10.0, Mm(20.0), Mm(y), &font_bold);
    layer.use_text(&customer.name, 10.0, Mm(72.0), Mm(y), &font);
    y -= 8.0;

    layer.use_text("Mobile:", 10.0, Mm(20.0), Mm(y), &font_bold);
    layer.use_text(&customer.phone, 10.0, Mm(72.0), Mm(y), &font);
    y -= 8.0;

    if let Some(addr) = &customer.address {
        layer.use_text("Address:", 10.0, Mm(20.0), Mm(y), &font_bold);
        layer.use_text(addr, 10.0, Mm(72.0), Mm(y), &font);
        y -= 8.0;
    }

    // Aadhaar / PAN
    if let Some(nid) = &customer.national_id {
        let label = match &customer.national_id_type {
            Some(NationalIdType::Pan) => "PAN:",
            Some(NationalIdType::Aadhar) => "Aadhaar:",
            _ => "National ID:",
        };
        layer.use_text(label, 10.0, Mm(20.0), Mm(y), &font_bold);
        layer.use_text(nid, 10.0, Mm(72.0), Mm(y), &font);
        y -= 8.0;
    }

    // ── Product & period ──────────────────────────────────────────────────────
    layer.use_text("Product:", 10.0, Mm(20.0), Mm(y), &font_bold);
    layer.use_text(&product.name, 10.0, Mm(72.0), Mm(y), &font);
    y -= 8.0;

    let eff = policy.start_date.format("%d/%m/%Y").to_string();
    let exp = policy.end_date.format("%d/%m/%Y").to_string();
    layer.use_text("Policy Period:", 10.0, Mm(20.0), Mm(y), &font_bold);
    layer.use_text(&format!("{eff}  to  {exp}"), 10.0, Mm(72.0), Mm(y), &font);
    y -= 16.0;

    // ── Coverage table ────────────────────────────────────────────────────────
    layer.use_text("Coverage", 11.0, Mm(20.0), Mm(y), &font_bold);
    layer.use_text("Sum Insured (INR)", 11.0, Mm(130.0), Mm(y), &font_bold);
    y -= 8.0;

    if let Some(obj) = policy.coverage.as_object() {
        for (key, val) in obj {
            let label = key.replace('_', " ");
            let value_str = match val {
                serde_json::Value::Number(n) => format!("Rs {n}"),
                serde_json::Value::String(s) => s.clone(),
                _ => val.to_string(),
            };
            layer.use_text(&label, 10.0, Mm(20.0), Mm(y), &font);
            layer.use_text(&value_str, 10.0, Mm(130.0), Mm(y), &font);
            y -= 7.0;
        }
    }

    // ── Premium ───────────────────────────────────────────────────────────────
    y -= 8.0;
    layer.use_text("Annual Premium (INR):", 12.0, Mm(20.0), Mm(y), &font_bold);
    layer.use_text(
        &format!("Rs {:.2}", policy.premium),
        12.0,
        Mm(90.0),
        Mm(y),
        &font_bold,
    );
    y -= 20.0;

    // ── Signature ─────────────────────────────────────────────────────────────
    layer.use_text(
        "Authorised Signatory: ____________________________",
        10.0,
        Mm(20.0),
        Mm(y),
        &font,
    );
    y -= 8.0;
    layer.use_text(
        &format!("Date: {}", policy.created_at.format("%d/%m/%Y")),
        9.0,
        Mm(20.0),
        Mm(y),
        &font,
    );

    // ── Footer ────────────────────────────────────────────────────────────────
    layer.use_text(
        "SageSure Insurance India | info@sagesure.io | IRDAI Reg. No. XXXX",
        8.0,
        Mm(20.0),
        Mm(12.0),
        &font,
    );

    to_bytes(doc)
}
