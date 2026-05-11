use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{CharonError, Result};

pub type DbPool = Pool<SqliteConnectionManager>;

/// Build an r2d2 connection pool for the SQLite database at `path`.
/// Enables WAL mode for concurrent read/write performance and sets
/// a busy timeout so that write contention doesn't immediately fail.
pub fn create_pool(path: &str) -> Result<DbPool> {
    if !Path::new(path).exists() {
        // Ensure the parent directory exists.
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CharonError::Internal(format!("Failed to create DB directory: {}", e))
            })?;
        }
    }

    let manager = SqliteConnectionManager::file(path)
        .with_init(|conn| {
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA busy_timeout = 5000;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;",
            )?;
            Ok(())
        });

    let pool = Pool::builder()
        .max_size(8)
        .connection_timeout(Duration::from_secs(10))
        .build(manager)
        .map_err(|e| CharonError::Internal(format!("Failed to build DB pool: {}", e)))?;

    Ok(pool)
}

/// Run all pending schema migrations.  This is a lightweight version that
/// executes embedded SQL; for a production deployment you would typically
/// use a migration tool like `sqlx` or `refinery`.
pub fn run_migrations(pool: &DbPool) -> Result<()> {
    let conn = pool.get()?;
    schema::create_tables(&conn)?;
    tracing::info!("Database schema initialized / migrated");
    Ok(())
}

pub mod schema;
pub mod models;
pub mod repos;

pub use repos::{CandidateRepo, PositionRepo, StrategyRepo, DecisionRepo, LessonRepo};
