use error::{YError, YResult};
use reqwest::{Url, cookie::Jar};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GeckoCookie {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) host: String,
    pub(crate) path: String,
    pub(crate) expiry: i64,
    pub(crate) is_secure: bool,
    pub(crate) is_http_only: bool,
    pub(crate) same_site: i64,
    pub(crate) origin_attributes: String,
}

pub(crate) fn cookie_to_set_cookie(
    name: &str,
    value: &str,
    domain: &str,
    path: &str,
    secure: bool,
    http_only: bool,
) -> String {
    let mut parts = Vec::new();
    parts.push(format!("{name}={value}"));

    if domain.starts_with('.') {
        parts.push(format!("Domain={domain}"));
    }

    let path = if path.is_empty() || !path.starts_with('/') {
        "/"
    } else {
        path
    };
    parts.push(format!("Path={path}"));

    if secure {
        parts.push("Secure".to_string());
    }

    if http_only {
        parts.push("HttpOnly".to_string());
    }

    parts.join("; ")
}

pub(crate) struct TempSnapshotGuard {
    temp_dir: PathBuf,
}

impl Drop for TempSnapshotGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

pub(crate) fn snapshot_sqlite_db(src_db: &Path) -> Result<(PathBuf, TempSnapshotGuard), YError> {
    if !src_db.is_file() {
        return Err(YError::DatabaseError(format!(
            "Gecko cookie database not found at {}",
            src_db.display()
        )));
    }

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let mut created_dir = None;
    for _ in 0..10 {
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = std::env::temp_dir().join(format!(
            "gytm_gecko_{}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            count
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            if builder.create(&candidate).is_ok() {
                created_dir = Some(candidate);
                break;
            }
        }
        #[cfg(not(unix))]
        {
            if std::fs::create_dir(&candidate).is_ok() {
                created_dir = Some(candidate);
                break;
            }
        }
    }

    let temp_dir = created_dir.ok_or_else(|| {
        YError::DatabaseError(
            "Failed to securely create temporary directory for cookie snapshot".to_string(),
        )
    })?;

    let guard = TempSnapshotGuard {
        temp_dir: temp_dir.clone(),
    };

    let parent = src_db.parent().unwrap_or(Path::new("."));
    let mut last_error = None;

    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let attempt_dir = temp_dir.join(format!("attempt-{attempt}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            if let Err(e) = builder.create(&attempt_dir) {
                last_error = Some(format!("Failed to create attempt directory: {e}"));
                continue;
            }
        }
        #[cfg(not(unix))]
        {
            if let Err(e) = std::fs::create_dir(&attempt_dir) {
                last_error = Some(format!("Failed to create attempt directory: {e}"));
                continue;
            }
        }

        let target_db = attempt_dir.join("cookies.sqlite");
        if let Err(e) = std::fs::copy(src_db, &target_db) {
            last_error = Some(format!("Failed to copy {}: {e}", src_db.display()));
            continue;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&target_db, std::fs::Permissions::from_mode(0o600));
        }

        let wal = parent.join("cookies.sqlite-wal");
        if wal.exists() {
            let target_wal = attempt_dir.join("cookies.sqlite-wal");
            if let Err(e) = std::fs::copy(&wal, &target_wal) {
                last_error = Some(format!("Failed to copy {}: {e}", wal.display()));
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&target_wal, std::fs::Permissions::from_mode(0o600));
            }
        }

        // Verify snapshot integrity using SQLite quick_check
        match rusqlite::Connection::open_with_flags(
            &target_db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        ) {
            Ok(conn) => {
                let check_result: Result<String, _> =
                    conn.query_row("PRAGMA quick_check(1);", [], |r| r.get(0));
                match check_result {
                    Ok(ref s) if s == "ok" => {
                        return Ok((target_db, guard));
                    }
                    Ok(other) => {
                        last_error = Some(format!("Snapshot failed integrity check: {other}"));
                    }
                    Err(e) => {
                        last_error = Some(format!("Snapshot integrity query failed: {e}"));
                    }
                }
            }
            Err(e) => {
                last_error = Some(format!("Failed to open snapshot: {e}"));
            }
        }
    }

    Err(YError::DatabaseError(last_error.unwrap_or_else(|| {
        "Failed to create valid SQLite snapshot after 3 attempts".to_string()
    })))
}

pub(crate) fn read_gecko_cookies(db_path: &Path) -> YResult<Vec<GeckoCookie>> {
    let (query_path, _guard) = snapshot_sqlite_db(db_path)?;

    let conn = rusqlite::Connection::open_with_flags(
        &query_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| {
        YError::DatabaseError(format!(
            "Failed to open database {}: {e}",
            db_path.display()
        ))
    })?;

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // Reject expired cookies directly in SQL. Normalizes both standard seconds (<= 10B)
    // and modern millisecond timestamps (> 10B) against the single `now_secs` timestamp.
    let mut stmt = conn.prepare(
        "SELECT name, value, host, path, expiry, isSecure, isHttpOnly, sameSite, originAttributes
         FROM moz_cookies
         WHERE (host = '.youtube.com' OR host = 'youtube.com' OR host = 'music.youtube.com' OR host = '.music.youtube.com' OR host LIKE '%.youtube.com')
           AND (expiry = 0 OR (CASE WHEN expiry > 10000000000 THEN expiry / 1000 ELSE expiry END) > ?1)
         ORDER BY creationTime ASC",
    )
    .map_err(|e| YError::DatabaseError(format!("Failed to prepare cookie query: {e}")))?;

    let rows = stmt
        .query_map(rusqlite::params![now_secs], |row| {
            Ok(GeckoCookie {
                name: row.get(0)?,
                value: row.get(1)?,
                host: row.get(2)?,
                path: row.get(3)?,
                expiry: row.get(4)?,
                is_secure: row.get::<_, i64>(5)? != 0,
                is_http_only: row.get::<_, i64>(6)? != 0,
                same_site: row.get(7)?,
                origin_attributes: row.get(8)?,
            })
        })
        .map_err(|e| YError::DatabaseError(format!("Failed to execute cookie query: {e}")))?;

    let mut cookies = Vec::new();
    for row in rows {
        let cookie: GeckoCookie =
            row.map_err(|e| YError::DatabaseError(format!("Failed to read cookie row: {e}")))?;
        if crate::dao::cookie_domain_applies_to_host(&cookie.host, crate::dao::YTM_HOST) {
            cookies.push(cookie);
        }
    }
    Ok(cookies)
}

pub(crate) fn select_gecko_container_cookies(
    cookies: Vec<GeckoCookie>,
) -> YResult<Vec<GeckoCookie>> {
    if cookies.is_empty() {
        return Ok(Vec::new());
    }

    let mut by_origin: BTreeMap<String, Vec<GeckoCookie>> = BTreeMap::new();
    for cookie in cookies {
        by_origin
            .entry(cookie.origin_attributes.clone())
            .or_default()
            .push(cookie);
    }

    // Identify which originAttributes identities contain an active SAPISID authentication cookie
    let mut authed_origins: Vec<String> = Vec::new();
    for (origin, group) in &by_origin {
        let (_, sapisid) = build_jar_from_gecko_cookies(group.clone())?;
        if sapisid.is_some() {
            authed_origins.push(origin.clone());
        }
    }

    if authed_origins.len() > 1 {
        return Err(YError::ConflictingContainerIdentities);
    }

    if authed_origins.len() == 1 {
        let origin = &authed_origins[0];
        return Ok(by_origin.remove(origin).unwrap_or_default());
    }

    // If no origin has SAPISID, prefer default origin identity ("") for logged-out mode
    if let Some(default_group) = by_origin.remove("") {
        Ok(default_group)
    } else if let Some((_, first_group)) = by_origin.into_iter().next() {
        // BTreeMap guarantees deterministic alphabetical ordering of identity keys
        Ok(first_group)
    } else {
        Ok(Vec::new())
    }
}

pub(crate) fn build_jar_from_gecko_cookies(
    cookies: Vec<GeckoCookie>,
) -> YResult<(Jar, Option<String>)> {
    let jar = Jar::default();
    let ytm_url = Url::parse(crate::dao::YTM_DOMAIN)?;

    for cookie in cookies {
        let cookie_str = cookie_to_set_cookie(
            &cookie.name,
            &cookie.value,
            &cookie.host,
            &cookie.path,
            cookie.is_secure,
            cookie.is_http_only,
        );
        let origin_url = crate::dao::origin_url_for_cookie(&cookie.host, &ytm_url);
        jar.add_cookie_str(&cookie_str, &origin_url);
    }

    let sapisid = crate::dao::sapisid_from_jar(&jar, &ytm_url);
    Ok((jar, sapisid))
}

pub(crate) fn load_gecko_cookies_from_db(db_path: &Path) -> YResult<Option<(Jar, Option<String>)>> {
    let raw_cookies = read_gecko_cookies(db_path)?;
    if raw_cookies.is_empty() {
        return Ok(None);
    }

    let selected = select_gecko_container_cookies(raw_cookies)?;
    if selected.is_empty() {
        return Ok(None);
    }

    let (jar, sapisid) = build_jar_from_gecko_cookies(selected)?;
    Ok(Some((jar, sapisid)))
}

/// Loads YouTube cookies across all discovered Gecko browser profiles.
///
/// Candidate selection policy:
/// 1. Evaluates Gecko candidate databases deterministically in resolved root order.
/// 2. Each candidate profile is loaded and evaluated independently; cookies across different profiles
///    are never merged.
/// 3. If a candidate contains an authenticated session (coherent `SAPISID`), it is immediately selected
///    and returned. If multiple profiles are authenticated, the first one encountered according to
///    the deterministic precedence hierarchy is selected.
/// 4. If no candidate contains an authenticated session, the first usable anonymous candidate (non-empty
///    cookies, no SAPISID) is returned for logged-out browsing.
/// 5. If no profiles contain YouTube cookies, returns `Ok(None)`.
pub(crate) fn load_gecko_cookies_with_roots(
    config_dir: Option<&Path>,
    home_dir: Option<&Path>,
) -> YResult<Option<(Jar, Option<String>)>> {
    load_gecko_cookies_from_paths(find_gecko_cookie_databases_with_roots(config_dir, home_dir))
}

pub(crate) fn load_gecko_cookies_from_paths<I>(paths: I) -> YResult<Option<(Jar, Option<String>)>>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut first_anonymous = None;

    for db_path in paths {
        if let Some((jar, sapisid)) = load_gecko_cookies_from_db(&db_path)?
            && let Some(candidate) =
                crate::dao::select_authenticated_candidate((jar, sapisid), &mut first_anonymous)
        {
            return Ok(Some(candidate));
        }
    }

    Ok(first_anonymous)
}

fn push_unique<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if !items.contains(&item) {
        items.push(item);
    }
}

pub(crate) fn parse_profiles_ini(ini_content: &str, base_dir: &Path) -> Vec<PathBuf> {
    let mut install_defaults = Vec::new();
    let mut profile_default = None;
    let mut other_profiles = Vec::new();

    let mut current_section = "";
    let mut current_path: Option<String> = None;
    let mut current_is_relative = true;
    let mut current_is_default = false;

    let finish_section = |section: &str,
                          path: Option<String>,
                          is_relative: bool,
                          is_default: bool,
                          prof_def: &mut Option<PathBuf>,
                          prof_others: &mut Vec<PathBuf>| {
        if section.starts_with("Profile")
            && let Some(p) = path
        {
            let resolved = if is_relative {
                base_dir.join(&p)
            } else {
                PathBuf::from(&p)
            };
            if is_default && prof_def.is_none() {
                *prof_def = Some(resolved);
            } else {
                prof_others.push(resolved);
            }
        }
    };

    for line in ini_content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            finish_section(
                current_section,
                current_path.take(),
                current_is_relative,
                current_is_default,
                &mut profile_default,
                &mut other_profiles,
            );
            current_section = &trimmed[1..trimmed.len() - 1];
            current_path = None;
            current_is_relative = true;
            current_is_default = false;
        } else if let Some((key, val)) = trimmed.split_once('=') {
            let key = key.trim();
            let val = val.trim();
            if current_section.starts_with("Install") && key == "Default" {
                let resolved = if Path::new(val).is_relative() {
                    base_dir.join(val)
                } else {
                    PathBuf::from(val)
                };
                install_defaults.push(resolved);
            } else if current_section.starts_with("Profile") {
                match key {
                    "Path" => current_path = Some(val.to_string()),
                    "IsRelative" => current_is_relative = val != "0",
                    "Default" => current_is_default = val == "1",
                    _ => {}
                }
            }
        }
    }
    finish_section(
        current_section,
        current_path,
        current_is_relative,
        current_is_default,
        &mut profile_default,
        &mut other_profiles,
    );

    let mut result = Vec::new();
    // Precedence 1: Install section Default path
    for path in install_defaults {
        push_unique(&mut result, path);
    }
    // Precedence 2: Profile section Default=1
    if let Some(path) = profile_default {
        push_unique(&mut result, path);
    }
    // Precedence 3: Other Profile sections
    for path in other_profiles {
        push_unique(&mut result, path);
    }

    result
}

pub(crate) fn find_gecko_cookie_databases_in_base(base: &Path) -> Vec<PathBuf> {
    if !base.exists() {
        return Vec::new();
    }

    let mut registered_candidates = Vec::new();
    let ini_path = base.join("profiles.ini");
    if ini_path.is_file()
        && let Ok(ini_content) = std::fs::read_to_string(&ini_path)
    {
        let profiles = parse_profiles_ini(&ini_content, base);
        for prof in profiles {
            let db_path = prof.join("cookies.sqlite");
            if db_path.is_file() {
                push_unique(&mut registered_candidates, db_path);
            }
        }
    }

    // Policy: If profiles.ini exists and yields at least one valid cookies.sqlite candidate,
    // use only those registered candidates.
    if !registered_candidates.is_empty() {
        registered_candidates
    } else {
        // Fallback: If profiles.ini missing or gave no valid candidates, scan subdirectories deterministically (alphabetically)
        let mut fallback_candidates = Vec::new();
        if let Ok(entries) = std::fs::read_dir(base) {
            let mut sub_dirs: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            sub_dirs.sort();

            for dir in sub_dirs {
                let db_path = dir.join("cookies.sqlite");
                if db_path.is_file() {
                    push_unique(&mut fallback_candidates, db_path);
                }
            }
        }

        let direct_db = base.join("cookies.sqlite");
        if direct_db.is_file() {
            push_unique(&mut fallback_candidates, direct_db);
        }

        fallback_candidates
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GeckoBrowser {
    LibreWolf,
    Zen,
    Firefox,
    Cachy,
}

// Keep browser precedence separate from installation-root precedence.
// LibreWolf, Zen, Firefox order matches the pre-regression Gecko resolver.
const GECKO_BROWSER_PRECEDENCE: [GeckoBrowser; 4] = [
    GeckoBrowser::LibreWolf,
    GeckoBrowser::Zen,
    GeckoBrowser::Firefox,
    GeckoBrowser::Cachy,
];

pub(crate) fn gecko_profile_roots(
    browser: GeckoBrowser,
    config_dir: Option<&Path>,
    home_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut add = |path: PathBuf| {
        push_unique(&mut roots, path);
    };

    match browser {
        GeckoBrowser::LibreWolf => {
            if let Some(config) = config_dir {
                add(config.join("librewolf"));
                add(config.join("librewolf/librewolf"));
            }
            if let Some(home) = home_dir {
                add(home.join(".librewolf"));
                add(home.join("snap/librewolf/common/.librewolf"));
                add(home.join(".var/app/io.gitlab.librewolf-community/.librewolf"));
            }
        }
        GeckoBrowser::Zen => {
            if let Some(config) = config_dir {
                add(config.join("zen"));
                add(config.join("zen/zen"));
            }
            if let Some(home) = home_dir {
                add(home.join(".zen"));
                add(home.join(".var/app/app.zen_browser.zen/zen"));
                add(home.join(".var/app/app.zen_browser.zen/.zen"));
                add(home.join(".var/app/app.zen_browser.zen/config/zen"));
                add(home.join(".var/app/io.github.zen_browser.zen/.zen"));
            }
        }
        GeckoBrowser::Firefox => {
            if let Some(config) = config_dir {
                add(config.join("mozilla/firefox"));
            }
            if let Some(home) = home_dir {
                add(home.join(".mozilla/firefox"));
                add(home.join("snap/firefox/common/.mozilla/firefox"));
                add(home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"));
                add(home.join(".var/app/org.mozilla.firefox/config/mozilla/firefox"));
            }
        }
        GeckoBrowser::Cachy => {
            if let Some(home) = home_dir {
                add(home.join(".cachy"));
            }
        }
    }

    roots
}

pub(crate) fn find_gecko_cookie_databases_with_roots(
    config_dir: Option<&Path>,
    home_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    for browser in GECKO_BROWSER_PRECEDENCE {
        for base in gecko_profile_roots(browser, config_dir, home_dir) {
            for db in find_gecko_cookie_databases_in_base(&base) {
                push_unique(&mut candidates, db);
            }
        }
    }

    candidates
}

#[cfg(test)]
mod tests;
