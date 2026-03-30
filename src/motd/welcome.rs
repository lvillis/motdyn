use reqx::blocking::Client;
use reqx::prelude::RedirectPolicy;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::{Position, Url};

use crate::config::{expand_tilde, MotdConfig};

use super::types::{
    RemoteWelcomeSettings, WelcomeCacheEntry, WelcomeIssue, WelcomeResolution, WelcomeSource,
    DEFAULT_WELCOME, DEFAULT_WELCOME_CACHE_PATH, DEFAULT_WELCOME_CACHE_TTL_SECS,
    DEFAULT_WELCOME_TIMEOUT_MS, MAX_WELCOME_BODY_BYTES,
};

enum WelcomeAttempt {
    Resolved(WelcomeResolution),
    Unusable(Vec<WelcomeIssue>),
}

struct RemoteFetchResult {
    body: String,
    etag: Option<String>,
    last_modified: Option<String>,
    not_modified: bool,
}

pub(super) fn resolve_welcome_text(cfg: &MotdConfig) -> WelcomeResolution {
    let settings = resolve_remote_welcome_settings(cfg);
    let sources = configured_welcome_sources(cfg);
    if sources.is_empty() {
        return default_welcome(settings);
    }

    let mut accumulated_warnings = Vec::new();
    for source in sources {
        match resolve_welcome_source(&source, &settings) {
            WelcomeAttempt::Resolved(mut resolution) => {
                if !accumulated_warnings.is_empty() {
                    let mut warnings = accumulated_warnings;
                    warnings.extend(resolution.warnings);
                    resolution.warnings = warnings;
                }
                return resolution;
            }
            WelcomeAttempt::Unusable(warnings) => accumulated_warnings.extend(warnings),
        }
    }

    let mut resolution = default_welcome(settings);
    resolution.warnings = accumulated_warnings;
    resolution
}

pub(super) fn resolve_remote_welcome_settings(cfg: &MotdConfig) -> RemoteWelcomeSettings {
    let cache_path = cfg
        .remote_welcome
        .cache_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(expand_tilde)
        .unwrap_or_else(|| expand_tilde(DEFAULT_WELCOME_CACHE_PATH));

    RemoteWelcomeSettings {
        enabled: cfg.remote_welcome.enabled.unwrap_or(true),
        timeout_ms: cfg
            .remote_welcome
            .timeout_ms
            .unwrap_or(DEFAULT_WELCOME_TIMEOUT_MS)
            .max(1),
        cache_ttl_secs: cfg
            .remote_welcome
            .cache_ttl_secs
            .unwrap_or(DEFAULT_WELCOME_CACHE_TTL_SECS),
        cache_path,
        follow_redirects: cfg.remote_welcome.follow_redirects.unwrap_or(true),
        allow_http: cfg.remote_welcome.allow_http.unwrap_or(false),
    }
}

fn configured_welcome_sources(cfg: &MotdConfig) -> Vec<String> {
    if let Some(sources) = cfg.welcome_sources.as_ref() {
        return sources.clone();
    }

    cfg.welcome
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| vec![value.trim().to_string()])
        .unwrap_or_default()
}

fn default_welcome(settings: RemoteWelcomeSettings) -> WelcomeResolution {
    WelcomeResolution {
        text: DEFAULT_WELCOME.to_string(),
        source: WelcomeSource::Default,
        source_detail: "default welcome".to_string(),
        url: None,
        settings,
        warnings: Vec::new(),
    }
}

fn resolve_welcome_source(raw_source: &str, settings: &RemoteWelcomeSettings) -> WelcomeAttempt {
    if let Ok(url) = Url::parse(raw_source) {
        return match url.scheme() {
            "file" => resolve_file_url_source(&url, settings),
            "http" | "https" => {
                resolve_remote_welcome_source(normalize_remote_welcome_url(url), settings)
            }
            other => WelcomeAttempt::Unusable(vec![WelcomeIssue::UnsupportedUrlScheme(
                other.to_string(),
            )]),
        };
    }

    if looks_like_local_path(raw_source) {
        return resolve_local_file_source(
            expand_tilde(raw_source),
            raw_source.to_string(),
            settings,
        );
    }

    WelcomeAttempt::Resolved(WelcomeResolution {
        text: raw_source.to_string(),
        source: WelcomeSource::Literal,
        source_detail: "literal config value".to_string(),
        url: None,
        settings: settings.clone(),
        warnings: Vec::new(),
    })
}

fn resolve_file_url_source(url: &Url, settings: &RemoteWelcomeSettings) -> WelcomeAttempt {
    if let Some(host) = url.host_str() {
        if !host.is_empty() && host != "localhost" {
            return WelcomeAttempt::Unusable(vec![WelcomeIssue::FileUrlUnsupportedHost(
                host.to_string(),
            )]);
        }
    }

    let path = match url.to_file_path() {
        Ok(path) => path,
        Err(_) => {
            return WelcomeAttempt::Unusable(vec![WelcomeIssue::SourceFailed(url.to_string())]);
        }
    };

    resolve_local_file_source(path, url.to_string(), settings)
}

fn resolve_local_file_source(
    path: PathBuf,
    source_label: String,
    settings: &RemoteWelcomeSettings,
) -> WelcomeAttempt {
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            return WelcomeAttempt::Unusable(vec![WelcomeIssue::LocalFileRead {
                path,
                message: err.to_string(),
            }]);
        }
    };

    if content.trim().is_empty() {
        return WelcomeAttempt::Unusable(vec![WelcomeIssue::LocalFileEmpty { path }]);
    }

    WelcomeAttempt::Resolved(WelcomeResolution {
        text: content,
        source: WelcomeSource::LocalFile,
        source_detail: format!("read from {}", source_label),
        url: Some(source_label),
        settings: settings.clone(),
        warnings: Vec::new(),
    })
}

fn resolve_remote_welcome_source(
    parsed_url: Url,
    settings: &RemoteWelcomeSettings,
) -> WelcomeAttempt {
    let normalized_url = parsed_url.to_string();
    let mut warnings = Vec::new();
    let cached_entry = match read_welcome_cache(&settings.cache_path) {
        Ok(entry) => entry.filter(|entry| entry.url == normalized_url),
        Err(err) => {
            warnings.push(err);
            None
        }
    };

    if let Some(entry) = cached_entry.as_ref() {
        if cache_entry_is_fresh(entry, settings.cache_ttl_secs) {
            return WelcomeAttempt::Resolved(WelcomeResolution {
                text: entry.body.clone(),
                source: WelcomeSource::CacheFresh,
                source_detail: format!("cache hit ({})", settings.cache_path.display()),
                url: Some(normalized_url),
                settings: settings.clone(),
                warnings,
            });
        }
    }

    if parsed_url.scheme() == "http" && !settings.allow_http {
        return cache_or_unusable(
            settings,
            normalized_url,
            cached_entry,
            WelcomeIssue::HttpDisabled,
            warnings,
        );
    }

    if !parsed_url.username().is_empty() || parsed_url.password().is_some() {
        return cache_or_unusable(
            settings,
            normalized_url,
            cached_entry,
            WelcomeIssue::EmbeddedCredentials,
            warnings,
        );
    }

    if !settings.enabled {
        return cache_or_unusable(
            settings,
            normalized_url,
            cached_entry,
            WelcomeIssue::RemoteDisabled,
            warnings,
        );
    }

    match fetch_remote_welcome_text(&parsed_url, settings, cached_entry.as_ref()) {
        Ok(fetch) if fetch.not_modified => {
            let Some(entry) = cached_entry else {
                return WelcomeAttempt::Unusable(vec![WelcomeIssue::SourceFailed(
                    parsed_url.to_string(),
                )]);
            };
            let refreshed_entry = WelcomeCacheEntry {
                url: normalized_url.clone(),
                fetched_at_secs: current_unix_secs(),
                etag: fetch.etag.or(entry.etag),
                last_modified: fetch.last_modified.or(entry.last_modified),
                body: entry.body.clone(),
            };
            if let Err(err) = write_welcome_cache(&settings.cache_path, &refreshed_entry) {
                warnings.push(err);
            }

            WelcomeAttempt::Resolved(WelcomeResolution {
                text: refreshed_entry.body,
                source: WelcomeSource::CacheRevalidated,
                source_detail: format!(
                    "cache revalidated via HTTP 304 ({})",
                    settings.cache_path.display()
                ),
                url: Some(parsed_url.to_string()),
                settings: settings.clone(),
                warnings,
            })
        }
        Ok(fetch) => {
            let cache_entry = WelcomeCacheEntry {
                url: normalized_url.clone(),
                fetched_at_secs: current_unix_secs(),
                etag: fetch.etag,
                last_modified: fetch.last_modified,
                body: fetch.body.clone(),
            };
            if let Err(err) = write_welcome_cache(&settings.cache_path, &cache_entry) {
                warnings.push(err);
            }

            WelcomeAttempt::Resolved(WelcomeResolution {
                text: fetch.body,
                source: WelcomeSource::RemoteFresh,
                source_detail: format!("fetched from {}", parsed_url),
                url: Some(parsed_url.to_string()),
                settings: settings.clone(),
                warnings,
            })
        }
        Err(err) => cache_or_unusable(
            settings,
            parsed_url.to_string(),
            cached_entry,
            err,
            warnings,
        ),
    }
}

fn looks_like_local_path(raw: &str) -> bool {
    raw.starts_with("~/") || raw.starts_with('/') || raw.starts_with("./") || raw.starts_with("../")
}

fn normalize_remote_welcome_url(mut url: Url) -> Url {
    url.set_fragment(None);

    let default_port = match url.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    if url.port().is_some() && url.port() == default_port {
        let _ = url.set_port(None);
    }

    url
}

fn cache_or_unusable(
    settings: &RemoteWelcomeSettings,
    url: String,
    cached_entry: Option<WelcomeCacheEntry>,
    reason: WelcomeIssue,
    mut warnings: Vec<WelcomeIssue>,
) -> WelcomeAttempt {
    warnings.push(reason);

    if let Some(entry) = cached_entry {
        return WelcomeAttempt::Resolved(WelcomeResolution {
            text: entry.body,
            source: WelcomeSource::CacheStale,
            source_detail: format!("stale cache fallback ({})", settings.cache_path.display()),
            url: Some(url),
            settings: settings.clone(),
            warnings,
        });
    }

    WelcomeAttempt::Unusable(warnings)
}

fn fetch_remote_welcome_text(
    parsed_url: &Url,
    settings: &RemoteWelcomeSettings,
    cached_entry: Option<&WelcomeCacheEntry>,
) -> Result<RemoteFetchResult, WelcomeIssue> {
    let timeout = Duration::from_millis(settings.timeout_ms);
    let base_url = parsed_url.origin().ascii_serialization();
    let path_and_query = parsed_url[Position::BeforePath..Position::AfterQuery].to_string();
    let redirect_policy = if settings.follow_redirects {
        RedirectPolicy::follow()
    } else {
        RedirectPolicy::none()
    };

    let client = Client::builder(base_url)
        .request_timeout(timeout)
        .total_timeout(timeout)
        .max_response_body_bytes(MAX_WELCOME_BODY_BYTES)
        .redirect_policy(redirect_policy)
        .build()
        .map_err(|err| WelcomeIssue::BuildClient(err.to_string()))?;

    let mut request = client.get(path_and_query);
    if let Some(entry) = cached_entry {
        if let Some(etag) = entry.etag.as_deref() {
            request = request
                .try_header("if-none-match", etag)
                .map_err(|err| WelcomeIssue::BuildClient(err.to_string()))?;
        }
        if let Some(last_modified) = entry.last_modified.as_deref() {
            request = request
                .try_header("if-modified-since", last_modified)
                .map_err(|err| WelcomeIssue::BuildClient(err.to_string()))?;
        }
    }

    let response = request
        .send_response()
        .map_err(|err| WelcomeIssue::Fetch(err.to_string()))?;

    let etag = response
        .headers()
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let last_modified = response
        .headers()
        .get("last-modified")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    if response.status().as_u16() == 304 {
        return Ok(RemoteFetchResult {
            body: String::new(),
            etag,
            last_modified,
            not_modified: true,
        });
    }

    if !response.status().is_success() {
        return Err(WelcomeIssue::HttpStatus(response.status().to_string()));
    }

    let body = response
        .text()
        .map_err(|err| WelcomeIssue::Decode(err.to_string()))?;

    if body.trim().is_empty() {
        Err(WelcomeIssue::EmptyResponse)
    } else {
        Ok(RemoteFetchResult {
            body: body.to_string(),
            etag,
            last_modified,
            not_modified: false,
        })
    }
}

pub(super) fn read_welcome_cache(path: &Path) -> Result<Option<WelcomeCacheEntry>, WelcomeIssue> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path).map_err(|err| WelcomeIssue::CacheRead {
        path: path.to_path_buf(),
        message: err.to_string(),
    })?;
    let (header, body) =
        content
            .split_once("\n\n")
            .ok_or_else(|| WelcomeIssue::CacheMalformed {
                path: path.to_path_buf(),
                reason: "malformed",
            })?;

    let mut url = None;
    let mut fetched_at_secs = None;
    let mut etag = None;
    let mut last_modified = None;
    for line in header.lines() {
        if let Some(value) = line.strip_prefix("url=") {
            url = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("fetched_at=") {
            fetched_at_secs = value.parse::<u64>().ok();
        } else if let Some(value) = line.strip_prefix("etag=") {
            if !value.is_empty() {
                etag = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix("last_modified=") {
            if !value.is_empty() {
                last_modified = Some(value.to_string());
            }
        }
    }

    let url = url.ok_or_else(|| WelcomeIssue::CacheMalformed {
        path: path.to_path_buf(),
        reason: "missing url",
    })?;
    let fetched_at_secs = fetched_at_secs.ok_or_else(|| WelcomeIssue::CacheMalformed {
        path: path.to_path_buf(),
        reason: "missing fetched_at",
    })?;
    let body = body.to_string();

    if body.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(WelcomeCacheEntry {
        url,
        fetched_at_secs,
        etag,
        last_modified,
        body,
    }))
}

pub(super) fn write_welcome_cache(
    path: &Path,
    entry: &WelcomeCacheEntry,
) -> Result<(), WelcomeIssue> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| WelcomeIssue::CacheWrite {
            path: parent.to_path_buf(),
            message: err.to_string(),
        })?;
    }

    let mut header = format!("url={}\nfetched_at={}\n", entry.url, entry.fetched_at_secs);
    if let Some(etag) = &entry.etag {
        header.push_str(&format!("etag={}\n", etag));
    }
    if let Some(last_modified) = &entry.last_modified {
        header.push_str(&format!("last_modified={}\n", last_modified));
    }
    header.push('\n');
    header.push_str(&entry.body);

    fs::write(path, header).map_err(|err| WelcomeIssue::CacheWrite {
        path: path.to_path_buf(),
        message: err.to_string(),
    })
}

fn cache_entry_is_fresh(entry: &WelcomeCacheEntry, ttl_secs: u64) -> bool {
    current_unix_secs().saturating_sub(entry.fetched_at_secs) <= ttl_secs
}

pub(super) fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
