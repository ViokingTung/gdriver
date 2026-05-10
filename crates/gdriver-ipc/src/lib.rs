pub mod client;
pub mod methods;
pub mod types;

pub use types::*;

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::{methods::*, types::*};

    // ── JSON-RPC protocol round-trips ────────────────────────────────────────

    #[test]
    fn json_rpc_request_roundtrip() {
        let req = JsonRpcRequest::new(PING, None, JsonRpcId::Num(1));
        let json = serde_json::to_string(&req).unwrap();
        let parsed: JsonRpcRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.jsonrpc, "2.0");
        assert_eq!(parsed.method, PING);
        assert_eq!(parsed.id, Some(JsonRpcId::Num(1)));
        assert!(parsed.params.is_none());
    }

    #[test]
    fn json_rpc_notification_has_no_id() {
        let notif = JsonRpcRequest::notification(EVENT_SYNC_STATUS_CHANGED, None);
        let json = serde_json::to_string(&notif).unwrap();

        assert!(!json.contains("\"id\""));
        assert!(notif.is_notification());
    }

    #[test]
    fn json_rpc_response_success_roundtrip() {
        let resp = JsonRpcResponse::success(Some(JsonRpcId::Num(42)), json!("pong"));
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: JsonRpcResponse = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_success());
        assert_eq!(parsed.result, Some(Value::String("pong".into())));
        assert_eq!(parsed.id, Some(JsonRpcId::Num(42)));
    }

    #[test]
    fn json_rpc_error_method_not_found() {
        let err = JsonRpcError::method_not_found("unknown.method");
        let resp = JsonRpcResponse::error(Some(JsonRpcId::Str("req-1".into())), err);
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: JsonRpcResponse = serde_json::from_str(&json).unwrap();

        assert!(!parsed.is_success());
        assert_eq!(parsed.error.unwrap().code, -32601);
    }

    #[test]
    fn json_rpc_id_str_roundtrip() {
        let id = JsonRpcId::Str("abc-123".into());
        let json = serde_json::to_string(&id).unwrap();
        let parsed: JsonRpcId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, parsed);
    }

    // ── Enum serialization ───────────────────────────────────────────────────

    #[test]
    fn sync_status_kebab_case() {
        assert_eq!(
            serde_json::to_string(&SyncStatus::UpToDate).unwrap(),
            "\"up-to-date\""
        );
        assert_eq!(
            serde_json::from_str::<SyncStatus>("\"syncing\"").unwrap(),
            SyncStatus::Syncing
        );
    }

    #[test]
    fn sync_state_snake_case() {
        assert_eq!(
            serde_json::to_string(&SyncState::CloudOnly).unwrap(),
            "\"cloud_only\""
        );
        assert_eq!(
            serde_json::from_str::<SyncState>("\"cloud_only\"").unwrap(),
            SyncState::CloudOnly
        );
    }

    #[test]
    fn appearance_roundtrip() {
        for v in [
            Appearance::Light,
            Appearance::Dark,
            Appearance::FollowSystem,
        ] {
            let s = serde_json::to_string(&v).unwrap();
            let back: Appearance = serde_json::from_str(&s).unwrap();
            assert_eq!(v, back);
        }
    }

    // ── Data type round-trips ────────────────────────────────────────────────

    #[test]
    fn sync_item_roundtrip() {
        let item = SyncItem {
            file_id: Some("abc".into()),
            name: "report.pdf".into(),
            mime_type: Some("application/pdf".into()),
            local_path: Some("/home/user/GoogleDrive/report.pdf".into()),
            sync_state: SyncState::Synced,
            progress: None,
            file_size: Some(1_024_000),
            error_msg: None,
            drive_url: Some("https://drive.google.com/file/d/abc".into()),
            updated_at: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: SyncItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "report.pdf");
        assert_eq!(back.sync_state, SyncState::Synced);
    }

    #[test]
    fn notification_conflict_roundtrip() {
        let notif = Notification {
            id: 1,
            account_id: Some("user-123".into()),
            is_read: false,
            created_at: 1_700_000_000_000,
            kind: NotificationKind::Conflict {
                file_id: Some("xyz".into()),
                file_name: "notes.txt".into(),
                conflict_copy_name: "notes (conflict copy 2024-01-01 12:00:00).txt".into(),
            },
        };
        let json = serde_json::to_string(&notif).unwrap();
        let back: Notification = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, 1);
        matches!(back.kind, NotificationKind::Conflict { .. });
    }

    #[test]
    fn storage_warning_notification_roundtrip() {
        let notif = Notification {
            id: 2,
            account_id: None,
            is_read: false,
            created_at: 1_700_000_000_000,
            kind: NotificationKind::StorageWarning {
                usage_percent: 95.5,
            },
        };
        let json = serde_json::to_string(&notif).unwrap();
        let back: Notification = serde_json::from_str(&json).unwrap();
        matches!(back.kind, NotificationKind::StorageWarning { .. });
    }

    #[test]
    fn preferences_default_roundtrip() {
        let prefs = Preferences::default();
        let json = serde_json::to_string(&prefs).unwrap();
        let back: Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(back.general.launch_on_login, true);
        assert_eq!(back.general.appearance, Appearance::FollowSystem);
        assert_eq!(back.general.language, "follow_account");
        assert_eq!(back.network.download_rate_limit, 0);
    }

    // ── Push event round-trips ───────────────────────────────────────────────

    #[test]
    fn sync_status_payload_roundtrip() {
        let payload = SyncStatusPayload {
            status: SyncStatus::Syncing,
            ts: 1_700_000_000_000,
            speed: Some(512_000),
            pending: Some(3),
        };
        let event = PushEvent::SyncStatusChanged(payload);
        let notif = event.to_notification().unwrap();

        assert_eq!(notif.method, EVENT_SYNC_STATUS_CHANGED);
        assert!(notif.is_notification());

        let back = PushEvent::from_notification(&notif.method, notif.params).unwrap();
        let PushEvent::SyncStatusChanged(p) = back else {
            panic!("wrong variant");
        };
        assert_eq!(p.status, SyncStatus::Syncing);
        assert_eq!(p.speed, Some(512_000));
    }

    #[test]
    fn oauth_complete_event_roundtrip() {
        let payload = OauthCompletePayload {
            account_id: "acct-001".into(),
        };
        let event = PushEvent::OauthComplete(payload);
        let notif = event.to_notification().unwrap();

        assert_eq!(notif.method, EVENT_ONBOARDING_OAUTH_COMPLETE);

        let back = PushEvent::from_notification(&notif.method, notif.params).unwrap();
        let PushEvent::OauthComplete(p) = back else {
            panic!("wrong variant");
        };
        assert_eq!(p.account_id, "acct-001");
    }

    #[test]
    fn unknown_push_event_is_preserved() {
        let back =
            PushEvent::from_notification("future:new-event", Some(json!({"x": 42}))).unwrap();
        let PushEvent::Unknown { method, .. } = back else {
            panic!("should be Unknown");
        };
        assert_eq!(method, "future:new-event");
    }

    #[test]
    fn account_changed_event_roundtrip() {
        let payload = AccountChangedPayload {
            accounts: vec![Account {
                id: "uid-1".into(),
                email: "test@example.com".into(),
                display_name: Some("Test User".into()),
                photo_url: None,
                locale: Some("en".into()),
                created_at: 0,
                last_used_at: 0,
            }],
        };
        let event = PushEvent::AccountChanged(payload);
        let notif = event.to_notification().unwrap();
        let back = PushEvent::from_notification(&notif.method, notif.params).unwrap();
        let PushEvent::AccountChanged(p) = back else {
            panic!("wrong variant");
        };
        assert_eq!(p.accounts[0].email, "test@example.com");
    }
}
