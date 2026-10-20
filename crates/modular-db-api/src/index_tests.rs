use super::*;
use crate::indexes::{Declaration, IndexQuery, Work as IndexWork};
use crate::tests::temporary;

fn put(key: &str, value: Value, expected: u64) -> Operation {
    Operation::Put {
        namespace: "docs".into(),
        key: key.into(),
        value,
        expected_version: Some(expected),
    }
}
fn declare(db: &mut Engine, pointer: &str) -> Result<Value, model::ApiError> {
    db.index(
        "docs",
        IndexWork::Declare(
            "by_value".into(),
            Declaration {
                pointer: pointer.into(),
            },
        ),
    )
}
fn query(db: &mut Engine, value: Value) -> Value {
    db.index(
        "docs",
        IndexWork::Query(
            "by_value".into(),
            IndexQuery {
                value,
                cursor: None,
                limit: None,
            },
        ),
    )
    .unwrap()
}
#[test]
fn backfill_distinguishes_scalar_types_and_ignores_missing_containers() {
    let (_temp, mut db) = temporary();
    let values = [
        json!(1),
        json!(1.0),
        json!("1"),
        json!(true),
        Value::Null,
        json!([]),
        json!({}),
    ];
    for (index, value) in values.into_iter().enumerate() {
        db.transaction(&[put(&index.to_string(), json!({"value": value}), 0)])
            .unwrap();
    }
    db.transaction(&[put("missing", json!({"other": 1}), 0)])
        .unwrap();
    declare(&mut db, "/value").unwrap();
    for (value, key) in [
        (json!(1), "0"),
        (json!(1.0), "1"),
        (json!("1"), "2"),
        (json!(true), "3"),
        (Value::Null, "4"),
    ] {
        let result = query(&mut db, value);
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert_eq!(result["items"][0]["key"], key);
    }
}
#[test]
fn updates_and_deletes_share_transaction_rollback_with_indexes() {
    let (_temp, mut db) = temporary();
    declare(&mut db, "/value").unwrap();
    db.transaction(&[put(
        "a'b",
        json!({"value":"x'); DROP TABLE api_records; --"}),
        0,
    )])
    .unwrap();
    assert!(
        db.transaction(&[
            put("a'b", json!({"value":"new"}), 1),
            put("a'b", json!({"value":"wrong"}), 99)
        ])
        .is_err()
    );
    assert!(
        query(&mut db, json!("new"))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        query(&mut db, json!("x'); DROP TABLE api_records; --"))["items"][0]["key"],
        "a'b"
    );
    db.transaction(&[put("a'b", json!({"value":"new"}), 1)])
        .unwrap();
    assert!(
        query(&mut db, json!("x'); DROP TABLE api_records; --"))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    db.transaction(&[Operation::Delete {
        namespace: "docs".into(),
        key: "a'b".into(),
        expected_version: Some(2),
    }])
    .unwrap();
    assert!(
        query(&mut db, json!("new"))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn index_redefinition_and_invalid_input_preserve_existing_definition() {
    let (_temp, mut db) = temporary();
    db.transaction(&[put("one", json!({"value":1,"nested":{"a/b~":2}}), 0)])
        .unwrap();
    declare(&mut db, "/value").unwrap();
    for pointer in ["value", "/bad~2", "/bad~", "/line\nbreak"] {
        assert!(declare(&mut db, pointer).is_err());
    }
    assert_eq!(query(&mut db, json!(1))["items"][0]["key"], "one");
    assert!(
        db.index(
            "docs",
            IndexWork::Query(
                "by_value".into(),
                IndexQuery {
                    value: json!({}),
                    cursor: None,
                    limit: None
                }
            )
        )
        .is_err()
    );
    declare(&mut db, "/nested/a~1b~0").unwrap();
    assert!(
        query(&mut db, json!(1))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(query(&mut db, json!(2))["items"][0]["key"], "one");
    db.index("docs", IndexWork::Remove("by_value".into()))
        .unwrap();
    assert!(
        db.index(
            "docs",
            IndexWork::Query(
                "by_value".into(),
                IndexQuery {
                    value: json!(2),
                    cursor: None,
                    limit: None
                }
            )
        )
        .is_err()
    );
}
#[test]
fn scalar_index_pagination_and_namespace_isolation() {
    let (_temp, mut db) = temporary();
    for key in ["a", "b", "c"] {
        db.transaction(&[put(key, json!({"value":42}), 0)]).unwrap();
    }
    db.transaction(&[Operation::Put {
        namespace: "private".into(),
        key: "secret".into(),
        value: json!({"value":42}),
        expected_version: Some(0),
    }])
    .unwrap();
    declare(&mut db, "/value").unwrap();
    let first = db
        .index(
            "docs",
            IndexWork::Query(
                "by_value".into(),
                IndexQuery {
                    value: json!(42),
                    cursor: None,
                    limit: Some(2),
                },
            ),
        )
        .unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    assert_eq!(first["next_cursor"], "b");
    let last = db
        .index(
            "docs",
            IndexWork::Query(
                "by_value".into(),
                IndexQuery {
                    value: json!(42),
                    cursor: Some("b".into()),
                    limit: Some(2),
                },
            ),
        )
        .unwrap();
    assert_eq!(last["items"][0]["key"], "c");
    assert!(last["next_cursor"].is_null());
    assert!(
        db.index(
            "private",
            IndexWork::Query(
                "by_value".into(),
                IndexQuery {
                    value: json!(42),
                    cursor: None,
                    limit: None
                }
            )
        )
        .is_err()
    );
}
#[test]
fn declaration_limit_allows_replacing_an_existing_index() {
    let (_temp, mut db) = temporary();
    for i in 0..32 {
        db.index(
            "docs",
            IndexWork::Declare(
                format!("index_{i}"),
                Declaration {
                    pointer: "/value".into(),
                },
            ),
        )
        .unwrap();
    }
    assert!(declare(&mut db, "/value").is_err());
    db.index(
        "docs",
        IndexWork::Declare(
            "index_0".into(),
            Declaration {
                pointer: "/other".into(),
            },
        ),
    )
    .unwrap();
    assert_eq!(
        db.index("docs", IndexWork::List).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        32
    );
}
#[tokio::test]
async fn index_routes_enforce_namespace_credentials() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let token = "a".repeat(48);
    let (actor, _receiver) = mpsc::channel(1);
    let app = router(App {
        actor,
        principals: Arc::new(vec![Principal {
            token: token.clone(),
            namespaces: vec!["docs".into()],
            admin: false,
        }]),
    });
    let cases = [
        ("GET", "/v1/namespaces/private/indexes", ""),
        (
            "PUT",
            "/v1/namespaces/private/indexes/by_value",
            "{\"pointer\":\"/value\"}",
        ),
        ("DELETE", "/v1/namespaces/private/indexes/by_value", ""),
        (
            "POST",
            "/v1/namespaces/private/indexes/by_value/query",
            "{\"value\":1}",
        ),
    ];
    for (method, path, body) in cases {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    let request = Request::builder()
        .uri("/v1/namespaces/docs/indexes")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}
