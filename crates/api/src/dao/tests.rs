use super::*;
use reqwest::cookie::CookieStore;

fn make_test_cookie(
    name: &str,
    value: &str,
    domain: &str,
    path: &str,
    secure: bool,
    http_only: bool,
    expires: Option<u64>,
) -> Cookie {
    Cookie {
        domain: domain.to_string(),
        path: path.to_string(),
        secure,
        expires,
        name: name.to_string(),
        value: value.to_string(),
        http_only,
        same_site: 0,
    }
}

// 1. .youtube.com domain is preserved.
#[test]
fn test_domain_preserved() {
    let cookie = make_test_cookie("SID", "val", ".youtube.com", "/", true, true, None);
    let set_cookie = browser_cookie_to_set_cookie(&cookie);
    assert!(
        set_cookie.contains("Domain=.youtube.com"),
        "Expected Domain=.youtube.com in '{set_cookie}'"
    );

    let cookie_no_dot = make_test_cookie("SID", "val", "youtube.com", "/", true, true, None);
    let set_cookie_no_dot = browser_cookie_to_set_cookie(&cookie_no_dot);
    assert!(
        !set_cookie_no_dot.contains("Domain="),
        "Host-only cookie without leading dot must not have Domain= attribute in '{set_cookie_no_dot}'"
    );

    let cookie_empty_domain = make_test_cookie("SID", "val", "", "/", true, true, None);
    let set_cookie_empty = browser_cookie_to_set_cookie(&cookie_empty_domain);
    assert!(
        !set_cookie_empty.contains("Domain="),
        "Expected no Domain attribute in '{set_cookie_empty}'"
    );
}

// 2. / path is preserved.
#[test]
fn test_path_preserved() {
    let cookie = make_test_cookie("SID", "val", ".youtube.com", "/", true, true, None);
    let set_cookie = browser_cookie_to_set_cookie(&cookie);
    assert!(
        set_cookie.contains("Path=/"),
        "Expected Path=/ in '{set_cookie}'"
    );

    let cookie_sub = make_test_cookie("SID", "val", ".youtube.com", "/youtubei", true, true, None);
    let set_cookie_sub = browser_cookie_to_set_cookie(&cookie_sub);
    assert!(
        set_cookie_sub.contains("Path=/youtubei"),
        "Expected Path=/youtubei in '{set_cookie_sub}'"
    );
}

// 3. secure=true adds Secure.
#[test]
fn test_secure_true_adds_secure() {
    let cookie = make_test_cookie("SAPISID", "val", ".youtube.com", "/", true, false, None);
    let set_cookie = browser_cookie_to_set_cookie(&cookie);
    assert!(
        set_cookie.contains("Secure"),
        "Expected Secure in '{set_cookie}'"
    );
}

// 4. secure=false does not add Secure.
#[test]
fn test_secure_false_does_not_add_secure() {
    let cookie = make_test_cookie("PREF", "val", ".youtube.com", "/", false, false, None);
    let set_cookie = browser_cookie_to_set_cookie(&cookie);
    assert!(
        !set_cookie.contains("Secure"),
        "Did not expect Secure in '{set_cookie}'"
    );
}

// 5. http_only=true adds HttpOnly.
#[test]
fn test_http_only_true_adds_httponly() {
    let cookie = make_test_cookie("HSID", "val", ".youtube.com", "/", true, true, None);
    let set_cookie = browser_cookie_to_set_cookie(&cookie);
    assert!(
        set_cookie.contains("HttpOnly"),
        "Expected HttpOnly in '{set_cookie}'"
    );
}

// 6. http_only=false does not add HttpOnly.
#[test]
fn test_http_only_false_does_not_add_httponly() {
    let cookie = make_test_cookie("SAPISID", "val", ".youtube.com", "/", true, false, None);
    let set_cookie = browser_cookie_to_set_cookie(&cookie);
    assert!(
        !set_cookie.contains("HttpOnly"),
        "Did not expect HttpOnly in '{set_cookie}'"
    );
}

// 7. Cookie name/value are preserved.
#[test]
fn test_name_value_preserved() {
    let cookie = make_test_cookie(
        "__Secure-3PAPISID",
        "abc123XYZ_value!=",
        ".youtube.com",
        "/",
        true,
        false,
        None,
    );
    let set_cookie = browser_cookie_to_set_cookie(&cookie);
    assert!(
        set_cookie.starts_with("__Secure-3PAPISID=abc123XYZ_value!="),
        "Expected name and value preserved in '{set_cookie}'"
    );
}

// 8. Empty or unusual paths are handled safely.
#[test]
fn test_empty_or_unusual_paths_handled_safely() {
    let cookie_empty = make_test_cookie("c1", "v1", ".youtube.com", "", true, false, None);
    let set_cookie_empty = browser_cookie_to_set_cookie(&cookie_empty);
    assert!(
        set_cookie_empty.contains("Path=/"),
        "Expected empty path to default to Path=/, got '{set_cookie_empty}'"
    );

    let cookie_no_slash = make_test_cookie(
        "c2",
        "v2",
        ".youtube.com",
        "relative_path",
        true,
        false,
        None,
    );
    let set_cookie_no_slash = browser_cookie_to_set_cookie(&cookie_no_slash);
    assert!(
        set_cookie_no_slash.contains("Path=/"),
        "Expected unusual path without leading slash to default to Path=/, got '{set_cookie_no_slash}'"
    );
}

// 9. InvalidCookie error formatting contains only generic message and no cookie values
#[test]
fn test_invalid_cookie_error_contains_no_secrets() {
    let err = YError::InvalidCookie;
    let err_str = format!("{err}");
    assert_eq!(err_str, "Invalid Cookie");

    let startup_msg = error::startup_error_message(&err);
    assert_eq!(
        startup_msg,
        "Authentication error: invalid or expired session cookies"
    );
}

// 10. Integration test: verify that a .youtube.com cookie imported through
// browser_cookie_to_set_cookie is applicable to https://music.youtube.com/
#[test]
fn test_jar_integration_youtube_domain_applicable_to_music_youtube_com() {
    let jar = Jar::default();
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();

    let sapisid = make_test_cookie(
        "SAPISID",
        "test_sapisid_secret",
        ".youtube.com",
        "/",
        true,
        false,
        None,
    );
    let ssid = make_test_cookie(
        "SSID",
        "test_ssid_secret",
        ".youtube.com",
        "/",
        true,
        true,
        None,
    );
    let visitor = make_test_cookie(
        "VISITOR_INFO1_LIVE",
        "visitor_secret",
        "music.youtube.com",
        "/",
        false,
        false,
        None,
    );

    let sapisid_str = browser_cookie_to_set_cookie(&sapisid);
    let ssid_str = browser_cookie_to_set_cookie(&ssid);
    let visitor_str = browser_cookie_to_set_cookie(&visitor);

    let yt_url = Url::parse("https://youtube.com").unwrap();
    jar.add_cookie_str(&sapisid_str, &yt_url);
    jar.add_cookie_str(&ssid_str, &yt_url);
    jar.add_cookie_str(&visitor_str, &ytm_url);

    let header_val = jar
        .cookies(&ytm_url)
        .expect("Cookies must be found for music.youtube.com");
    let header_str = header_val.to_str().unwrap();

    assert!(
        header_str.contains("SAPISID=test_sapisid_secret"),
        "SAPISID cookie must be applicable to music.youtube.com: {header_str}"
    );
    assert!(
        header_str.contains("SSID=test_ssid_secret"),
        "SSID cookie must be applicable to music.youtube.com: {header_str}"
    );
    assert!(
        header_str.contains("VISITOR_INFO1_LIVE=visitor_secret"),
        "VISITOR_INFO1_LIVE cookie must be applicable to music.youtube.com: {header_str}"
    );
}

// 11. Chromium cookie filtering: rejects expired cookies, preserves session cookies (expires=None)
#[test]
fn test_chromium_filter_rejects_expired_and_preserves_session() {
    let now = 1_000_000u64;
    let valid_persistent = make_test_cookie(
        "c_valid",
        "v1",
        ".youtube.com",
        "/",
        true,
        false,
        Some(now + 1000),
    );
    let expired_persistent = make_test_cookie(
        "c_exp",
        "v2",
        ".youtube.com",
        "/",
        true,
        false,
        Some(now - 1000),
    );
    let session_cookie = make_test_cookie("c_sess", "v3", ".youtube.com", "/", true, false, None);
    let wrong_domain = make_test_cookie("c_wrong", "v4", ".example.com", "/", true, false, None);

    let raw = vec![
        valid_persistent,
        expired_persistent,
        session_cookie,
        wrong_domain,
    ];
    let filtered = filter_chromium_cookies(raw, now as i64);

    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].name, "c_valid");
    assert_eq!(filtered[1].name, "c_sess");
}

// 12. Chromium candidate selection logic: authenticated candidate preferred over anonymous candidate
#[test]
fn test_chromium_authenticated_candidate_preferred_over_anonymous() {
    let now = 1_000_000u64;

    // Browser 1 (e.g. Chrome) has only anonymous cookies
    let anon_browser = vec![
        make_test_cookie("PREF", "f1=val", ".youtube.com", "/", false, false, None),
        make_test_cookie(
            "VISITOR",
            "v123",
            "music.youtube.com",
            "/",
            false,
            false,
            None,
        ),
    ];
    let anon_filtered = filter_chromium_cookies(anon_browser, now as i64);

    // Browser 2 (e.g. Brave) has authenticated SAPISID
    let auth_browser = vec![
        make_test_cookie(
            "SAPISID",
            "brave_sapisid_token",
            ".youtube.com",
            "/",
            true,
            false,
            None,
        ),
        make_test_cookie(
            "SID",
            "brave_sid_token",
            ".youtube.com",
            "/",
            false,
            false,
            None,
        ),
    ];
    let auth_filtered = filter_chromium_cookies(auth_browser, now as i64);

    let anon_has_auth = anon_filtered
        .iter()
        .any(|c| c.name == "SAPISID" && !c.value.is_empty());
    assert!(!anon_has_auth, "Browser 1 must be anonymous");

    let auth_has_auth = auth_filtered
        .iter()
        .any(|c| c.name == "SAPISID" && !c.value.is_empty());
    assert!(auth_has_auth, "Browser 2 must have authentication");

    // Building jar from winning Browser 2
    let (jar, sapisid) = build_jar_from_chromium_cookies(&auth_filtered).unwrap();
    assert_eq!(sapisid, Some("brave_sapisid_token".to_string()));

    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(header_str.contains("SAPISID=brave_sapisid_token"));
    assert!(
        !header_str.contains("VISITOR"),
        "Must not merge cookies from Browser 1"
    );
}

// 13. Expired SAPISID is rejected and not treated as authenticated
#[test]
fn test_chromium_expired_sapisid_rejected() {
    let now = 1_000_000u64;
    let expired_auth = vec![make_test_cookie(
        "SAPISID",
        "expired_token",
        ".youtube.com",
        "/",
        true,
        false,
        Some(now - 100),
    )];
    let filtered = filter_chromium_cookies(expired_auth, now as i64);
    assert!(filtered.is_empty(), "Expired SAPISID must be rejected");

    let (jar, sapisid) = build_jar_from_chromium_cookies(&filtered).unwrap();
    assert_eq!(sapisid, None);
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    assert!(jar.cookies(&ytm_url).is_none());
}

// 14. Cross-family selection: Gecko anonymous + Chromium authenticated -> Chromium authenticated preferred
#[test]
fn test_cross_family_selection_gecko_anon_chromium_auth_prefers_chromium() {
    let gecko_jar = Jar::default();
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    gecko_jar.add_cookie_str("PREF=gecko_anon; Domain=.youtube.com; Path=/", &ytm_url);
    let gecko_candidate = Some((gecko_jar, None));

    let chromium_jar = Jar::default();
    chromium_jar.add_cookie_str(
        "SAPISID=chromium_sapisid; Domain=.youtube.com; Path=/; Secure",
        &ytm_url,
    );
    let chromium_candidate = Some((chromium_jar, Some("chromium_sapisid".to_string())));

    let (winning_jar, winning_sapisid) =
        select_cross_family_session(gecko_candidate, chromium_candidate);

    assert_eq!(winning_sapisid, Some("chromium_sapisid".to_string()));
    let header = winning_jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(header_str.contains("SAPISID=chromium_sapisid"));
    assert!(
        !header_str.contains("PREF=gecko_anon"),
        "Cookies across browser families must never be merged"
    );
}

// 15. Cross-family selection: Both authenticated -> Gecko authenticated preferred (deterministic family order)
#[test]
fn test_cross_family_selection_both_authenticated_prefers_gecko() {
    let gecko_jar = Jar::default();
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    gecko_jar.add_cookie_str(
        "SAPISID=gecko_sapisid; Domain=.youtube.com; Path=/; Secure",
        &ytm_url,
    );
    let gecko_candidate = Some((gecko_jar, Some("gecko_sapisid".to_string())));

    let chromium_jar = Jar::default();
    chromium_jar.add_cookie_str(
        "SAPISID=chromium_sapisid; Domain=.youtube.com; Path=/; Secure",
        &ytm_url,
    );
    let chromium_candidate = Some((chromium_jar, Some("chromium_sapisid".to_string())));

    let (winning_jar, winning_sapisid) =
        select_cross_family_session(gecko_candidate, chromium_candidate);

    assert_eq!(winning_sapisid, Some("gecko_sapisid".to_string()));
    let header = winning_jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(header_str.contains("SAPISID=gecko_sapisid"));
    assert!(!header_str.contains("chromium_sapisid"));
}

// 16. Cross-family selection: Both anonymous -> Gecko anonymous preferred
#[test]
fn test_cross_family_selection_both_anonymous_prefers_gecko() {
    let gecko_jar = Jar::default();
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    gecko_jar.add_cookie_str("PREF=gecko_anon; Domain=.youtube.com; Path=/", &ytm_url);
    let gecko_candidate = Some((gecko_jar, None));

    let chromium_jar = Jar::default();
    chromium_jar.add_cookie_str("PREF=chrom_anon; Domain=.youtube.com; Path=/", &ytm_url);
    let chromium_candidate = Some((chromium_jar, None));

    let (winning_jar, winning_sapisid) =
        select_cross_family_session(gecko_candidate, chromium_candidate);

    assert_eq!(winning_sapisid, None);
    let header = winning_jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(header_str.contains("PREF=gecko_anon"));
    assert!(!header_str.contains("chrom_anon"));
}

// 17. Cross-family selection: Gecko None + Chromium anonymous -> Chromium anonymous selected
#[test]
fn test_cross_family_selection_gecko_none_chromium_anon_returns_chromium() {
    let chromium_jar = Jar::default();
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    chromium_jar.add_cookie_str("PREF=chrom_anon; Domain=.youtube.com; Path=/", &ytm_url);
    let chromium_candidate = Some((chromium_jar, None));

    let (winning_jar, winning_sapisid) = select_cross_family_session(None, chromium_candidate);

    assert_eq!(winning_sapisid, None);
    let header = winning_jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(header_str.contains("PREF=chrom_anon"));
}
// 18. Server LOGGED_IN validation state
#[test]
fn test_validate_bootstrap_auth_state_matrix() {
    // SAPISID present + LOGGED_IN=true -> Ok
    assert!(validate_bootstrap_auth_state(true, true).is_ok());

    // SAPISID present + LOGGED_IN=false -> Err(InvalidCookie)
    match validate_bootstrap_auth_state(true, false) {
        Err(YError::InvalidCookie) => {}
        other => panic!("Expected InvalidCookie, got: {other:?}"),
    }

    // SAPISID absent + LOGGED_IN=false -> Ok (anonymous session)
    assert!(validate_bootstrap_auth_state(false, false).is_ok());

    // SAPISID absent + LOGGED_IN=true -> Ok
    assert!(validate_bootstrap_auth_state(false, true).is_ok());
}

// 19. Exact cookie domain applicability for music.youtube.com
#[test]
fn test_cookie_domain_applies_to_host() {
    let host = YTM_HOST; // music.youtube.com

    // Applicable domains
    assert!(cookie_domain_applies_to_host(".youtube.com", host));
    assert!(!cookie_domain_applies_to_host("youtube.com", host)); // host-only root domain excluded
    assert!(cookie_domain_applies_to_host("music.youtube.com", host));
    assert!(cookie_domain_applies_to_host(".music.youtube.com", host));

    // Non-applicable domains
    assert!(!cookie_domain_applies_to_host("studio.youtube.com", host));
    assert!(!cookie_domain_applies_to_host(".studio.youtube.com", host));
    assert!(!cookie_domain_applies_to_host("www.youtube.com", host));
    assert!(!cookie_domain_applies_to_host(".www.youtube.com", host));
    assert!(!cookie_domain_applies_to_host("notyoutube.com", host));
    assert!(!cookie_domain_applies_to_host("exampleyoutube.com", host));
    assert!(!cookie_domain_applies_to_host(
        "youtube.com.example.org",
        host
    ));
    assert!(!cookie_domain_applies_to_host("", host));
}

// 20. SAPISID coherence: Non-applicable domain SAPISID cannot override valid .youtube.com SAPISID
#[test]
fn test_sapisid_coherence_rejects_non_applicable_domain() {
    let now = 1_000_000u64;
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();

    let wrong_cookie = make_test_cookie(
        "SAPISID",
        "wrong_studio_token",
        "studio.youtube.com",
        "/",
        true,
        false,
        Some(now + 1000),
    );
    let correct_cookie = make_test_cookie(
        "SAPISID",
        "correct_ytm_token",
        ".youtube.com",
        "/",
        true,
        false,
        Some(now + 1000),
    );

    let cookies = vec![wrong_cookie, correct_cookie];
    let filtered = filter_chromium_cookies(cookies, now as i64);

    assert_eq!(
        filtered.len(),
        1,
        "studio.youtube.com cookie must be filtered out"
    );
    assert_eq!(filtered[0].value, "correct_ytm_token");

    let (jar, sapisid) = build_jar_from_chromium_cookies(&filtered).unwrap();
    assert_eq!(sapisid, Some("correct_ytm_token".to_string()));

    let header = jar.cookies(&ytm_url).expect("Cookies must be present");
    let header_str = header.to_str().unwrap();
    assert!(
        header_str.contains("SAPISID=correct_ytm_token"),
        "Cookie header must contain correct SAPISID: {header_str}"
    );
    assert!(
        !header_str.contains("wrong_studio_token"),
        "Cookie header must not contain non-applicable SAPISID: {header_str}"
    );
}

// 21. SAPISID coherence: notyoutube.com cookie is rejected
#[test]
fn test_sapisid_notyoutube_rejected() {
    let now = 1_000_000u64;
    let notyt_cookie = make_test_cookie(
        "SAPISID",
        "fake_token",
        "notyoutube.com",
        "/",
        true,
        false,
        Some(now + 1000),
    );
    let filtered = filter_chromium_cookies(vec![notyt_cookie], now as i64);
    assert!(filtered.is_empty());
}
// 22. Host-only cookie preservation: youtube.com is present for youtube.com but absent for music.youtube.com
#[test]
fn test_host_only_preservation() {
    let cookie = make_test_cookie(
        "ROOT_COOKIE",
        "root_val",
        "youtube.com",
        "/",
        true,
        false,
        None,
    );
    let jar = Jar::default();
    let default_url = Url::parse(YTM_DOMAIN).unwrap();
    let cookie_str = browser_cookie_to_set_cookie(&cookie);
    let origin_url = origin_url_for_cookie(&cookie.domain, &default_url);
    jar.add_cookie_str(&cookie_str, &origin_url);

    let yt_url = Url::parse("https://youtube.com/").unwrap();
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();

    let yt_header = jar
        .cookies(&yt_url)
        .expect("Cookie must be present for youtube.com");
    assert!(yt_header.to_str().unwrap().contains("ROOT_COOKIE=root_val"));

    assert!(
        jar.cookies(&ytm_url).is_none(),
        "Host-only cookie for youtube.com must not apply to music.youtube.com"
    );
}

// 23. Domain-cookie propagation: .youtube.com applies to both youtube.com and music.youtube.com
#[test]
fn test_domain_cookie_propagation() {
    let cookie = make_test_cookie(
        "DOMAIN_COOKIE",
        "dom_val",
        ".youtube.com",
        "/",
        true,
        false,
        None,
    );
    let jar = Jar::default();
    let default_url = Url::parse(YTM_DOMAIN).unwrap();
    let cookie_str = browser_cookie_to_set_cookie(&cookie);
    let origin_url = origin_url_for_cookie(&cookie.domain, &default_url);
    jar.add_cookie_str(&cookie_str, &origin_url);

    let yt_url = Url::parse("https://youtube.com/").unwrap();
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();

    let yt_header = jar
        .cookies(&yt_url)
        .expect("Cookie must be present for youtube.com");
    assert!(
        yt_header
            .to_str()
            .unwrap()
            .contains("DOMAIN_COOKIE=dom_val")
    );

    let ytm_header = jar
        .cookies(&ytm_url)
        .expect("Cookie must be present for music.youtube.com");
    assert!(
        ytm_header
            .to_str()
            .unwrap()
            .contains("DOMAIN_COOKIE=dom_val")
    );
}

// 24. Exact-host cookie: music.youtube.com applies to music.youtube.com and remains host-only
#[test]
fn test_exact_host_cookie() {
    let cookie = make_test_cookie(
        "EXACT_COOKIE",
        "exact_val",
        "music.youtube.com",
        "/",
        true,
        false,
        None,
    );
    let jar = Jar::default();
    let default_url = Url::parse(YTM_DOMAIN).unwrap();
    let cookie_str = browser_cookie_to_set_cookie(&cookie);
    let origin_url = origin_url_for_cookie(&cookie.domain, &default_url);
    jar.add_cookie_str(&cookie_str, &origin_url);

    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let yt_url = Url::parse("https://youtube.com/").unwrap();

    let ytm_header = jar
        .cookies(&ytm_url)
        .expect("Cookie must be present for music.youtube.com");
    assert!(
        ytm_header
            .to_str()
            .unwrap()
            .contains("EXACT_COOKIE=exact_val")
    );

    assert!(
        jar.cookies(&ytm_url).is_some() && jar.cookies(&yt_url).is_none(),
        "Host-only cookie for music.youtube.com must not apply to youtube.com"
    );
}

// 25. SAPISID path coherence: Path=/foo is rejected at / and Path=/ is selected
#[test]
fn test_sapisid_path_coherence() {
    let now = 1_000_000u64;
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();

    let wrong_path = make_test_cookie(
        "SAPISID",
        "wrong_path_token",
        ".youtube.com",
        "/foo",
        true,
        false,
        Some(now + 1000),
    );
    let correct_path = make_test_cookie(
        "SAPISID",
        "correct_path_token",
        ".youtube.com",
        "/",
        true,
        false,
        Some(now + 1000),
    );

    let (jar, sapisid) = build_jar_from_chromium_cookies(&[wrong_path, correct_path]).unwrap();
    assert_eq!(sapisid, Some("correct_path_token".to_string()));

    let header = jar.cookies(&ytm_url).expect("Cookies must be present");
    let header_str = header.to_str().unwrap();
    assert!(
        header_str.contains("SAPISID=correct_path_token"),
        "Header must contain path-matching SAPISID"
    );
    assert!(
        !header_str.contains("wrong_path_token"),
        "Header must not contain non-matching path SAPISID"
    );
}

// 26. Gecko and Chromium parity: Equivalent cookie inputs yield equivalent effective Jar & SAPISID
#[test]
fn test_gecko_and_chromium_parity() {
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();

    // Chromium representation
    let chrom_cookies = vec![
        make_test_cookie(
            "SAPISID",
            "shared_token",
            ".youtube.com",
            "/",
            true,
            false,
            None,
        ),
        make_test_cookie(
            "PREF",
            "shared_pref",
            "music.youtube.com",
            "/",
            false,
            false,
            None,
        ),
    ];
    let (chrom_jar, chrom_sapisid) = build_jar_from_chromium_cookies(&chrom_cookies).unwrap();

    // Gecko representation
    let gecko_cookies = vec![
        gecko::GeckoCookie {
            name: "SAPISID".to_string(),
            value: "shared_token".to_string(),
            host: ".youtube.com".to_string(),
            path: "/".to_string(),
            expiry: 0,
            is_secure: true,
            is_http_only: false,
            same_site: 0,
            origin_attributes: "".to_string(),
        },
        gecko::GeckoCookie {
            name: "PREF".to_string(),
            value: "shared_pref".to_string(),
            host: "music.youtube.com".to_string(),
            path: "/".to_string(),
            expiry: 0,
            is_secure: false,
            is_http_only: false,
            same_site: 0,
            origin_attributes: "".to_string(),
        },
    ];
    let (gecko_jar, gecko_sapisid) = gecko::build_jar_from_gecko_cookies(gecko_cookies).unwrap();

    assert_eq!(chrom_sapisid, gecko_sapisid);
    assert_eq!(chrom_sapisid, Some("shared_token".to_string()));

    let chrom_header = chrom_jar.cookies(&ytm_url).unwrap();
    let gecko_header = gecko_jar.cookies(&ytm_url).unwrap();

    let chrom_header_str = chrom_header.to_str().unwrap();
    let gecko_header_str = gecko_header.to_str().unwrap();

    assert!(chrom_header_str.contains("SAPISID=shared_token"));
    assert!(gecko_header_str.contains("SAPISID=shared_token"));
    assert!(chrom_header_str.contains("PREF=shared_pref"));
    assert!(gecko_header_str.contains("PREF=shared_pref"));
}

struct TestDirGuard(PathBuf);
impl Drop for TestDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct TestChromiumCookieRow<'a> {
    host_key: &'a str,
    name: &'a str,
    value: &'a str,
    path: &'a str,
    expires_utc: i64,
    is_secure: i64,
    is_httponly: i64,
}

fn create_test_chromium_db(db_path: &Path, cookies: &[TestChromiumCookieRow]) {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute(
        "CREATE TABLE cookies (
            host_key TEXT NOT NULL,
            name TEXT NOT NULL,
            value TEXT NOT NULL,
            encrypted_value BLOB DEFAULT '',
            path TEXT NOT NULL,
            expires_utc INTEGER NOT NULL,
            is_secure INTEGER NOT NULL,
            is_httponly INTEGER NOT NULL,
            has_expires INTEGER DEFAULT 1,
            is_persistent INTEGER DEFAULT 1,
            samesite INTEGER DEFAULT 0,
            source_port INTEGER DEFAULT 443,
            last_access_utc INTEGER DEFAULT 0
        )",
        [],
    )
    .unwrap();

    for c in cookies {
        conn.execute(
            "INSERT INTO cookies (host_key, name, value, encrypted_value, path, expires_utc, is_secure, is_httponly)
             VALUES (?1, ?2, ?3, X'01', ?4, ?5, ?6, ?7)",
            rusqlite::params![
                c.host_key,
                c.name,
                c.value,
                c.path,
                c.expires_utc,
                c.is_secure,
                c.is_httponly,
            ],
        )
        .unwrap();
    }
}

// 27. Regression: Brave Origin discovery fallback works and produces an isolated Chromium candidate
#[test]
fn test_brave_origin_discovery_and_isolated_candidate() {
    let temp_root = std::env::temp_dir().join(format!(
        "gytm_test_brave_origin_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let config_dir = temp_root.join("config");
    let _guard = TestDirGuard(temp_root.clone());

    let db_path = config_dir.join("BraveSoftware/Brave-Origin/Default/Network/Cookies");
    let cookies = vec![
        TestChromiumCookieRow {
            host_key: ".youtube.com",
            name: "SAPISID",
            value: "brave_origin_sapisid_token",
            path: "/",
            expires_utc: 0,
            is_secure: 1,
            is_httponly: 0,
        },
        TestChromiumCookieRow {
            host_key: "music.youtube.com",
            name: "PREF",
            value: "brave_origin_pref",
            path: "/",
            expires_utc: 0,
            is_secure: 0,
            is_httponly: 0,
        },
    ];
    create_test_chromium_db(&db_path, &cookies);

    let discovered = find_brave_origin_cookie_databases_with_root(Some(&config_dir));
    assert_eq!(discovered, vec![db_path.clone()]);

    let candidate = load_brave_origin_candidate_with_root(Some(&config_dir)).unwrap();
    assert!(candidate.is_some());
    let (jar, sapisid) = candidate.unwrap();
    assert_eq!(sapisid, Some("brave_origin_sapisid_token".to_string()));

    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(header_str.contains("SAPISID=brave_origin_sapisid_token"));
    assert!(header_str.contains("PREF=brave_origin_pref"));
}

// 28. Regression: Brave Origin cookies pass through Chromium expiry and domain filtering and jar construction
#[test]
fn test_brave_origin_expiry_and_domain_filtering() {
    let temp_root = std::env::temp_dir().join(format!(
        "gytm_test_brave_origin_filtering_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let config_dir = temp_root.join("config");
    let _guard = TestDirGuard(temp_root.clone());

    let db_path = config_dir.join("BraveSoftware/Brave-Origin/Profile 1/Cookies");

    // Microseconds since 1601-01-01 for year 2040 and year 2020
    let year_2040_chromium_ts: i64 = (2208988800 + 11644473600) * 1_000_000;
    let year_2020_chromium_ts: i64 = (1577836800 + 11644473600) * 1_000_000;

    let cookies = vec![
        // Valid session SAPISID
        TestChromiumCookieRow {
            host_key: ".youtube.com",
            name: "SAPISID",
            value: "valid_session_sapisid",
            path: "/",
            expires_utc: 0,
            is_secure: 1,
            is_httponly: 0,
        },
        // Valid future cookie
        TestChromiumCookieRow {
            host_key: "music.youtube.com",
            name: "PREF",
            value: "valid_future_pref",
            path: "/",
            expires_utc: year_2040_chromium_ts,
            is_secure: 0,
            is_httponly: 0,
        },
        // Expired cookie: must be dropped
        TestChromiumCookieRow {
            host_key: "music.youtube.com",
            name: "EXPIRED_COOKIE",
            value: "expired_value",
            path: "/",
            expires_utc: year_2020_chromium_ts,
            is_secure: 0,
            is_httponly: 0,
        },
        // Foreign domain cookie: must be dropped
        TestChromiumCookieRow {
            host_key: ".example.com",
            name: "FOREIGN_COOKIE",
            value: "foreign_value",
            path: "/",
            expires_utc: 0,
            is_secure: 0,
            is_httponly: 0,
        },
    ];
    create_test_chromium_db(&db_path, &cookies);

    let candidate = load_brave_origin_candidate_with_root(Some(&config_dir)).unwrap();
    assert!(candidate.is_some());
    let (jar, sapisid) = candidate.unwrap();
    assert_eq!(sapisid, Some("valid_session_sapisid".to_string()));

    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(header_str.contains("SAPISID=valid_session_sapisid"));
    assert!(header_str.contains("PREF=valid_future_pref"));
    assert!(!header_str.contains("EXPIRED_COOKIE"));
    assert!(!header_str.contains("FOREIGN_COOKIE"));
}

// 29. Regression: Expired SAPISID in Brave Origin is rejected and does not authenticate
#[test]
fn test_brave_origin_expired_sapisid_rejected() {
    let temp_root = std::env::temp_dir().join(format!(
        "gytm_test_brave_origin_expired_sapisid_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let config_dir = temp_root.join("config");
    let _guard = TestDirGuard(temp_root.clone());

    let db_path = config_dir.join("BraveSoftware/Brave-Origin/Default/Network/Cookies");
    let year_2020_chromium_ts: i64 = (1577836800 + 11644473600) * 1_000_000;

    let cookies = vec![
        // Expired SAPISID
        TestChromiumCookieRow {
            host_key: ".youtube.com",
            name: "SAPISID",
            value: "expired_sapisid_token",
            path: "/",
            expires_utc: year_2020_chromium_ts,
            is_secure: 1,
            is_httponly: 0,
        },
        // Valid anonymous cookie
        TestChromiumCookieRow {
            host_key: "music.youtube.com",
            name: "PREF",
            value: "anon_pref_token",
            path: "/",
            expires_utc: 0,
            is_secure: 0,
            is_httponly: 0,
        },
    ];
    create_test_chromium_db(&db_path, &cookies);

    let candidate = load_brave_origin_candidate_with_root(Some(&config_dir)).unwrap();
    assert!(candidate.is_some());
    let (jar, sapisid) = candidate.unwrap();
    // Must NOT be authenticated because SAPISID was expired
    assert_eq!(sapisid, None);

    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    let header = jar.cookies(&ytm_url).unwrap();
    let header_str = header.to_str().unwrap();
    assert!(!header_str.contains("SAPISID"));
    assert!(header_str.contains("PREF=anon_pref_token"));
}

// 30. Regression: Brave Origin candidate precedence relative to normal Chromium loaders
#[test]
fn test_brave_origin_candidate_precedence_matrix() {
    let temp_root = std::env::temp_dir().join(format!(
        "gytm_test_brave_origin_precedence_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let config_dir = temp_root.join("config");
    let _guard = TestDirGuard(temp_root.clone());

    let db_path = config_dir.join("BraveSoftware/Brave-Origin/Default/Network/Cookies");
    let cookies = vec![TestChromiumCookieRow {
        host_key: ".youtube.com",
        name: "SAPISID",
        value: "brave_origin_auth_token",
        path: "/",
        expires_utc: 0,
        is_secure: 1,
        is_httponly: 0,
    }];
    create_test_chromium_db(&db_path, &cookies);

    fn authed_normal_loader(_: Option<Vec<String>>) -> rookie::Result<Vec<Cookie>> {
        Ok(vec![make_test_cookie(
            "SAPISID",
            "normal_loader_auth_token",
            ".youtube.com",
            "/",
            true,
            false,
            None,
        )])
    }

    fn anon_normal_loader(_: Option<Vec<String>>) -> rookie::Result<Vec<Cookie>> {
        Ok(vec![make_test_cookie(
            "PREF",
            "normal_loader_anon_token",
            "music.youtube.com",
            "/",
            false,
            false,
            None,
        )])
    }

    // Precedence Case 1: Normal loader is authenticated -> normal loader candidate wins
    let loaders: [(&str, BrowserLoader); 1] = [("chrome", authed_normal_loader)];
    let (jar, sapisid) = load_chromium_candidate_with_loaders_and_root(&loaders, Some(&config_dir))
        .unwrap()
        .unwrap();
    assert_eq!(sapisid, Some("normal_loader_auth_token".to_string()));
    let ytm_url = Url::parse("https://music.youtube.com/").unwrap();
    assert!(
        jar.cookies(&ytm_url)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("normal_loader_auth_token")
    );

    // Precedence Case 2: Normal loader is only anonymous, Brave Origin is authenticated -> Brave Origin wins
    let loaders: [(&str, BrowserLoader); 1] = [("chrome", anon_normal_loader)];
    let (jar, sapisid) = load_chromium_candidate_with_loaders_and_root(&loaders, Some(&config_dir))
        .unwrap()
        .unwrap();
    assert_eq!(sapisid, Some("brave_origin_auth_token".to_string()));
    assert!(
        jar.cookies(&ytm_url)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("brave_origin_auth_token")
    );
}
