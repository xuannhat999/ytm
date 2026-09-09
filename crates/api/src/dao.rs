use data::app::{PlayListPrivacy, Song};
use error::{YError, YResult, log_to_file};
use reqwest::{
    Client, Url,
    cookie::Jar,
    header::{HeaderMap, HeaderValue},
};
use rookie::common::enums::Cookie;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    gecko,
    request::{
        ActionsContent, BrowseIdRequest, CreatePlaylistRequest, GetContinuationRequest,
        GetRelatedSongsRequest, PlaylistIdRequest, QueryRequest, QueryWithParamsRequest,
        RequestClient, RequestContext, SaveAlbumRequest, SaveUnsaveListRequest, TargetContent,
        TargetRequest, VideoIdRequest,
    },
};

pub struct YTDao {
    pub http: Client,
    pub sapisid: Option<String>,
    pub innertube_api_key: String,
    pub client_version: String,
}

pub(crate) const YTM_HOST: &str = "music.youtube.com";
pub(crate) const YTM_DOMAIN: &str = "https://music.youtube.com";

pub(crate) fn cookie_domain_applies_to_host(cookie_domain: &str, host: &str) -> bool {
    if let Some(domain) = cookie_domain.strip_prefix('.') {
        if domain.is_empty() {
            return false;
        }
        host.eq_ignore_ascii_case(domain)
            || host
                .to_ascii_lowercase()
                .strip_suffix(&domain.to_ascii_lowercase())
                .is_some_and(|prefix| prefix.ends_with('.'))
    } else {
        host.eq_ignore_ascii_case(cookie_domain)
    }
}

pub(crate) fn origin_url_for_cookie(host: &str, default_url: &Url) -> Url {
    let origin_host = host.trim_start_matches('.');
    if !origin_host.is_empty() {
        Url::parse(&format!("https://{origin_host}/")).unwrap_or_else(|_| default_url.clone())
    } else {
        default_url.clone()
    }
}

pub(crate) fn sapisid_from_jar(jar: &Jar, url: &Url) -> Option<String> {
    use reqwest::cookie::CookieStore;
    let header = jar.cookies(url)?;
    let header_str = header.to_str().ok()?;
    for pair in header_str.split(';') {
        let trimmed = pair.trim();
        if let Some((name, val)) = trimmed.split_once('=') {
            let value = val.trim();
            if name.trim() == "SAPISID" && !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

impl YTDao {
    pub async fn new() -> YResult<Self> {
        let (jar, sapisid) = load_cookies()?;
        let dao = Self::new_with_session_internal(jar, sapisid).await?;
        Ok(dao)
    }

    pub(crate) async fn new_with_session_internal(
        jar: Jar,
        sapisid: Option<String>,
    ) -> YResult<Self> {
        let http = Client::builder()
            .cookie_provider(Arc::new(jar))
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()?;

        let response = http.get(YTM_DOMAIN).send().await?;
        log_to_file(format!(
            "Init client response status: {}",
            response.status()
        ));
        let response_text = response.text().await?;
        let has_auth = sapisid.is_some();
        let innertube_api_key = extract_between(&response_text, "INNERTUBE_API_KEY\":\"", "\"")
            .ok_or_else(|| {
                if has_auth {
                    YError::InvalidCookie
                } else {
                    YError::InvalidResponse(
                        "YouTube Music bootstrap data (INNERTUBE_API_KEY)".to_string(),
                    )
                }
            })?;

        let client_version = extract_between(&response_text, "INNERTUBE_CLIENT_VERSION\":\"", "\"")
            .ok_or_else(|| {
                if has_auth {
                    YError::InvalidCookie
                } else {
                    YError::InvalidResponse(
                        "YouTube Music bootstrap data (INNERTUBE_CLIENT_VERSION)".to_string(),
                    )
                }
            })?;

        let has_logged_in_marker = response_text.contains("\"LOGGED_IN\":true");
        log_to_file(format!("Has auth: {}", has_auth));
        log_to_file(format!("Has logged_in marker: {}", has_logged_in_marker));

        if !has_logged_in_marker {
            return Ok(Self {
                http,
                sapisid: None,
                innertube_api_key,
                client_version,
            });
        }

        let dao = Self {
            http,
            sapisid,
            innertube_api_key,
            client_version,
        };

        Ok(dao)
    }

    // This function is adapted from: https://github.com/ccgauche/ytermusic.git
    // Original source: https://github.com/ccgauche/ytermusic/blob/master/crates/ytpapi2/src/lib.rs
    fn compute_sapi_hash(&self, sapisid: &str) -> String {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut hasher = sha1_smol::Sha1::new();
        let message = format!("{timestamp} {sapisid} {YTM_DOMAIN}");
        hasher.update(message.as_bytes());
        let result = hasher.digest();
        let hex_hash = result.to_string();
        format!("{}_{}", timestamp, hex_hash)
    }

    // This function is adapted from: https://github.com/ccgauche/ytermusic.git
    // Original source: https://github.com/ccgauche/ytermusic/blob/master/crates/ytpapi2/src/lib.rs
    pub fn get_api_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        headers.insert("Origin", HeaderValue::from_static(YTM_DOMAIN));
        headers.insert("X-Goog-AuthUser", HeaderValue::from_static("0"));
        if let Some(ref sapisid) = self.sapisid {
            let auth_val = format!("SAPISIDHASH {}", self.compute_sapi_hash(sapisid));
            headers.insert("Authorization", HeaderValue::from_str(&auth_val).unwrap());
        }
        headers
    }

    fn api_url(&self, endpoint: &str) -> String {
        format!(
            "{}/youtubei/v1/{}?key={}&alt=json",
            YTM_DOMAIN, endpoint, self.innertube_api_key
        )
    }

    fn get_context(&self) -> RequestContext<'_> {
        RequestContext {
            client: RequestClient {
                client_name: "WEB_REMIX",
                client_version: &self.client_version,
            },
        }
    }

    pub async fn get_raw_lists(&self) -> YResult<String> {
        let url = self.api_url("browse");

        let body = BrowseIdRequest {
            context: self.get_context(),
            browse_id: "FEmusic_library_landing",
        };
        let response = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?
            .text()
            .await?;
        Ok(response)
    }

    pub async fn get_continuation_raw(&self, token: &str) -> YResult<String> {
        let url = self.api_url("browse");
        let body = GetContinuationRequest {
            context: self.get_context(),
            continuation: token,
        };
        let response = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?
            .text()
            .await?;

        Ok(response)
    }
    pub async fn get_songs_raw(&self, browse_id: &str) -> YResult<String> {
        let url = self.api_url("browse");
        let body = BrowseIdRequest {
            context: self.get_context(),
            browse_id,
        };
        let text = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?
            .text()
            .await?;
        Ok(text)
    }

    pub async fn search_with_params_raw(&self, query: &str, rtype: u8) -> YResult<String> {
        let params = if rtype == 1 {
            "EgWKAQIIAWoMEAQQAxAFEAkQEBAK" // SONG
        } else {
            "EgWKAQIYAWoMEAQQAxAFEAkQEBAK" // ALBUM
        };
        let url = self.api_url("search");

        let body = QueryWithParamsRequest {
            context: self.get_context(),
            query,
            params,
        };
        let response = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?
            .text()
            .await?;
        Ok(response)
    }
    pub async fn search_raw(&self, query: &str) -> YResult<String> {
        let body = QueryRequest {
            context: self.get_context(),
            query,
        };
        let url = self.api_url("search");
        let response = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?
            .text()
            .await?;
        Ok(response)
    }
    pub async fn get_params_raw(&self, video_id: &str) -> YResult<String> {
        let url = self.api_url("next");
        let body = VideoIdRequest {
            context: self.get_context(),
            video_id,
        };
        let response = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?
            .text()
            .await?;
        Ok(response)
    }

    pub async fn create_playlist_raw(
        &self,
        title: &str,
        desc: &str,
        privacy: PlayListPrivacy,
    ) -> YResult<String> {
        let url = self.api_url("playlist/create");
        let body = CreatePlaylistRequest {
            context: self.get_context(),
            title,
            params: "KAA%3D",
            description: if desc.is_empty() { None } else { Some(desc) },
            privacy_status: if privacy == PlayListPrivacy::Private {
                None
            } else {
                Some(privacy)
            },
        };
        let response = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?
            .text()
            .await?;
        Ok(response)
    }

    pub async fn get_related_songs_raw(&self, playlist_id: &str, params: &str) -> YResult<String> {
        let url = self.api_url("next");
        let body = GetRelatedSongsRequest {
            context: self.get_context(),
            playlist_id,
            params,
            tuner_setting_value: "AUTOMIX_SETTING_NORMAL",
        };
        let response = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?
            .text()
            .await?;
        Ok(response)
    }

    pub async fn save_album_raw(&self, playlist_id: &str) -> YResult<()> {
        let url = self.api_url("like/like");
        let body = SaveAlbumRequest {
            context: self.get_context(),
            target: TargetContent {
                playlist_id: Some(playlist_id),
                video_id: None,
            },
            status: "LIKE",
        };

        let status = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?
            .status()
            .is_success();
        if status {
            Ok(())
        } else {
            Err(YError::BadStatus(String::from("Unsave custom playlist")))
        }
    }

    pub async fn unsave_cus_playlist_raw(&self, playlist_id: &str) -> YResult<()> {
        let url = self.api_url("playlist/delete");

        let body = PlaylistIdRequest {
            context: self.get_context(),
            playlist_id,
        };
        let status = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?
            .status()
            .is_success();
        if status {
            Ok(())
        } else {
            Err(YError::BadStatus(String::from("Unsave custom playlist")))
        }
    }

    pub async fn unsave_album_raw(&self, playlist_id: &str) -> YResult<()> {
        let url = self.api_url("like/removelike");
        let body = TargetRequest {
            context: self.get_context(),
            target: TargetContent {
                playlist_id: Some(playlist_id),
                video_id: None,
            },
        };

        let status = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?
            .status()
            .is_success();
        if status {
            Ok(())
        } else {
            Err(YError::BadStatus(String::from("Unsave album")))
        }
    }

    pub async fn save_to_playlist_raw(&self, song: &Song, playlist_id: &str) -> YResult<()> {
        let video_id = &song.video_id;
        let actions = vec![ActionsContent {
            action: "ACTION_ADD_VIDEO",
            added_video_id: Some(video_id),
            dedupe_option: Some("DEDUPE_OPTION_CHECK"),
            removed_video_id: None,
            set_video_id: None,
        }];

        let url = self.api_url("browse/edit_playlist");
        let body = SaveUnsaveListRequest {
            context: self.get_context(),
            actions,
            playlist_id,
        };
        let response = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            Err(YError::BadStatus(String::from("Save song to playlist")))
        } else {
            let text = response.text().await?;
            if text.contains("STATUS_SUCCEEDED") {
                Ok(())
            } else {
                Err(YError::AlreadyInPlaylist)
            }
        }
    }

    pub async fn unsave_to_playlist_raw(&self, song: &Song, playlist_id: &str) -> YResult<()> {
        let video_id = &song.video_id;
        let set_video_id = &song.set_video_id;
        let url = self.api_url("browse/edit_playlist");
        let actions = vec![ActionsContent {
            action: "ACTION_REMOVE_VIDEO",
            added_video_id: None,
            dedupe_option: None,
            removed_video_id: Some(video_id),
            set_video_id: Some(set_video_id),
        }];

        let body = SaveUnsaveListRequest {
            context: self.get_context(),
            actions,
            playlist_id,
        };
        let response = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            Err(YError::BadStatus(String::from("Unsave song to playlist")))
        } else {
            let text = response.text().await?;
            if text.contains("STATUS_SUCCEEDED") {
                Ok(())
            } else {
                Err(YError::AlreadyInPlaylist)
            }
        }
    }

    pub async fn unlike_song_raw(&self, song: &Song) -> YResult<()> {
        let video_id = &song.video_id;
        let url = self.api_url("like/removelike");
        let body = TargetRequest {
            context: self.get_context(),
            target: TargetContent {
                video_id: Some(video_id),
                playlist_id: None,
            },
        };
        let response = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(YError::BadStatus(String::from("Unlike Song")))
        }
    }

    pub async fn like_song_raw(&self, song: &Song) -> YResult<()> {
        let video_id = &song.video_id;
        let url = self.api_url("like/like");

        let body = TargetRequest {
            context: self.get_context(),
            target: TargetContent {
                video_id: Some(video_id),
                playlist_id: None,
            },
        };
        let response = self
            .http
            .post(&url)
            .headers(self.get_api_headers())
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(YError::BadStatus(String::from("Like Song")))
        }
    }
}

pub(crate) fn browser_cookie_to_set_cookie(cookie: &Cookie) -> String {
    gecko::cookie_to_set_cookie(
        &cookie.name,
        &cookie.value,
        &cookie.domain,
        &cookie.path,
        cookie.secure,
        cookie.http_only,
    )
}

pub(crate) fn filter_chromium_cookies(cookies: Vec<Cookie>, now_secs: i64) -> Vec<Cookie> {
    cookies
        .into_iter()
        .filter(|c| {
            let domain_match = cookie_domain_applies_to_host(&c.domain, YTM_HOST);
            let not_expired = match c.expires {
                None => true, // session cookie
                Some(exp) => exp > (now_secs.max(0) as u64),
            };
            domain_match && not_expired && !c.name.is_empty() && !c.value.is_empty()
        })
        .collect()
}

pub(crate) fn build_jar_from_chromium_cookies(
    cookies: &[Cookie],
) -> YResult<(Jar, Option<String>)> {
    let jar = Jar::default();
    let ytm_url = Url::parse(YTM_DOMAIN)?;

    for cookie in cookies {
        let cookie_str = browser_cookie_to_set_cookie(cookie);
        let origin_url = origin_url_for_cookie(&cookie.domain, &ytm_url);
        jar.add_cookie_str(&cookie_str, &origin_url);
    }

    let sapisid = sapisid_from_jar(&jar, &ytm_url);
    Ok((jar, sapisid))
}

/// Selects the winning browser session across browser families according to the global precedence policy:
/// 1. Gecko authenticated candidate (first deterministic match)
/// 2. Chromium authenticated candidate (first deterministic match)
/// 3. Gecko anonymous candidate (first deterministic match)
/// 4. Chromium anonymous candidate (first deterministic match)
/// 5. Empty anonymous session
///
/// Cookie sets across browser families are never merged.
pub(crate) fn select_cross_family_session(
    gecko: Option<(Jar, Option<String>)>,
    chromium: Option<(Jar, Option<String>)>,
) -> (Jar, Option<String>) {
    // 1. Gecko authenticated candidate
    if let Some((jar, Some(sapisid))) = gecko {
        return (jar, Some(sapisid));
    }
    // 2. Chromium authenticated candidate
    if let Some((jar, Some(sapisid))) = chromium {
        return (jar, Some(sapisid));
    }
    // 3. Gecko anonymous candidate
    if let Some((jar, None)) = gecko {
        return (jar, None);
    }
    // 4. Chromium anonymous candidate
    if let Some((jar, None)) = chromium {
        return (jar, None);
    }
    // 5. Empty anonymous session
    (Jar::default(), None)
}

pub fn load_cookies() -> YResult<(Jar, Option<String>)> {
    let config_dir = dirs::config_dir();
    let home_dir = dirs::home_dir();
    let gecko_candidate =
        gecko::load_gecko_cookies_with_roots(config_dir.as_deref(), home_dir.as_deref())?;
    // Fast-path: If Gecko candidate has authentication, it takes highest global precedence (#1)
    if let Some((_, Some(_))) = &gecko_candidate {
        return Ok(gecko_candidate.unwrap());
    }

    let chromium_candidate = load_chromium_candidate_with_roots(config_dir.as_deref())?;
    Ok(select_cross_family_session(
        gecko_candidate,
        chromium_candidate,
    ))
}

pub(crate) type BrowserLoader = fn(Option<Vec<String>>) -> rookie::Result<Vec<Cookie>>;

pub(crate) fn select_authenticated_candidate(
    candidate: (Jar, Option<String>),
    first_anonymous: &mut Option<(Jar, Option<String>)>,
) -> Option<(Jar, Option<String>)> {
    if candidate.1.is_some() {
        return Some(candidate);
    }

    if first_anonymous.is_none() {
        *first_anonymous = Some(candidate);
    }
    None
}

pub(crate) const DEFAULT_CHROMIUM_LOADERS: [(&str, BrowserLoader); 7] = [
    ("chrome", rookie::chrome),
    ("chromium", rookie::chromium),
    ("brave", rookie::brave),
    ("edge", rookie::edge),
    ("opera", rookie::opera),
    ("vivaldi", rookie::vivaldi),
    ("arc", rookie::arc),
];

pub(crate) fn find_brave_origin_cookie_databases_with_root(
    config_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let config = match config_dir {
        Some(c) => c,
        None => return candidates,
    };

    let base_path = config.join("BraveSoftware/Brave-Origin");
    if !base_path.exists() {
        return candidates;
    }

    if let Ok(entries) = std::fs::read_dir(&base_path) {
        let mut sub_dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        sub_dirs.sort();

        for dir in sub_dirs {
            let network_db = dir.join("Network/Cookies");
            let direct_cookies = dir.join("Cookies");
            let sqlite_db = dir.join("cookies.sqlite");

            let db = if network_db.is_file() {
                Some(network_db)
            } else if direct_cookies.is_file() {
                Some(direct_cookies)
            } else if sqlite_db.is_file() {
                Some(sqlite_db)
            } else {
                None
            };

            if let Some(db_path) = db
                && !candidates.contains(&db_path)
            {
                candidates.push(db_path);
            }
        }
    }

    for filename in &["Network/Cookies", "Cookies", "cookies.sqlite"] {
        let direct_db = base_path.join(filename);
        if direct_db.is_file() && !candidates.contains(&direct_db) {
            candidates.push(direct_db);
        }
    }

    candidates
}

pub(crate) fn load_brave_origin_cookies_from_paths<I>(
    paths: I,
) -> YResult<Option<(Jar, Option<String>)>>
where
    I: IntoIterator<Item = PathBuf>,
{
    let domains = vec!["youtube.com".to_string(), "music.youtube.com".to_string()];
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut first_anonymous = None;

    for db_path in paths {
        let raw = match rookie::any_browser(&db_path.to_string_lossy(), Some(domains.clone()), None)
        {
            Ok(c) => c,
            Err(_) => continue,
        };

        let filtered = filter_chromium_cookies(raw, now_secs);
        if filtered.is_empty() {
            continue;
        }

        let candidate = build_jar_from_chromium_cookies(&filtered)?;
        if let Some(candidate) = select_authenticated_candidate(candidate, &mut first_anonymous) {
            return Ok(Some(candidate));
        }
    }

    Ok(first_anonymous)
}

pub(crate) fn load_brave_origin_candidate_with_root(
    config_dir: Option<&Path>,
) -> YResult<Option<(Jar, Option<String>)>> {
    load_brave_origin_cookies_from_paths(find_brave_origin_cookie_databases_with_root(config_dir))
}

pub(crate) fn load_chromium_candidate_with_roots(
    config_dir: Option<&Path>,
) -> YResult<Option<(Jar, Option<String>)>> {
    load_chromium_candidate_with_loaders_and_root(&DEFAULT_CHROMIUM_LOADERS, config_dir)
}

pub(crate) fn load_chromium_candidate_with_loaders_and_root(
    loaders: &[(&str, BrowserLoader)],
    config_dir: Option<&Path>,
) -> YResult<Option<(Jar, Option<String>)>> {
    let domains = vec!["youtube.com".to_string(), "music.youtube.com".to_string()];
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut first_anonymous_candidate = None;

    for (_name, loader) in loaders {
        let raw = match loader(Some(domains.clone())) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let filtered = filter_chromium_cookies(raw, now_secs);
        if filtered.is_empty() {
            continue;
        }

        let candidate = build_jar_from_chromium_cookies(&filtered)?;
        if let Some(candidate) =
            select_authenticated_candidate(candidate, &mut first_anonymous_candidate)
        {
            return Ok(Some(candidate));
        }
    }

    // Brave Origin fallback: rookie::brave() does not discover Brave Origin.
    // Use explicit database-path discovery and pass each discovered database to rookie::any_browser.
    if let Some(candidate) = load_brave_origin_candidate_with_root(config_dir)?
        && let Some(candidate) =
            select_authenticated_candidate(candidate, &mut first_anonymous_candidate)
    {
        return Ok(Some(candidate));
    }

    Ok(first_anonymous_candidate)
}

fn extract_between(source: &str, start: &str, end: &str) -> Option<String> {
    source.find(start).and_then(|start_idx| {
        let start_pos = start_idx + start.len();
        source[start_pos..]
            .find(end)
            .map(|end_idx| source[start_pos..start_pos + end_idx].to_string())
    })
}

#[cfg(test)]
mod tests;
