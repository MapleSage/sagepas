use anyhow::Context;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use infra::db::DbPool;

#[derive(Debug, Clone)]
pub struct MigrationRunSummary {
    pub files: Vec<String>,
    pub applied_now: usize,
    pub count: usize,
}

fn checksum(input: &str) -> String {
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn discover_migration_dir() -> Option<PathBuf> {
    let candidates = [
        std::env::var("MIGRATIONS_DIR").ok().map(PathBuf::from),
        Some(PathBuf::from("/app/migrations")),
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")),
    ];

    candidates.into_iter().flatten().find(|p| p.exists())
}

pub fn migration_registry() -> anyhow::Result<(PathBuf, Vec<String>)> {
    let dir = discover_migration_dir().ok_or_else(|| {
        anyhow::anyhow!(
            "no migrations directory found (checked MIGRATIONS_DIR, /app/migrations, ../../migrations)"
        )
    })?;

    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("unable to read migrations dir: {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("sql") {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                files.push(name.to_string());
            }
        }
    }

    files.sort();
    if files.is_empty() {
        anyhow::bail!("no SQL migrations discovered in {}", dir.display());
    }

    Ok((dir, files))
}

pub async fn run_startup_migrations(pool: &DbPool) -> anyhow::Result<MigrationRunSummary> {
    let (dir, files) = migration_registry()?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migration_registry (
            file_name TEXT PRIMARY KEY,
            checksum TEXT NOT NULL,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(&**pool)
    .await
    .context("failed creating schema_migration_registry")?;

    let mut applied_now = 0usize;

    for file_name in &files {
        let sql_path = dir.join(file_name);
        let sql = std::fs::read_to_string(&sql_path)
            .with_context(|| format!("failed to read migration file {}", sql_path.display()))?;
        let sum = checksum(&sql);

        let existing: Option<String> = sqlx::query_scalar(
            "SELECT checksum FROM schema_migration_registry WHERE file_name = $1",
        )
        .bind(file_name)
        .fetch_optional(&**pool)
        .await
        .with_context(|| format!("failed lookup for migration {file_name}"))?;

        if let Some(old) = existing {
            if old != sum {
                anyhow::bail!(
                    "migration checksum mismatch for {} (stored={}, current={})",
                    file_name,
                    old,
                    sum
                );
            }
            continue;
        }

        sqlx::raw_sql(&sql)
            .execute(&**pool)
            .await
            .with_context(|| format!("failed applying migration {file_name}"))?;

        sqlx::query("INSERT INTO schema_migration_registry (file_name, checksum) VALUES ($1, $2)")
            .bind(file_name)
            .bind(&sum)
            .execute(&**pool)
            .await
            .with_context(|| format!("failed writing migration registry for {file_name}"))?;

        applied_now += 1;
    }

    Ok(MigrationRunSummary {
        count: files.len(),
        files,
        applied_now,
    })
}
