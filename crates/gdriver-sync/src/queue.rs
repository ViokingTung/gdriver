//! Sync task queue types and priority management.

/// A sync task representing one pending or in-progress operation.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncTask {
    pub id: Option<i64>,
    pub account_id: String,
    pub file_id: Option<String>,
    /// One of: `upload`, `download`, `delete`, `rename`, `move`.
    pub operation: String,
    pub local_path: Option<String>,
    /// 1 (highest) – 10 (lowest).
    pub priority: i32,
    /// One of: `pending`, `in_progress`, `completed`, `failed`.
    pub status: String,
    pub retry_count: i32,
    pub error_msg: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl SyncTask {
    /// Create a new task with defaults.  The `priority` defaults to 5
    /// (medium) unless overridden by [`Self::with_priority`].
    pub fn new(account_id: impl Into<String>, operation: impl Into<String>) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            id: None,
            account_id: account_id.into(),
            file_id: None,
            operation: operation.into(),
            local_path: None,
            priority: 5,
            status: "pending".into(),
            retry_count: 0,
            error_msg: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set the file ID.
    pub fn with_file_id(mut self, file_id: impl Into<String>) -> Self {
        self.file_id = Some(file_id.into());
        self
    }

    /// Set the local path.
    pub fn with_local_path(mut self, path: impl Into<String>) -> Self {
        self.local_path = Some(path.into());
        self
    }

    /// Set the priority (1 = highest, 10 = lowest).
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority.clamp(1, 10);
        self
    }

    /// Returns `true` if the task has exceeded the maximum retry count.
    pub fn should_give_up(&self, max_retries: i32) -> bool {
        self.retry_count >= max_retries
    }

    /// Returns `true` if the task is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.status == "completed" || self.status == "failed"
    }

    /// Returns `true` if the task can be retried.
    pub fn is_retryable(&self, max_retries: i32) -> bool {
        self.status == "failed" && !self.should_give_up(max_retries)
    }
}

// ─── Priority ordering ────────────────────────────────────────────────────────

/// Priority constants for sync tasks.
pub mod priority {
    pub const HIGHEST: i32 = 1;
    pub const HIGH: i32 = 3;
    pub const MEDIUM: i32 = 5;
    pub const LOW: i32 = 7;
    pub const LOWEST: i32 = 10;
}

/// Compare two tasks for priority ordering (lower number = higher priority).
/// Tasks with the same priority are ordered by creation time (FIFO).
pub fn compare_priority(a: &SyncTask, b: &SyncTask) -> std::cmp::Ordering {
    a.priority
        .cmp(&b.priority)
        .then(a.created_at.cmp(&b.created_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Task creation ──────────────────────────────────────────────────────

    #[test]
    fn new_task_has_default_values() {
        let task = SyncTask::new("acct-1", "upload");
        assert_eq!(task.account_id, "acct-1");
        assert_eq!(task.operation, "upload");
        assert_eq!(task.priority, 5);
        assert_eq!(task.status, "pending");
        assert_eq!(task.retry_count, 0);
        assert!(task.file_id.is_none());
        assert!(task.local_path.is_none());
        assert!(task.error_msg.is_none());
        assert!(task.id.is_none());
        assert!(task.created_at > 0);
        assert_eq!(task.created_at, task.updated_at);
    }

    #[test]
    fn task_builder_sets_all_fields() {
        let task = SyncTask::new("acct-2", "download")
            .with_file_id("file-123")
            .with_local_path("/tmp/test.pdf")
            .with_priority(2);
        assert_eq!(task.account_id, "acct-2");
        assert_eq!(task.operation, "download");
        assert_eq!(task.file_id.unwrap(), "file-123");
        assert_eq!(task.local_path.unwrap(), "/tmp/test.pdf");
        assert_eq!(task.priority, 2);
    }

    #[test]
    fn priority_is_clamped_to_range() {
        let task = SyncTask::new("a", "upload").with_priority(0);
        assert_eq!(task.priority, 1);

        let task = SyncTask::new("a", "upload").with_priority(-5);
        assert_eq!(task.priority, 1);

        let task = SyncTask::new("a", "upload").with_priority(11);
        assert_eq!(task.priority, 10);

        let task = SyncTask::new("a", "upload").with_priority(100);
        assert_eq!(task.priority, 10);
    }

    // ── Terminal state checks ──────────────────────────────────────────────

    #[test]
    fn pending_is_not_terminal() {
        let task = SyncTask::new("a", "upload");
        assert!(!task.is_terminal());
    }

    #[test]
    fn completed_is_terminal() {
        let mut task = SyncTask::new("a", "upload");
        task.status = "completed".into();
        assert!(task.is_terminal());
    }

    #[test]
    fn failed_is_terminal() {
        let mut task = SyncTask::new("a", "upload");
        task.status = "failed".into();
        assert!(task.is_terminal());
    }

    #[test]
    fn in_progress_is_not_terminal() {
        let mut task = SyncTask::new("a", "upload");
        task.status = "in_progress".into();
        assert!(!task.is_terminal());
    }

    // ── Retry logic ────────────────────────────────────────────────────────

    #[test]
    fn should_not_give_up_below_max_retries() {
        let task = SyncTask::new("a", "upload");
        assert!(!task.should_give_up(3));
    }

    #[test]
    fn should_give_up_at_max_retries() {
        let mut task = SyncTask::new("a", "upload");
        task.retry_count = 3;
        assert!(task.should_give_up(3));
    }

    #[test]
    fn should_give_up_above_max_retries() {
        let mut task = SyncTask::new("a", "upload");
        task.retry_count = 5;
        assert!(task.should_give_up(3));
    }

    #[test]
    fn failed_task_is_retryable_when_below_max() {
        let mut task = SyncTask::new("a", "upload");
        task.status = "failed".into();
        task.retry_count = 1;
        assert!(task.is_retryable(3));
    }

    #[test]
    fn failed_task_is_not_retryable_when_at_max() {
        let mut task = SyncTask::new("a", "upload");
        task.status = "failed".into();
        task.retry_count = 3;
        assert!(!task.is_retryable(3));
    }

    #[test]
    fn completed_task_is_not_retryable() {
        let mut task = SyncTask::new("a", "upload");
        task.status = "completed".into();
        assert!(!task.is_retryable(3));
    }

    #[test]
    fn pending_task_is_not_retryable() {
        let task = SyncTask::new("a", "upload");
        assert!(!task.is_retryable(3));
    }

    #[test]
    fn retry_uses_max_retries_of_zero() {
        // Even with 0 max retries, a fresh task shouldn't give up.
        // But if retry count >= 0, it does.
        let mut task = SyncTask::new("a", "upload");
        task.retry_count = 0;
        assert!(task.should_give_up(0));
    }

    // ── Priority ordering ──────────────────────────────────────────────────

    #[test]
    fn higher_priority_sorts_first() {
        let high = SyncTask::new("a", "upload").with_priority(1);
        let low = SyncTask::new("a", "upload").with_priority(10);
        assert_eq!(compare_priority(&high, &low), std::cmp::Ordering::Less);
        assert_eq!(compare_priority(&low, &high), std::cmp::Ordering::Greater);
    }

    #[test]
    fn same_priority_sorts_by_creation_time() {
        let older = SyncTask {
            id: Some(1),
            created_at: 1000,
            updated_at: 1000,
            ..SyncTask::new("a", "upload")
        };
        // Give a tiny gap so timestamps differ.
        let newer = SyncTask {
            id: Some(2),
            created_at: 2000,
            updated_at: 2000,
            ..SyncTask::new("a", "upload")
        };
        assert_eq!(compare_priority(&older, &newer), std::cmp::Ordering::Less);
        assert_eq!(
            compare_priority(&newer, &older),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn same_priority_and_time_are_equal() {
        let t = 1_700_000_000_000_i64;
        let a = SyncTask {
            id: Some(1),
            created_at: t,
            updated_at: t,
            ..SyncTask::new("a", "upload")
        };
        let b = SyncTask {
            id: Some(2),
            created_at: t,
            updated_at: t,
            ..SyncTask::new("a", "upload")
        };
        assert_eq!(compare_priority(&a, &b), std::cmp::Ordering::Equal);
    }

    #[test]
    fn priority_constants_are_ordered() {
        const {
            assert!(priority::HIGHEST < priority::HIGH);
            assert!(priority::HIGH < priority::MEDIUM);
            assert!(priority::MEDIUM < priority::LOW);
            assert!(priority::LOW < priority::LOWEST);
        };
    }

    #[test]
    fn priority_constants_are_in_range() {
        for p in [
            priority::HIGHEST,
            priority::HIGH,
            priority::MEDIUM,
            priority::LOW,
            priority::LOWEST,
        ] {
            assert!((1..=10).contains(&p), "priority {p} out of range");
        }
    }

    // ── Multiple operations ────────────────────────────────────────────────

    #[test]
    fn task_operation_variants() {
        for op in ["upload", "download", "delete", "rename", "move"] {
            let task = SyncTask::new("acct", op);
            assert_eq!(task.operation, op);
            assert_eq!(task.status, "pending");
        }
    }

    #[test]
    fn task_clone_is_equal() {
        let task = SyncTask::new("acct", "upload")
            .with_file_id("f1")
            .with_local_path("/tmp/f.txt")
            .with_priority(3);
        assert_eq!(task, task.clone());
    }

    #[test]
    fn tasks_with_different_ids_are_not_equal() {
        let mut a = SyncTask::new("acct", "upload");
        a.id = Some(1);
        let mut b = SyncTask::new("acct", "upload");
        b.id = Some(2);
        assert_ne!(a, b);
    }
}
