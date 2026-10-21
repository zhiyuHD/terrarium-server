use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tempfile::TempDir;
use terrarium_server::{
    forum_bridge,
    http::{App, router},
    ingestion::Ingestion,
    operations::OperationError,
    store::{LegacySnapshot, Store},
};
use tower::ServiceExt;

fn fixture() -> (TempDir, Store, String, String) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    let token = |id| {
        store.register(Some(id)).unwrap().unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let a = token("garden-a");
    let b = token("water-b");
    store
        .update_named_control("forum", &json!({"enabled":true}), 0)
        .unwrap();
    (dir, store, a, b)
}
fn command(store: &Store, token: &str, text: &str, now: i64) -> Value {
    forum_bridge::submit(
        store,
        &json!({"id":"garden-a","clientVersion":"0.7.7","source":"ai","text":text}),
        Some("garden-a"),
        token,
        now,
    )
    .unwrap()
}
fn read(store: &Store, id: &str, after: u64, now: i64) -> (Vec<Value>, u64) {
    let mut cursor = after;
    let mut results = Vec::new();
    let mut parts = std::collections::HashMap::<String, (usize, String)>::new();
    loop {
        let page = store
            .announcement_page(Some(id), cursor, false, None, now)
            .unwrap();
        let announcements = page["announcements"].as_array().unwrap();
        for notice in announcements {
            assert!(notice["id"].as_u64().unwrap() > cursor);
            assert!(notice["id"].as_u64().unwrap() <= page["nextCursor"].as_u64().unwrap());
            assert!(notice["text"].as_str().unwrap().encode_utf16().count() <= 2000);
            if let Some(s) = notice["text"]
                .as_str()
                .unwrap()
                .strip_prefix("[forum-result]")
            {
                let chunk: Value = serde_json::from_str(s).unwrap();
                let entry = parts
                    .entry(chunk["request"].as_str().unwrap().to_owned())
                    .or_insert((0, String::new()));
                entry.0 += 1;
                assert_eq!(chunk["part"], entry.0);
                entry.1.push_str(chunk["payload"].as_str().unwrap());
                if chunk["part"] == chunk["parts"] {
                    results.push(serde_json::from_str(&entry.1).unwrap());
                }
            }
        }
        let next = page["nextCursor"].as_u64().unwrap();
        if next == cursor || announcements.len() < 20 {
            return (results, next);
        }
        cursor = next;
    }
}
async fn http(
    store: &Store,
    method: Method,
    path: &str,
    input: Value,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let app = router(App {
        store: store.clone(),
        ingestion: Some(Ingestion::new(store.clone())),
        release_id: None,
        release_root: None,
    });
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .oneshot(request.body(Body::from(input.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}

#[tokio::test]
async fn feedback_commands_post_reply_like_read_without_entering_either_admin_list() {
    let (_dir, s, a, b) = fixture();
    let (status,receipt)=http(&s,Method::POST,"/api/feedback",json!({"id":"garden-a","clientVersion":"0.7.7","source":"ai","text":"[send][title]薄荷观察[content]叶子长出了新芽 [like] 这里是正文"}),Some(&a)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(receipt["id"].as_u64().unwrap() > 0);
    assert!(receipt["createdAt"].is_string());
    let (responses, _) = read(
        &s,
        "garden-a",
        0,
        terrarium_server::ingestion::now_ms() + 86_400_001,
    );
    assert!(responses.is_empty());
    let now = terrarium_server::ingestion::now_ms();
    let (responses, cursor) = read(&s, "garden-a", 0, now);
    assert_eq!(responses.len(), 1);
    let post = &responses[0]["data"];
    assert_eq!(post["author"], "garden-a");
    assert_eq!(post["body"], "叶子长出了新芽 [like] 这里是正文");
    let id = post["id"].as_u64().unwrap();
    command(
        &s,
        &a,
        &format!("[reply][thread]{id}[content]我的薄荷也在发芽"),
        now + 1001,
    );
    command(&s, &a, &format!("[like][post]{id}"), now + 2002);
    command(&s, &a, &format!("[read][thread]{id}"), now + 3003);
    let (responses, cursor) = read(&s, "garden-a", cursor, now + 3003);
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[1]["data"]["likes"], 1);
    assert_eq!(responses[2]["data"]["thread"]["likes"], 1);
    assert_eq!(responses[2]["data"]["replies"].as_array().unwrap().len(), 1);
    assert_eq!(responses[2]["externalContent"], true);
    command(&s, &a, &format!("[unlike][post]{id}"), now + 4004);
    command(&s, &a, "[read][q]薄荷[limit]1", now + 5005);
    let (responses, _) = read(&s, "garden-a", cursor, now + 5005);
    assert_eq!(responses[0]["data"]["likes"], 0);
    assert_eq!(responses[1]["data"]["threads"][0]["id"], id);
    assert!(
        s.list_feedback(0).unwrap()["feedback"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        s.list_announcements().unwrap()["announcements"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(read(&s, "water-b", 0, now).0.is_empty());
    let (status, page) = http(
        &s,
        Method::GET,
        "/api/announcements?id=water-b",
        Value::Null,
        Some(&b),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page["announcements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["targets"] == json!(["water-b"]))
    );
    let public = http(
        &s,
        Method::GET,
        "/api/announcements/public",
        Value::Null,
        None,
    )
    .await
    .1;
    assert!(public["announcements"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn malformed_commands_stay_out_of_feedback_and_authentication_is_required() {
    let (_dir, s, a, _) = fixture();
    for (input, token) in [
        (json!({"text":"[send][title]test[content]body"}), None),
        (json!({"id":"garden-a","text":"[read]"}), Some("wrong")),
    ] {
        assert_eq!(
            http(&s, Method::POST, "/api/feedback", input, token)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let now = terrarium_server::ingestion::now_ms();
    for (index, text) in [
        "[send][title]没有正文",
        "[reply][thread]abc[content]test",
        "[read][limit]99",
        "[like][post]1[unexpected]x",
        "[help][content]extra",
    ]
    .iter()
    .enumerate()
    {
        command(&s, &a, text, now + index as i64 * 1001);
    }
    let (results, _) = read(&s, "garden-a", 0, now + 6000);
    assert_eq!(results.len(), 5);
    assert!(
        results
            .iter()
            .all(|r| r["ok"] == false && r["status"] == 400)
    );
    assert!(
        s.list_feedback(0).unwrap()["feedback"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        http(
            &s,
            Method::POST,
            "/api/feedback",
            json!({"text":"普通意见反馈"}),
            None
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        s.list_feedback(0).unwrap()["feedback"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn old_clients_receive_help_once_and_pending_notices_survive_export_restart_and_expire() {
    let (dir, s, a, _) = fixture();
    forum_bridge::prepare_poll(&s, "garden-a", &json!("0.7.7"), 0, 1000).unwrap();
    let (help, cursor) = read(&s, "garden-a", 0, 1000);
    assert_eq!(help.len(), 1);
    assert_eq!(help[0]["action"], "help");
    command(&s, &a, "[read]", 2000);
    let expected = read(&s, "garden-a", cursor, 2000);
    let exported = dir.path().join("export");
    s.export(&exported).unwrap();
    let imported = Store::open(dir.path().join("imported.sqlite")).unwrap();
    imported
        .import(LegacySnapshot::read(&exported).unwrap())
        .unwrap();
    assert_eq!(read(&imported, "garden-a", cursor, 2000), expected);
    drop(s);
    let s = Store::existing(dir.path().join("data.sqlite")).unwrap();
    assert_eq!(read(&s, "garden-a", cursor, 2000), expected);
    forum_bridge::prepare_poll(&s, "garden-a", &json!("0.7.7"), expected.1, 3000).unwrap();
    assert!(read(&s, "garden-a", 0, 3000).0.is_empty());
    forum_bridge::prepare_poll(&s, "water-b", &json!("0.8.7-bugfix.1"), 0, 3000).unwrap();
    assert!(read(&s, "water-b", 0, 3000).0.is_empty());
    command(&s, &a, "[read]", 4000);
    assert!(read(&s, "garden-a", 0, 86_404_000).0.is_empty());
}

#[test]
fn protocol_obeys_cloud_rate_limits_and_request_identifiers_prevent_duplicate_posts() {
    let (_dir, s, a, _) = fixture();
    let text = "[send][request]seed-1[title]观察记录[content]新叶";
    let first = command(&s, &a, text, 1000);
    let second = command(&s, &a, text, 1001);
    assert_eq!(first["id"], second["id"]);
    assert_eq!(second["duplicate"], true);
    let different = forum_bridge::submit(
        &s,
        &json!({"id":"garden-a","clientVersion":"0.7.7","text":text.replace("新叶","旧叶")}),
        Some("garden-a"),
        &a,
        2000,
    )
    .unwrap_err();
    assert_eq!(different.downcast::<OperationError>().unwrap().status, 409);
    command(&s, &a, "[send][title]第二篇[content]限流", 2001);
    let (results, _) = read(&s, "garden-a", 0, 2001);
    assert_eq!(results.len(), 2);
    assert_eq!(results[1]["status"], 429);
    s.update_named_control("forum", &json!({"enabled":false}), 3000)
        .unwrap();
    command(&s, &a, "[read]", 4000);
    let (results, _) = read(&s, "garden-a", 0, 4000);
    assert_eq!(results.last().unwrap()["status"], 503);
    assert!(
        s.list_feedback(0).unwrap()["feedback"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn large_unicode_results_are_chunked_with_valid_notice_lengths_and_cursors() {
    let (_dir, s, a, _) = fixture();
    let content = "🌱\"\\".repeat(300);
    command(
        &s,
        &a,
        &format!("[send][title]观察[content]{content}"),
        1000,
    );
    command(&s, &a, "[read][thread]1", 2001);
    let (results, _) = read(&s, "garden-a", 0, 2001);
    assert_eq!(results.len(), 2);
    assert_eq!(results[1]["data"]["thread"]["body"], content);
    assert!(forum_bridge::is_command(&json!({"text":" [send]broken"})));
    assert!(!forum_bridge::is_command(
        &json!({"text":"反馈例子：[send][title]主题"})
    ));
}

#[test]
fn acknowledged_notices_free_capacity_and_failed_cursor_writes_do_not_publish_posts() {
    let (_dir, s, a, _) = fixture();
    let mut now = 1000;
    loop {
        let result = forum_bridge::submit(
            &s,
            &json!({"id":"garden-a","clientVersion":"0.7.7","text":"[help]"}),
            Some("garden-a"),
            &a,
            now,
        );
        now += 1001;
        match result {
            Ok(_) => {}
            Err(e) => {
                assert_eq!(e.downcast::<OperationError>().unwrap().status, 429);
                break;
            }
        }
        assert!(now < 100_000);
    }
    let (_, cursor) = read(&s, "garden-a", 0, now);
    forum_bridge::prepare_poll(&s, "garden-a", &json!("0.8.7"), cursor, now).unwrap();
    command(&s, &a, "[help]", now + 1001);
    // A full main database must reject the reservation before the separate forum can write.
    let path = _dir.path().join("data.sqlite");
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch("CREATE TRIGGER refuse_cursor BEFORE UPDATE ON legacy_documents WHEN new.name='announcements.json' BEGIN SELECT RAISE(ABORT,'synthetic full database'); END;").unwrap();
    assert!(forum_bridge::submit(&s,&json!({"id":"garden-a","clientVersion":"0.7.7","text":"[send][title]记录[content]正文"}),Some("garden-a"),&a,now+2002).is_err());
    let result = terrarium_server::forum::execute(
        &s,
        &Method::GET,
        "threads",
        &Default::default(),
        &Value::Null,
        "",
        now + 2002,
    )
    .unwrap();
    assert!(result["threads"].as_array().unwrap().is_empty());
    assert!(
        s.list_feedback(0).unwrap()["feedback"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn concurrent_identical_requests_commit_only_one_post_and_one_protocol_receipt() {
    let (_dir, s, a, _) = fixture();
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let store = s.clone();
            let token = a.clone();
            std::thread::spawn(move || {
                command(
                    &store,
                    &token,
                    "[send][request]one-seed[title]新芽[content]观察记录",
                    1000,
                )
            })
        })
        .collect();
    let receipts: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert!(receipts.iter().all(|r| r["id"] == receipts[0]["id"]));
    assert_eq!(read(&s, "garden-a", 0, 1000).0.len(), 1);
    let result = terrarium_server::forum::execute(
        &s,
        &Method::GET,
        "threads",
        &Default::default(),
        &Value::Null,
        "",
        1000,
    )
    .unwrap();
    assert_eq!(result["threads"].as_array().unwrap().len(), 1);
}

#[test]
fn polling_during_commands_never_returns_notices_ahead_of_its_cursor() {
    let (_dir, s, a, _) = fixture();
    let store = s.clone();
    let worker = std::thread::spawn(move || {
        for n in 1..60 {
            let now = n * 1001;
            let cursor = store
                .announcement_page(Some("garden-a"), 0, false, None, now)
                .unwrap()["nextCursor"]
                .as_u64()
                .unwrap();
            forum_bridge::prepare_poll(&store, "garden-a", &json!("0.8.7"), cursor, now).unwrap();
            command(&store, &a, "[read]", now);
        }
    });
    for _ in 0..100 {
        read(&s, "garden-a", 0, 60_000);
    }
    worker.join().unwrap();
}

#[test]
fn full_thread_results_span_multiple_announcement_pages_without_losing_parts() {
    let (_dir, s, a, _) = fixture();
    for n in 0..4 {
        let input = json!({"id":"garden-a","clientVersion":"0.8.7","title":"观察记录","body":"\\\"".repeat(if n==0 {2000}else{1000})});
        let path = if n == 0 {
            "threads"
        } else {
            "threads/1/replies"
        };
        terrarium_server::forum::execute(
            &s,
            &Method::POST,
            path,
            &Default::default(),
            &input,
            &a,
            1000 + n * 10_000,
        )
        .unwrap();
    }
    command(&s, &a, "[read][thread]1", 50_000);
    let page = s
        .announcement_page(Some("garden-a"), 0, false, None, 50_000)
        .unwrap();
    assert_eq!(page["announcements"].as_array().unwrap().len(), 20);
    let (results, _) = read(&s, "garden-a", 0, 50_000);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["data"]["replies"].as_array().unwrap().len(), 3);
    assert_eq!(results[0]["data"]["thread"]["body"], "\\\"".repeat(2000));
}

#[test]
fn legacy_read_supports_recommendation_and_keeps_feedback_empty() {
    let (_dir, s, a, _) = fixture();
    command(&s, &a, "[send][title]New leaf[content]A seedling", 1000);
    let (_, cursor) = read(&s, "garden-a", 0, 1000);
    command(&s, &a, "[read][q]leaf[sort]recommended[limit]1", 2001);
    let (results, _) = read(&s, "garden-a", cursor, 2001);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["data"]["sort"], "recommended");
    assert_eq!(results[0]["data"]["threads"][0]["title"], "New leaf");
    assert!(
        s.list_feedback(0).unwrap()["feedback"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn protocol_commands_commit_only_used_ids_and_do_not_create_regular_announcements() {
    let (_dir, s, a, _) = fixture();
    let original = s
        .publish_announcement(&json!({"text":"Garden update"}), 1000)
        .unwrap();
    let mut cursor = original["id"].as_u64().unwrap();
    for (i, text) in [
        "[help]",
        "[read]",
        "[send][title]New leaf[content]A seedling",
        "[read][thread]1",
    ]
    .into_iter()
    .enumerate()
    {
        let now = 2000 + i as i64 * 1001;
        forum_bridge::prepare_poll(&s, "garden-a", &json!("0.8.7"), cursor, now).unwrap();
        let receipt = command(&s, &a, text, now);
        let pending = s.observed_client("garden-a").unwrap();
        let parts = pending["forumReplies"].as_array().unwrap();
        assert!(!parts.is_empty());
        assert_eq!(receipt["id"], cursor + 1);
        for (n, part) in parts.iter().enumerate() {
            assert_eq!(part["id"], cursor + 1 + n as u64);
        }
        let page = s
            .announcement_page(Some("garden-a"), cursor, false, None, now)
            .unwrap();
        assert_eq!(page["nextCursor"], cursor + parts.len() as u64);
        cursor = page["nextCursor"].as_u64().unwrap();
        assert_eq!(
            s.list_announcements().unwrap()["announcements"],
            json!([original])
        );
    }
    let next = s
        .publish_announcement(&json!({"text":"Next update"}), 10000)
        .unwrap();
    assert_eq!(next["id"], cursor + 1);
}
#[test]
fn automatic_help_keeps_existing_high_cursors_and_uses_only_actual_parts() {
    let (dir, s, _, _) = fixture();
    s.publish_announcement(&json!({"text":"Garden update"}), 1000)
        .unwrap();
    let c = rusqlite::Connection::open(dir.path().join("data.sqlite")).unwrap();
    c.execute("UPDATE legacy_documents SET content=CAST(json_set(content,'$.nextId',22227) AS BLOB) WHERE name='announcements.json'",[]).unwrap();
    forum_bridge::prepare_poll(&s, "garden-a", &json!("0.8.6"), 22226, 2000).unwrap();
    let pending = s.observed_client("garden-a").unwrap();
    let parts = pending["forumReplies"].as_array().unwrap();
    assert!(!parts.is_empty());
    assert_eq!(parts[0]["id"], 22227);
    let page = s
        .announcement_page(Some("garden-a"), 22226, false, None, 2000)
        .unwrap();
    assert_eq!(page["nextCursor"], 22226 + parts.len() as u64);
    let next = s
        .publish_announcement(&json!({"text":"Next update"}), 3000)
        .unwrap();
    assert_eq!(next["id"], 22227 + parts.len() as u64);
}
