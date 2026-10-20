use super::*;
use rnmdb_cli::LocalSession;
use serde_json::json;

fn put(namespace: &str, key: &str, value: Value, expected: u64) -> Operation {
    Operation::Put {
        namespace: namespace.into(),
        key: key.into(),
        value,
        expected_version: Some(expected),
    }
}
#[test]
fn transactions_rollback_and_tombstones_prevent_aba() {
    let mut db = Engine::new(LocalSession::memory().unwrap()).unwrap();
    db.transaction(&[put(
        "sample",
        "a'b",
        json!({"text":"x'); DROP TABLE api_records; --"}),
        0,
    )])
    .unwrap();
    let before = db.get("sample", "a'b").unwrap().unwrap();
    assert_eq!(before.version, 1);
    assert!(
        db.transaction(&[
            put("sample", "second", json!(2), 0),
            put("sample", "a'b", json!(3), 99)
        ])
        .is_err()
    );
    assert!(db.get("sample", "second").unwrap().is_none());
    db.transaction(&[Operation::Delete {
        namespace: "sample".into(),
        key: "a'b".into(),
        expected_version: Some(1),
    }])
    .unwrap();
    db.transaction(&[put("sample", "a'b", json!(4), 0)])
        .unwrap();
    assert_eq!(db.get("sample", "a'b").unwrap().unwrap().version, 3);
    assert!(
        db.transaction(&[put("sample", "a'b", json!(5), 1)])
            .is_err()
    );
}
#[test]
fn durable_reopen_wrong_key_and_backup() {
    let temp = tempfile::tempdir().unwrap();
    let key_path = temp.path().join("key");
    std::fs::write(&key_path, "42".repeat(32)).unwrap();
    let config = Config {
        listen: String::new(),
        path: temp.path().join("app.rnmdb"),
        page_key_file: key_path.clone(),
        principals_file: PathBuf::new(),
        backup_dir: temp.path().join("backups"),
    };
    {
        let mut db = Engine::open(&config).unwrap();
        db.transaction(&[put("docs", "a", json!({"name":"中文"}), 0)])
            .unwrap();
        let backup = db.backup(&config).unwrap();
        assert!(
            config
                .backup_dir
                .join(backup["backup"].as_str().unwrap())
                .is_file()
        );
    }
    assert_eq!(
        Engine::open(&config)
            .unwrap()
            .get("docs", "a")
            .unwrap()
            .unwrap()
            .value["name"],
        "中文"
    );
    std::fs::write(key_path, "43".repeat(32)).unwrap();
    assert!(Engine::open(&config).is_err());
}
#[test]
fn namespace_credentials_and_input_boundary() {
    let principal = Principal {
        token: "a".repeat(48),
        namespaces: vec!["docs".into()],
        admin: false,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {}", principal.token).parse().unwrap(),
    );
    let current = authorize(std::slice::from_ref(&principal), &headers).unwrap();
    assert!(allow(&current, "private").is_err());
    assert!(allow(&current, "docs").is_ok());
    assert!(authorize(&[principal], &HeaderMap::new()).is_err());
    assert!(validate_location("x';DROP", "x").is_err());
    assert!(validate_location("docs", "line\nbreak").is_err());
}
#[test]
fn prefix_pagination_skips_unrelated_leading_records() {
    let mut db = Engine::new(LocalSession::memory().unwrap()).unwrap();
    for key in ["a", "b", "c", "pre'1", "pre'2", "z"] {
        db.transaction(&[put("docs", key, json!(key), 0)]).unwrap();
    }
    let mut query = ListQuery {
        prefix: Some("pre'".into()),
        limit: Some(1),
        cursor: None,
    };
    let first = db.list("docs", &query).unwrap();
    assert_eq!(first["items"][0]["key"], "pre'1");
    query.cursor = Some(first["next_cursor"].as_str().unwrap().into());
    let second = db.list("docs", &query).unwrap();
    assert_eq!(second["items"][0]["key"], "pre'2");
    assert!(second["next_cursor"].is_null());
}
