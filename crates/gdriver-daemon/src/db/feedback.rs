use sqlx::SqlitePool;

// ─── Public domain type ────────────────────────────────────────────────────────

/// A row in the `feedback` table.
#[derive(Debug, Clone)]
pub struct Feedback {
    #[allow(dead_code)]
    pub id: Option<i64>,
    pub text: String,
    pub include_logs: bool,
    pub created_at: i64,
}

// ─── CRUD ─────────────────────────────────────────────────────────────────────

/// Insert a new feedback submission and return it with the assigned id.
pub async fn insert_feedback(pool: &SqlitePool, feedback: &Feedback) -> anyhow::Result<Feedback> {
    let row = sqlx::query_as::<_, FeedbackRow>(
        r#"
        INSERT INTO feedback (text, include_logs, created_at)
        VALUES (?, ?, ?)
        RETURNING id, text, include_logs, created_at
        "#,
    )
    .bind(&feedback.text)
    .bind(feedback.include_logs as i64)
    .bind(feedback.created_at)
    .fetch_one(pool)
    .await?;

    Ok(Feedback::from(row))
}

// ─── Internal row type ────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct FeedbackRow {
    id: i64,
    text: String,
    include_logs: i64,
    created_at: i64,
}

impl From<FeedbackRow> for Feedback {
    fn from(r: FeedbackRow) -> Self {
        Self {
            id: Some(r.id),
            text: r.text,
            include_logs: r.include_logs != 0,
            created_at: r.created_at,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    use super::*;

    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("in-memory pool");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");

        pool
    }

    #[tokio::test]
    async fn insert_and_retrieve() {
        let pool = test_pool().await;
        let fb = insert_feedback(
            &pool,
            &Feedback {
                id: None,
                text: "Great app!".into(),
                include_logs: true,
                created_at: 1_700_000_000_000,
            },
        )
        .await
        .unwrap();

        assert!(fb.id.is_some());
        assert_eq!(fb.text, "Great app!");
        assert!(fb.include_logs);
    }
}
