use super::*;
use reqwest::cookie::CookieStore;

struct TestCookieData<'a> {
    name: &'a str,
    value: &'a str,
    host: &'a str,
    path: &'a str,
    expiry: i64,
    is_secure: i64,
    is_http_only: i64,
    origin_attributes: &'a str,
}

fn create_test_db_at(db_path: &Path, cookies: &[TestCookieData]) {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute(
        "CREATE TABLE moz_cookies (
            id INTEGER PRIMARY KEY,
            originAttributes TEXT NOT NULL DEFAULT '',
            name TEXT,
            value TEXT,
            host TEXT,
            path TEXT,
            expiry INTEGER,
            lastAccessed INTEGER,
            creationTime INTEGER,
            isSecure INTEGER,
            isHttpOnly INTEGER,
            inBrowsingContextId INTEGER,
            sameSite INTEGER,
            rawSameSite INTEGER,
            schemeMap INTEGER
        )",
        [],
    )
    .unwrap();

    for (i, c) in cookies.iter().enumerate() {
        conn.execute(
            "INSERT INTO moz_cookies (id, originAttributes, name, value, host, path, expiry, lastAccessed, creationTime, isSecure, isHttpOnly, sameSite)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
            rusqlite::params![
                (i + 1) as i64,
                c.origin_attributes,
                c.name,
                c.value,
                c.host,
                c.path,
                c.expiry,
                1000i64,
                (i + 1) as i64,
                c.is_secure,
                c.is_http_only,
            ],
        )
        .unwrap();
    }
}

fn create_test_db(cookies: &[TestCookieData]) -> (PathBuf, TempSnapshotGuard) {
    static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_dir = std::env::temp_dir().join(format!(
        "gytm_gecko_test_db_{}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        count
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let guard = TempSnapshotGuard {
        temp_dir: temp_dir.clone(),
    };
    let db_path = temp_dir.join("cookies.sqlite");
    create_test_db_at(&db_path, cookies);
    (db_path, guard)
}

// 1. Valid persistent cookie: A non-expired .youtube.com SAPISID is imported and applies to https://music.youtube.com/
#[test]
fn test_valid_persistent_cookie_applies_to_music_youtube() {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let future_expiry = now_secs + 100_000;

    let test_cookies = vec![TestCookieData {
        name: "SAPISID",
        value: "valid_secret_123",
        host: ".youtube.com",
        path: "/",
        expiry: future_expiry,
        is_secure: 1,
        is_http_only: 0,
        origin_attributes: "",
    }];

    let (db_path, _guard) = create_test_db(&test_cookies);
    let res = load_gecko_cookies_from_db(&db_path).unwrap();
    assert!(res.is_some());
    let (jar, sapisid) = res.unwrap();
    assert_eq!(sapisid, Some("valid_secret_123".to_string()));

    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar
        .cookies(&ytm_url)
        .expect("Cookies must match YTM domain");
    assert!(
        header
            .to_str()
            .unwrap()
            .contains("SAPISID=valid_secret_123"),
        "SAPISID must be sent to music.youtube.com"
    );
}

// 2. Expired persistent cookie: Stale/expired authentication cookie is rejected directly in SQL
#[test]
fn test_expired_cookie_rejected() {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let future_expiry = now_secs + 100_000;
    let past_expiry = now_secs - 100_000;

    let test_cookies = vec![
        TestCookieData {
            name: "SAPISID",
            value: "active_token",
            host: ".youtube.com",
            path: "/",
            expiry: future_expiry,
            is_secure: 1,
            is_http_only: 0,
            origin_attributes: "",
        },
        TestCookieData {
            name: "ST-expired",
            value: "stale_token",
            host: ".youtube.com",
            path: "/",
            expiry: past_expiry,
            is_secure: 1,
            is_http_only: 0,
            origin_attributes: "",
        },
    ];

    let (db_path, _guard) = create_test_db(&test_cookies);
    let loaded = read_gecko_cookies(&db_path).unwrap();
    assert_eq!(
        loaded.len(),
        1,
        "Expired cookie must be filtered out by SQL"
    );
    assert_eq!(loaded[0].name, "SAPISID");
    assert_eq!(loaded[0].value, "active_token");
}

// 3. Session cookie: expiry = 0 is treated as a valid session cookie
#[test]
fn test_session_cookie_expiry_zero_valid() {
    let test_cookies = vec![TestCookieData {
        name: "SAPISID",
        value: "session_token",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 1,
        is_http_only: 0,
        origin_attributes: "",
    }];

    let (db_path, _guard) = create_test_db(&test_cookies);
    let loaded = read_gecko_cookies(&db_path).unwrap();
    assert_eq!(
        loaded.len(),
        1,
        "expiry=0 must be retained as session cookie"
    );
    assert_eq!(loaded[0].value, "session_token");
}

// 4. Millisecond expiry compatibility: modern Gecko storing 13-digit milliseconds is normalized
#[test]
fn test_millisecond_expiry_compatibility() {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let future_ms = (now_secs + 100_000) * 1000 + 123;
    let past_ms = (now_secs - 100_000) * 1000 + 456;

    let test_cookies = vec![
        TestCookieData {
            name: "SAPISID",
            value: "valid_ms_token",
            host: ".youtube.com",
            path: "/",
            expiry: future_ms,
            is_secure: 1,
            is_http_only: 0,
            origin_attributes: "",
        },
        TestCookieData {
            name: "ST-expired-ms",
            value: "stale_ms_token",
            host: ".youtube.com",
            path: "/",
            expiry: past_ms,
            is_secure: 1,
            is_http_only: 0,
            origin_attributes: "",
        },
    ];

    let (db_path, _guard) = create_test_db(&test_cookies);
    let loaded = read_gecko_cookies(&db_path).unwrap();
    assert_eq!(loaded.len(), 1, "Expired ms cookie must be filtered out");
    assert_eq!(loaded[0].name, "SAPISID");
    assert_eq!(loaded[0].value, "valid_ms_token");
}

// 5. Conflicting identities: Two cookies with different originAttributes both containing SAPISID produce an error
#[test]
fn test_conflicting_containers_return_error() {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let future_expiry = now_secs + 100_000;

    let test_cookies = vec![
        TestCookieData {
            name: "SAPISID",
            value: "token_container_default",
            host: ".youtube.com",
            path: "/",
            expiry: future_expiry,
            is_secure: 1,
            is_http_only: 0,
            origin_attributes: "",
        },
        TestCookieData {
            name: "SAPISID",
            value: "token_container_personal",
            host: ".youtube.com",
            path: "/",
            expiry: future_expiry,
            is_secure: 1,
            is_http_only: 0,
            origin_attributes: "^userContextId=1",
        },
    ];

    let (db_path, _guard) = create_test_db(&test_cookies);
    let result = load_gecko_cookies_from_db(&db_path);
    match result {
        Err(YError::ConflictingContainerIdentities) => {}
        other => panic!(
            "Expected ConflictingContainerIdentities error, got: {:?}",
            other
        ),
    }
}

// 6. Default identity: originAttributes="" works cleanly
#[test]
fn test_default_origin_identity_works() {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let future_expiry = now_secs + 100_000;

    let test_cookies = vec![
        TestCookieData {
            name: "SAPISID",
            value: "token_sapisid",
            host: ".youtube.com",
            path: "/",
            expiry: future_expiry,
            is_secure: 1,
            is_http_only: 0,
            origin_attributes: "",
        },
        TestCookieData {
            name: "SID",
            value: "token_sid",
            host: ".youtube.com",
            path: "/",
            expiry: future_expiry,
            is_secure: 0,
            is_http_only: 1,
            origin_attributes: "",
        },
    ];

    let (db_path, _guard) = create_test_db(&test_cookies);
    let (jar, sapisid) = load_gecko_cookies_from_db(&db_path).unwrap().unwrap();
    assert_eq!(sapisid, Some("token_sapisid".to_string()));

    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(header_str.contains("SAPISID=token_sapisid"));
    assert!(header_str.contains("SID=token_sid"));
}

// 7. Flags: Secure and HttpOnly semantics remain faithful
#[test]
fn test_flags_secure_and_httponly_semantics() {
    let cookie_plain = GeckoCookie {
        name: "test1".to_string(),
        value: "val1".to_string(),
        host: ".youtube.com".to_string(),
        path: "/".to_string(),
        expiry: 0,
        is_secure: false,
        is_http_only: false,
        same_site: 0,
        origin_attributes: "".to_string(),
    };
    let set_plain = cookie_to_set_cookie(
        &cookie_plain.name,
        &cookie_plain.value,
        &cookie_plain.host,
        &cookie_plain.path,
        cookie_plain.is_secure,
        cookie_plain.is_http_only,
    );
    assert!(!set_plain.contains("Secure"));
    assert!(!set_plain.contains("HttpOnly"));

    let cookie_flags = GeckoCookie {
        name: "test2".to_string(),
        value: "val2".to_string(),
        host: ".youtube.com".to_string(),
        path: "/".to_string(),
        expiry: 0,
        is_secure: true,
        is_http_only: true,
        same_site: 0,
        origin_attributes: "".to_string(),
    };
    let set_flags = cookie_to_set_cookie(
        &cookie_flags.name,
        &cookie_flags.value,
        &cookie_flags.host,
        &cookie_flags.path,
        cookie_flags.is_secure,
        cookie_flags.is_http_only,
    );
    assert!(set_flags.contains("Secure"));
    assert!(set_flags.contains("HttpOnly"));
}

// 8. Profiles.ini: Install Default= selection
#[test]
fn test_profiles_ini_install_default_selection() {
    let temp_dir = std::env::temp_dir().join(format!(
        "gytm_test_ini_install_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let base = temp_dir.join("base");
    std::fs::create_dir_all(&base).unwrap();

    let ini_content = r#"
[General]
StartWithLastProfile=1

[Profile0]
Name=other-profile
IsRelative=1
Path=Profiles/other.default

[InstallABC123]
Default=Profiles/target.default
Locked=1
"#;
    let profiles = parse_profiles_ini(ini_content, &base);
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0], base.join("Profiles/target.default"));
    assert_eq!(profiles[1], base.join("Profiles/other.default"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// 9. Profiles.ini: Profile Default=1 selection
#[test]
fn test_profiles_ini_profile_default_selection() {
    let temp_dir = std::env::temp_dir().join(format!(
        "gytm_test_ini_prof_def_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let base = temp_dir.join("base");
    std::fs::create_dir_all(&base).unwrap();

    let ini_content = r#"
[General]
StartWithLastProfile=1

[Profile0]
Name=p0
IsRelative=1
Path=Profiles/p0

[Profile1]
Name=p1
IsRelative=1
Path=Profiles/p1
Default=1

[Profile2]
Name=p2
IsRelative=1
Path=Profiles/p2
"#;
    let profiles = parse_profiles_ini(ini_content, &base);
    assert_eq!(profiles[0], base.join("Profiles/p1"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// 10. Profiles.ini: Relative vs Absolute profile path
#[test]
fn test_profiles_ini_relative_and_absolute_paths() {
    let temp_dir = std::env::temp_dir().join(format!(
        "gytm_test_ini_paths_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let base = temp_dir.join("base");
    std::fs::create_dir_all(&base).unwrap();

    let ini_content = r#"
[Profile0]
Name=p_rel
IsRelative=1
Path=relative_path
Default=1

[Profile1]
Name=p_abs
IsRelative=0
Path=/custom/abs/path
"#;
    let profiles = parse_profiles_ini(ini_content, &base);
    assert_eq!(profiles[0], base.join("relative_path"));
    assert_eq!(profiles[1], PathBuf::from("/custom/abs/path"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// 11. Deterministic profile discovery: fallback sorts alphabetically when profiles.ini is missing
#[test]
fn test_deterministic_profile_discovery_fallback() {
    let temp_dir = std::env::temp_dir().join(format!(
        "gytm_test_fallback_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let dir_z = temp_dir.join("z_profile");
    let dir_a = temp_dir.join("a_profile");
    let dir_m = temp_dir.join("m_profile");
    std::fs::create_dir_all(&dir_z).unwrap();
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_m).unwrap();

    std::fs::write(dir_z.join("cookies.sqlite"), b"").unwrap();
    std::fs::write(dir_a.join("cookies.sqlite"), b"").unwrap();
    std::fs::write(dir_m.join("cookies.sqlite"), b"").unwrap();

    let candidates = find_gecko_cookie_databases_in_base(&temp_dir);

    assert_eq!(candidates.len(), 3);
    assert_eq!(candidates[0], dir_a.join("cookies.sqlite"));
    assert_eq!(candidates[1], dir_m.join("cookies.sqlite"));
    assert_eq!(candidates[2], dir_z.join("cookies.sqlite"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// 12. Profiles.ini: Valid registered profile excludes unregistered disk directories
#[test]
fn test_profiles_ini_valid_candidates_excludes_unregistered_directories() {
    let temp_dir = std::env::temp_dir().join(format!(
        "gytm_test_registered_exclusive_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let active_dir = temp_dir.join("active.default");
    let stale_dir = temp_dir.join("stale.default");
    std::fs::create_dir_all(&active_dir).unwrap();
    std::fs::create_dir_all(&stale_dir).unwrap();

    std::fs::write(active_dir.join("cookies.sqlite"), b"").unwrap();
    std::fs::write(stale_dir.join("cookies.sqlite"), b"").unwrap();

    let ini_content = r#"
[Profile0]
Name=active
IsRelative=1
Path=active.default
Default=1
"#;
    std::fs::write(temp_dir.join("profiles.ini"), ini_content).unwrap();

    let candidates = find_gecko_cookie_databases_in_base(&temp_dir);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0], active_dir.join("cookies.sqlite"));
    assert!(
        !candidates.contains(&stale_dir.join("cookies.sqlite")),
        "Stale unregistered directory must not be included when profiles.ini has valid candidates"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// 13. Secure snapshot: Unix directory permissions 0700 (no group/other access)
#[cfg(unix)]
#[test]
fn test_snapshot_directory_permissions_unix() {
    use std::os::unix::fs::PermissionsExt;

    let test_cookies = vec![TestCookieData {
        name: "PREF",
        value: "f1=val",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 0,
        is_http_only: 0,
        origin_attributes: "",
    }];
    let (db_path, _guard) = create_test_db(&test_cookies);
    let (snap_path, _snap_guard) = snapshot_sqlite_db(&db_path).unwrap();

    let parent_dir = snap_path.parent().unwrap();
    let dir_meta = std::fs::metadata(parent_dir).unwrap();
    let dir_mode = dir_meta.permissions().mode() & 0o777;
    assert_eq!(
        dir_mode & 0o077,
        0,
        "Snapshot attempt directory must not grant group or other access (expected 0700, got {dir_mode:o})"
    );

    let root_dir = parent_dir.parent().unwrap();
    let root_meta = std::fs::metadata(root_dir).unwrap();
    let root_mode = root_meta.permissions().mode() & 0o777;
    assert_eq!(
        root_mode & 0o077,
        0,
        "Snapshot root directory must not grant group or other access (expected 0700, got {root_mode:o})"
    );

    let file_meta = std::fs::metadata(&snap_path).unwrap();
    let file_mode = file_meta.permissions().mode() & 0o777;
    assert_eq!(
        file_mode & 0o077,
        0,
        "Snapshot file must not grant group or other access (expected 0600, got {file_mode:o})"
    );
}

// 14. Snapshot retry: Per-attempt directory prevents stale WAL reuse if WAL disappears on retry
#[test]
fn test_snapshot_retry_isolates_wal_between_attempts() {
    let temp_dir = std::env::temp_dir().join(format!(
        "gytm_test_wal_isolation_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("cookies.sqlite");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute("CREATE TABLE t (id INTEGER);", []).unwrap();
    drop(conn);

    let (snap_path, guard) = snapshot_sqlite_db(&db_path).unwrap();
    let attempt_dir = snap_path.parent().unwrap();
    assert!(
        attempt_dir
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("attempt-"),
        "Snapshot must reside in a dedicated per-attempt directory"
    );

    // Simulate attempt-0 having left behind a WAL file in its own directory
    let stale_attempt_dir = guard.temp_dir.join("attempt-previous");
    std::fs::create_dir_all(&stale_attempt_dir).unwrap();
    std::fs::write(stale_attempt_dir.join("cookies.sqlite-wal"), b"stale wal").unwrap();

    // The active attempt directory does not retain the stale WAL from a previous attempt
    assert!(
        !attempt_dir.join("cookies.sqlite-wal").exists(),
        "Attempt directory must not inherit stale WAL files from earlier attempts"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

// 15. Anonymous container selection: Deterministic lexical order when default identity "" is absent
#[test]
fn test_anonymous_container_selection_deterministic() {
    let cookie_ctx2 = GeckoCookie {
        name: "PREF".to_string(),
        value: "ctx2_val".to_string(),
        host: ".youtube.com".to_string(),
        path: "/".to_string(),
        expiry: 0,
        is_secure: false,
        is_http_only: false,
        same_site: 0,
        origin_attributes: "^userContextId=2".to_string(),
    };
    let cookie_ctx1 = GeckoCookie {
        name: "PREF".to_string(),
        value: "ctx1_val".to_string(),
        host: ".youtube.com".to_string(),
        path: "/".to_string(),
        expiry: 0,
        is_secure: false,
        is_http_only: false,
        same_site: 0,
        origin_attributes: "^userContextId=1".to_string(),
    };

    // Order 1: ctx2 first, ctx1 second
    let list1 = vec![cookie_ctx2.clone(), cookie_ctx1.clone()];
    let selected1 = select_gecko_container_cookies(list1).unwrap();
    assert_eq!(selected1.len(), 1);
    assert_eq!(selected1[0].origin_attributes, "^userContextId=1");

    // Order 2: ctx1 first, ctx2 second
    let list2 = vec![cookie_ctx1, cookie_ctx2];
    let selected2 = select_gecko_container_cookies(list2).unwrap();
    assert_eq!(selected2.len(), 1);
    assert_eq!(selected2[0].origin_attributes, "^userContextId=1");
}

// 16. Gecko candidate selection: prefers authenticated candidate over anonymous candidate
#[test]
fn test_gecko_candidate_selection_prefers_authenticated_over_anonymous() {
    // DB 1 has only anonymous cookies
    let cookies_anon = vec![TestCookieData {
        name: "PREF",
        value: "anon_pref_val",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 0,
        is_http_only: 0,
        origin_attributes: "",
    }];
    let (db1_path, _guard1) = create_test_db(&cookies_anon);

    // DB 2 has authenticated SAPISID
    let cookies_auth = vec![TestCookieData {
        name: "SAPISID",
        value: "auth_token_profile_b",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 1,
        is_http_only: 0,
        origin_attributes: "",
    }];
    let (db2_path, _guard2) = create_test_db(&cookies_auth);

    // Passed in order [DB 1 (anon), DB 2 (auth)]
    let paths = vec![db1_path, db2_path];
    let result = load_gecko_cookies_from_paths(paths).unwrap();
    assert!(result.is_some(), "Must select a candidate");
    let (jar, sapisid) = result.unwrap();

    assert_eq!(
        sapisid,
        Some("auth_token_profile_b".to_string()),
        "Must prefer authenticated Profile B over anonymous Profile A"
    );

    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(header_str.contains("SAPISID=auth_token_profile_b"));
    assert!(
        !header_str.contains("anon_pref_val"),
        "Cookies from Profile A must not be merged into selected Profile B"
    );
}

// 17. Gecko candidate selection: falls back to first anonymous candidate when no profile is authenticated
#[test]
fn test_gecko_candidate_selection_falls_back_to_first_anonymous() {
    let cookies_anon1 = vec![TestCookieData {
        name: "VISITOR_INFO1_LIVE",
        value: "anon1_visitor",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 0,
        is_http_only: 0,
        origin_attributes: "",
    }];
    let (db1_path, _guard1) = create_test_db(&cookies_anon1);

    let cookies_anon2 = vec![TestCookieData {
        name: "PREF",
        value: "anon2_pref",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 0,
        is_http_only: 0,
        origin_attributes: "",
    }];
    let (db2_path, _guard2) = create_test_db(&cookies_anon2);

    let paths = vec![db1_path, db2_path];
    let result = load_gecko_cookies_from_paths(paths).unwrap();
    assert!(result.is_some());
    let (jar, sapisid) = result.unwrap();
    assert_eq!(sapisid, None);

    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(
        header_str.contains("anon1_visitor"),
        "First anonymous profile must be selected as fallback"
    );
    assert!(
        !header_str.contains("anon2_pref"),
        "Must never merge anonymous profiles"
    );
}

// 18. Gecko candidate selection: multiple authenticated profiles follow deterministic first-match precedence
#[test]
fn test_gecko_candidate_selection_multiple_authenticated_prefers_first_deterministic() {
    let cookies_auth1 = vec![TestCookieData {
        name: "SAPISID",
        value: "token_first_profile",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 1,
        is_http_only: 0,
        origin_attributes: "",
    }];
    let (db1_path, _guard1) = create_test_db(&cookies_auth1);

    let cookies_auth2 = vec![TestCookieData {
        name: "SAPISID",
        value: "token_second_profile",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 1,
        is_http_only: 0,
        origin_attributes: "",
    }];
    let (db2_path, _guard2) = create_test_db(&cookies_auth2);

    let paths = vec![db1_path, db2_path];
    let (jar, sapisid) = load_gecko_cookies_from_paths(paths).unwrap().unwrap();
    assert_eq!(sapisid, Some("token_first_profile".to_string()));

    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(header_str.contains("token_first_profile"));
    assert!(!header_str.contains("token_second_profile"));
}

// 19. Gecko SAPISID coherence: Non-applicable domain SAPISID cannot override valid .youtube.com SAPISID
#[test]
fn test_gecko_sapisid_coherence_rejects_non_applicable_domain() {
    let cookies = vec![
        TestCookieData {
            name: "SAPISID",
            value: "wrong_studio_token",
            host: "studio.youtube.com",
            path: "/",
            expiry: 0,
            is_secure: 1,
            is_http_only: 0,
            origin_attributes: "",
        },
        TestCookieData {
            name: "SAPISID",
            value: "correct_ytm_token",
            host: ".youtube.com",
            path: "/",
            expiry: 0,
            is_secure: 1,
            is_http_only: 0,
            origin_attributes: "",
        },
    ];
    let (db_path, _guard) = create_test_db(&cookies);
    let (jar, sapisid) = load_gecko_cookies_from_db(&db_path).unwrap().unwrap();
    assert_eq!(sapisid, Some("correct_ytm_token".to_string()));

    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).expect("Cookies must be present");
    let header_str = header.to_str().unwrap();
    assert!(header_str.contains("SAPISID=correct_ytm_token"));
    assert!(!header_str.contains("wrong_studio_token"));
}

// 20. Manual/external live profile integration test (ignored in default test suite)
#[tokio::test]
#[ignore = "requires live browser profile; set GYTM_TEST_GECKO_PROFILE to enable"]
async fn test_live_profile_manual() {
    let profile_var = std::env::var("GYTM_TEST_GECKO_PROFILE")
        .expect("GYTM_TEST_GECKO_PROFILE must be set for this ignored integration test");
    assert!(
        !profile_var.trim().is_empty(),
        "GYTM_TEST_GECKO_PROFILE must not be empty"
    );

    let db_path = PathBuf::from(profile_var);
    let res = load_gecko_cookies_from_db(&db_path).expect("Reading live profile must not fail");
    assert!(res.is_some(), "Live profile must contain cookies");
    let (jar, sapisid) = res.unwrap();
    assert!(
        sapisid.is_some(),
        "SAPISID must be extracted from live profile"
    );

    // Runs bootstrap initialization directly using the exact jar and sapisid extracted from the specified profile!
    let (dao, info) = crate::dao::YTDao::new_with_session_internal(jar, sapisid)
        .await
        .expect("YTDao initialization with specified session must succeed");

    assert_eq!(
        info.http_status,
        reqwest::StatusCode::OK,
        "HTTP bootstrap request must return 200 OK"
    );
    assert!(
        dao.sapisid.is_some(),
        "DAO must retain SAPISID from session"
    );
    assert!(
        !dao.innertube_api_key.is_empty(),
        "DAO must extract INNERTUBE_API_KEY"
    );
    assert!(
        !dao.client_version.is_empty(),
        "DAO must extract INNERTUBE_CLIENT_VERSION"
    );
    assert!(
        info.has_logged_in_marker,
        "Bootstrap response must confirm LOGGED_IN:true"
    );
}

// 16. Regression: Firefox under XDG config root discovery (<config>/mozilla/firefox/...)
#[test]
fn test_firefox_xdg_discovery() {
    let temp_root = std::env::temp_dir().join(format!(
        "gytm_test_firefox_xdg_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let config_dir = temp_root.join("config");
    let _guard = TempSnapshotGuard {
        temp_dir: temp_root.clone(),
    };

    let profile_dir = config_dir.join("mozilla/firefox/abcd1234.default-release");
    let db_path = profile_dir.join("cookies.sqlite");
    let cookies = vec![TestCookieData {
        name: "SAPISID",
        value: "firefox_xdg_sapisid_token",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 1,
        is_http_only: 0,
        origin_attributes: "",
    }];
    create_test_db_at(&db_path, &cookies);

    let ini_content = "[Profile0]
Name=default-release
IsRelative=1
Path=abcd1234.default-release
Default=1
";
    std::fs::write(config_dir.join("mozilla/firefox/profiles.ini"), ini_content).unwrap();

    let discovered = find_gecko_cookie_databases_with_roots(Some(&config_dir), None);
    assert_eq!(discovered, vec![db_path.clone()]);

    let loaded = load_gecko_cookies_with_roots(Some(&config_dir), None).unwrap();
    assert!(loaded.is_some());
    let (jar, sapisid) = loaded.unwrap();
    assert_eq!(sapisid, Some("firefox_xdg_sapisid_token".to_string()));
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    assert!(
        header
            .to_str()
            .unwrap()
            .contains("SAPISID=firefox_xdg_sapisid_token")
    );
}

// 17. Regression: Legacy Firefox discovery (<home>/.mozilla/firefox/...)
#[test]
fn test_firefox_legacy_discovery() {
    let temp_root = std::env::temp_dir().join(format!(
        "gytm_test_firefox_legacy_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let home_dir = temp_root.join("home");
    let _guard = TempSnapshotGuard {
        temp_dir: temp_root.clone(),
    };

    let profile_dir = home_dir.join(".mozilla/firefox/efgh5678.default");
    let db_path = profile_dir.join("cookies.sqlite");
    let cookies = vec![TestCookieData {
        name: "SAPISID",
        value: "firefox_legacy_sapisid_token",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 1,
        is_http_only: 0,
        origin_attributes: "",
    }];
    create_test_db_at(&db_path, &cookies);

    let ini_content = "[Profile0]
Name=default
IsRelative=1
Path=efgh5678.default
Default=1
";
    std::fs::write(home_dir.join(".mozilla/firefox/profiles.ini"), ini_content).unwrap();

    let discovered = find_gecko_cookie_databases_with_roots(None, Some(&home_dir));
    assert_eq!(discovered, vec![db_path.clone()]);

    let loaded = load_gecko_cookies_with_roots(None, Some(&home_dir)).unwrap();
    assert!(loaded.is_some());
    let (jar, sapisid) = loaded.unwrap();
    assert_eq!(sapisid, Some("firefox_legacy_sapisid_token".to_string()));
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    assert!(
        header
            .to_str()
            .unwrap()
            .contains("SAPISID=firefox_legacy_sapisid_token")
    );
}

// 18. Regression: LibreWolf XDG discovery still works (<config>/librewolf/...)
#[test]
fn test_librewolf_xdg_discovery() {
    let temp_root = std::env::temp_dir().join(format!(
        "gytm_test_librewolf_xdg_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let config_dir = temp_root.join("config");
    let _guard = TempSnapshotGuard {
        temp_dir: temp_root.clone(),
    };

    let profile_dir = config_dir.join("librewolf/profile.default");
    let db_path = profile_dir.join("cookies.sqlite");
    let cookies = vec![TestCookieData {
        name: "SAPISID",
        value: "librewolf_xdg_sapisid_token",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 1,
        is_http_only: 0,
        origin_attributes: "",
    }];
    create_test_db_at(&db_path, &cookies);

    let discovered = find_gecko_cookie_databases_with_roots(Some(&config_dir), None);
    assert!(discovered.contains(&db_path));

    let loaded = load_gecko_cookies_with_roots(Some(&config_dir), None).unwrap();
    assert!(loaded.is_some());
    let (jar, sapisid) = loaded.unwrap();
    assert_eq!(sapisid, Some("librewolf_xdg_sapisid_token".to_string()));
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    assert!(
        header
            .to_str()
            .unwrap()
            .contains("SAPISID=librewolf_xdg_sapisid_token")
    );
}

// 19. Regression: Zen Browser XDG discovery still works (<config>/zen/...)
#[test]
fn test_zen_browser_xdg_discovery() {
    let temp_root = std::env::temp_dir().join(format!(
        "gytm_test_zen_xdg_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let config_dir = temp_root.join("config");
    let _guard = TempSnapshotGuard {
        temp_dir: temp_root.clone(),
    };

    let profile_dir = config_dir.join("zen/default-zen");
    let db_path = profile_dir.join("cookies.sqlite");
    let cookies = vec![TestCookieData {
        name: "SAPISID",
        value: "zen_xdg_sapisid_token",
        host: ".youtube.com",
        path: "/",
        expiry: 0,
        is_secure: 1,
        is_http_only: 0,
        origin_attributes: "",
    }];
    create_test_db_at(&db_path, &cookies);

    let discovered = find_gecko_cookie_databases_with_roots(Some(&config_dir), None);
    assert!(discovered.contains(&db_path));

    let loaded = load_gecko_cookies_with_roots(Some(&config_dir), None).unwrap();
    assert!(loaded.is_some());
    let (jar, sapisid) = loaded.unwrap();
    assert_eq!(sapisid, Some("zen_xdg_sapisid_token".to_string()));
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    assert!(
        header
            .to_str()
            .unwrap()
            .contains("SAPISID=zen_xdg_sapisid_token")
    );
}

#[test]
fn test_gecko_profile_roots_cover_supported_linux_layouts() {
    let config_dir = PathBuf::from("/tmp/gytm-test-config");
    let home_dir = PathBuf::from("/tmp/gytm-test-home");

    assert_eq!(
        gecko_profile_roots(GeckoBrowser::Firefox, Some(&config_dir), Some(&home_dir)),
        vec![
            config_dir.join("mozilla/firefox"),
            home_dir.join(".mozilla/firefox"),
            home_dir.join("snap/firefox/common/.mozilla/firefox"),
            home_dir.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
            home_dir.join(".var/app/org.mozilla.firefox/config/mozilla/firefox"),
        ]
    );
    assert_eq!(
        gecko_profile_roots(GeckoBrowser::LibreWolf, Some(&config_dir), Some(&home_dir)),
        vec![
            config_dir.join("librewolf"),
            config_dir.join("librewolf/librewolf"),
            home_dir.join(".librewolf"),
            home_dir.join("snap/librewolf/common/.librewolf"),
            home_dir.join(".var/app/io.gitlab.librewolf-community/.librewolf"),
        ]
    );
    assert_eq!(
        gecko_profile_roots(GeckoBrowser::Zen, Some(&config_dir), Some(&home_dir)),
        vec![
            config_dir.join("zen"),
            config_dir.join("zen/zen"),
            home_dir.join(".zen"),
            home_dir.join(".var/app/app.zen_browser.zen/zen"),
            home_dir.join(".var/app/app.zen_browser.zen/.zen"),
            home_dir.join(".var/app/app.zen_browser.zen/config/zen"),
            home_dir.join(".var/app/io.github.zen_browser.zen/.zen"),
        ]
    );
    assert_eq!(
        gecko_profile_roots(GeckoBrowser::Cachy, Some(&config_dir), Some(&home_dir)),
        vec![home_dir.join(".cachy")]
    );
}

#[test]
fn test_gecko_browser_precedence_and_duplicate_database_handling() {
    let temp_root = std::env::temp_dir().join(format!(
        "gytm_test_gecko_precedence_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let config_dir = temp_root.join("config");
    let home_dir = temp_root.join("home");
    let _guard = TempSnapshotGuard {
        temp_dir: temp_root.clone(),
    };

    let browsers = [
        (GeckoBrowser::LibreWolf, "librewolf", "librewolf_token"),
        (GeckoBrowser::Zen, "zen", "zen_token"),
        (GeckoBrowser::Firefox, "mozilla/firefox", "firefox_token"),
    ];
    let mut expected = Vec::new();
    for (browser, relative_root, token) in browsers {
        let root = config_dir.join(relative_root);
        let profile = root.join("profile.default");
        let db_path = profile.join("cookies.sqlite");
        create_test_db_at(
            &db_path,
            &[TestCookieData {
                name: "SAPISID",
                value: token,
                host: ".youtube.com",
                path: "/",
                expiry: 0,
                is_secure: 1,
                is_http_only: 0,
                origin_attributes: "",
            }],
        );
        std::fs::write(
            root.join("profiles.ini"),
            "[Profile0]\nIsRelative=1\nPath=profile.default\nDefault=1\n",
        )
        .unwrap();
        expected.push(db_path);
        assert!(gecko_profile_roots(browser, Some(&config_dir), Some(&home_dir)).contains(&root));
    }

    let discovered = find_gecko_cookie_databases_with_roots(Some(&config_dir), Some(&home_dir));
    assert_eq!(discovered, expected);

    let (jar, sapisid) = load_gecko_cookies_with_roots(Some(&config_dir), Some(&home_dir))
        .unwrap()
        .unwrap();
    assert_eq!(sapisid, Some("librewolf_token".to_string()));
    let header = jar
        .cookies(&Url::parse("https://music.youtube.com/").unwrap())
        .unwrap();
    assert!(header.to_str().unwrap().contains("SAPISID=librewolf_token"));

    let shared_profile = temp_root.join("shared.default");
    let shared_db = shared_profile.join("cookies.sqlite");
    std::fs::create_dir_all(&shared_profile).unwrap();
    std::fs::write(&shared_db, b"").unwrap();
    let duplicate_config_dir = temp_root.join("duplicate-config");
    for root in [
        duplicate_config_dir.join("librewolf"),
        duplicate_config_dir.join("mozilla/firefox"),
    ] {
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("profiles.ini"),
            format!(
                "[Profile0]\nIsRelative=0\nPath={}\nDefault=1\n",
                shared_profile.display()
            ),
        )
        .unwrap();
    }
    assert_eq!(
        find_gecko_cookie_databases_with_roots(Some(&duplicate_config_dir), None),
        vec![shared_db]
    );
}
