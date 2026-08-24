//! Runs the real pipeline against a real PDF, against real CU/OpenAI, using
//! whatever workload identity / env config is present -- for verifying the
//! work order's "Done when" claims with actual evidence (entity_score,
//! schema_score, page_image_count, per-field citations), not an assertion
//! that they hold. Not part of the API binary; built and run once inside
//! the cluster (workload identity for CU token acquisition only exists
//! there), then discarded.

use doc_pipeline::{Domain, OutputKind, PipelineConfig};

#[tokio::main]
async fn main() {
    let pdf_path = std::env::args().nth(1).expect("usage: verify <pdf-path> <schema-key>");
    let schema_key = std::env::args().nth(2).unwrap_or_else(|| "life".to_string());

    let file_bytes = std::fs::read(&pdf_path).expect("read pdf");

    // Diagnostic mode: `verify <pdf> cu-raw <analyzer-id>` calls CU directly
    // with an explicit analyzer id and dumps page1's word/line counts --
    // bypasses run_pipeline entirely, doesn't touch select_analyzer().
    if schema_key == "cu-raw" {
        let analyzer_id = std::env::args().nth(3).unwrap_or_else(|| "prebuilt-document".to_string());
        let cu_endpoint = std::env::var("CONTENT_UNDERSTANDING_ENDPOINT").expect("CONTENT_UNDERSTANDING_ENDPOINT");
        let cu = doc_pipeline::content_understanding::ContentUnderstandingClient::new(&cu_endpoint);
        let bearer = doc_pipeline::content_understanding::acquire_cu_token().await.expect("cu token");
        println!("=== raw CU call, analyzer={} ===", analyzer_id);
        match cu.analyze_bytes(&analyzer_id, &file_bytes, &bearer, 120).await {
            Ok(result) => {
                let content = &result.result.contents[0];
                println!("content_blocks: {}", result.result.contents.len());
                println!("pages: {}", content.pages.len());
                if let Some(p1) = content.pages.first() {
                    println!("page1: words={} lines={} width={} height={}", p1.words.len(), p1.lines.len(), p1.width, p1.height);
                    if let Some(l) = p1.lines.first() {
                        println!("page1 first line: {:?}", l.content);
                    }
                    if let Some(w) = p1.words.first() {
                        println!("page1 first word: {:?} conf={}", w.content, w.confidence);
                    }
                }
                println!("fields present: {}", content.fields.is_some());
                println!("markdown_len: {}", content.markdown.len());
            }
            Err(e) => println!("CU call failed: {e}"),
        }
        return;
    }

    let cu_endpoint = std::env::var("CONTENT_UNDERSTANDING_ENDPOINT").unwrap_or_default();
    let cu_client = if cu_endpoint.trim().is_empty() {
        eprintln!("WARNING: CONTENT_UNDERSTANDING_ENDPOINT not set, running without CU");
        None
    } else {
        Some(doc_pipeline::content_understanding::ContentUnderstandingClient::new(&cu_endpoint))
    };

    let openai_endpoint = std::env::var("AZURE_OPENAI_ENDPOINT").expect("AZURE_OPENAI_ENDPOINT");
    let openai_key = std::env::var("AZURE_OPENAI_KEY").expect("AZURE_OPENAI_KEY");
    let openai_deployment = std::env::var("AZURE_OPENAI_DEPLOYMENT").unwrap_or_else(|_| "gpt-5.6-luna".to_string());
    let openai = infra::openai::OpenAIClient::new(&openai_endpoint, &openai_key, &openai_deployment);

    let config = PipelineConfig {
        domain: Domain::Underwriting,
        field_schema_key: schema_key,
        kb_index: "insurance-general-kb".to_string(),
        output_kind: OutputKind::HubspotDeal,
    };

    println!("=== running real pipeline against {} ({} bytes) ===", pdf_path, file_bytes.len());
    let result = doc_pipeline::run_pipeline(
        &file_bytes,
        "application/pdf",
        cu_client.as_ref(),
        &openai,
        &config,
    )
    .await
    .expect("pipeline failed");

    println!("\n=== STAGES ===");
    for stage in &result.stages {
        println!("  {} -> {} (detail: {})", stage.name, stage.status, stage.detail.clone().unwrap_or_default());
    }

    println!("\n=== SCORES ===");
    println!("  entity_score: {}", result.entity_score);
    println!("  schema_score: {}", result.schema_score);
    println!("  human_review_required: {}", result.human_review_required);

    println!("\n=== CITED FIELDS (first 5 non-null) ===");
    if let Some(obj) = result.extracted_json.as_object() {
        let mut shown = 0;
        for (k, v) in obj {
            if v.is_null() {
                continue;
            }
            println!("  {}: {}", k, v);
            shown += 1;
            if shown >= 5 {
                break;
            }
        }
    }
}
