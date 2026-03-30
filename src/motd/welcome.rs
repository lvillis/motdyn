use reqx::blocking::Client;
use reqx::prelude::RedirectPolicy;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::{Position, Url};

use crate::config::{expand_tilde, MotdConfig};

use super::types::{
    RemoteWelcomeSettings, WelcomeCacheEntry, WelcomeResolution, WelcomeSource, DEFAULT_WELCOME,
    DEFAULT_WELCOME_CACHE_PATH, DEFAULT_WELCOME_CACHE_TTL_SECS, DEFAULT_WELCOME_TIMEOUT_MS,
    MAX_WELCOME_BODY_BYTES,
};

pub(super) fn resolve_welcome_text(cfg: &MotdConfig) -> WelcomeResolution {
    let settings = resolve_remote_welcome_settings(cfg);
    let Some(raw_value) = cfg.welcome.as_deref() else {
        return default_welcome(settings);
    };

    if raw_value.trim().is_empty() {
        return default_welcome(settings);
    }

    let parsed_url = match Url::parse(raw_value) {
        Ok(url) => url,
        Err(_) => {
            return WelcomeResolution {
                text: raw_value.to_string(),
                source: WelcomeSource::Literal,
                source_detail: "literal config value".to_string(),
                url: None,
                settings,
                warnings: Vec::new(),
            };
        }
    };

    resolve_remote_welcome(normalize_remote_welcome_url(parsed_url), settings)
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

fn resolve_remote_welcome(parsed_url: Url, settings: RemoteWelcomeSettings) -> WelcomeResolution {
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
            return WelcomeResolution {
                text: entry.body.clone(),
                source: WelcomeSource::CacheFresh,
                source_detail: format!("cache hit ({})", settings.cache_path.display()),
                url: Some(normalized_url),
                settings,
                warnings,
            };
        }
    }

    if !matches!(parsed_url.scheme(), "http" | "https") {
        return cache_or_default(
            settings,
            normalized_url,
            cached_entry,
            format!(
                "unsupported URL scheme '{}'; using cached or default welcome",
                parsed_url.scheme()
            ),
            warnings,
        );
    }

    if parsed_url.scheme() == "http" && !settings.allow_http {
        return cache_or_default(
            settings,
            normalized_url,
            cached_entry,
            "HTTP welcome URLs are disabled; using cached or default welcome".to_string(),
            warnings,
        );
    }

    if !parsed_url.username().is_empty() || parsed_url.password().is_some() {
        return cache_or_default(
            settings,
            normalized_url,
            cached_entry,
            "URLs with embedded credentials are not supported; using cached or default welcome"
                .to_string(),
            warnings,
        );
    }

    if !settings.enabled {
        return cache_or_default(
            settings,
            normalized_url,
            cached_entry,
            "remote welcome fetch disabled; using cached or default welcome".to_string(),
            warnings,
        );
    }

    match fetch_remote_welcome_text(&parsed_url, &settings) {
        Ok(body) => {
            if let Err(err) = write_welcome_cache(
                &settings.cache_path,
                &WelcomeCacheEntry {
                    url: normalized_url.clone(),
                    fetched_at_secs: current_unix_secs(),
                    body: body.clone(),
                },
            ) {
                warnings.push(format!("failed to update welcome cache: {}", err));
            }

            WelcomeResolution {
                text: body,
                source: WelcomeSource::RemoteFresh,
                source_detail: format!("fetched from {}", parsed_url),
                url: Some(parsed_url.to_string()),
                settings,
                warnings,
            }
        }
        Err(err) => cache_or_default(
            settings,
            parsed_url.to_string(),
            cached_entry,
            err,
            warnings,
        ),
    }
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

fn cache_or_default(
    settings: RemoteWelcomeSettings,
    url: String,
    cached_entry: Option<WelcomeCacheEntry>,
    reason: String,
    mut warnings: Vec<String>,
) -> WelcomeResolution {
    warnings.push(reason);

    if let Some(entry) = cached_entry {
        return WelcomeResolution {
            text: entry.body,
            source: WelcomeSource::CacheStale,
            source_detail: format!("stale cache fallback ({})", settings.cache_path.display()),
            url: Some(url),
            settings,
            warnings,
        };
    }

    WelcomeResolution {
        text: DEFAULT_WELCOME.to_string(),
        source: WelcomeSource::Default,
        source_detail: "default welcome".to_string(),
        url: Some(url),
        settings,
        warnings,
    }
}

fn fetch_remote_welcome_text(
    parsed_url: &Url,
    settings: &RemoteWelcomeSettings,
) -> Result<String, String> {
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
        .map_err(|err| format!("failed to build reqx client: {}", err))?;

    let response = client
        .get(path_and_query)
        .send_response()
        .map_err(|err| format!("failed to fetch remote welcome: {}", err))?;

    if !response.status().is_success() {
        return Err(format!(
            "remote welcome returned HTTP {}; using cached or default welcome",
            response.status()
        ));
    }

    let body = response
        .text()
        .map_err(|err| format!("failed to decode remote welcome: {}", err))?;

    if body.trim().is_empty() {
        Err("remote welcome response was empty".to_string())
    } else {
        Ok(body.to_string())
    }
}

pub(super) fn read_welcome_cache(path: &Path) -> Result<Option<WelcomeCacheEntry>, String> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)
        .map_err(|err| format!("failed to read cache '{}': {}", path.display(), err))?;
    let (header, body) = content
        .split_once("\n\n")
        .ok_or_else(|| format!("cache '{}' is malformed", path.display()))?;

    let mut url = None;
    let mut fetched_at_secs = None;
    for line in header.lines() {
        if let Some(value) = line.strip_prefix("url=") {
            url = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("fetched_at=") {
            fetched_at_secs = value.parse::<u64>().ok();
        }
    }

    let url = url.ok_or_else(|| format!("cache '{}' is missing url", path.display()))?;
    let fetched_at_secs = fetched_at_secs
        .ok_or_else(|| format!("cache '{}' is missing fetched_at", path.display()))?;
    let body = body.to_string();

    if body.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(WelcomeCacheEntry {
        url,
        fetched_at_secs,
        body,
    }))
}

pub(super) fn write_welcome_cache(path: &Path, entry: &WelcomeCacheEntry) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create cache directory '{}': {}",
                parent.display(),
                err
            )
        })?;
    }

    let content = format!(
        "url={}\nfetched_at={}\n\n{}",
        entry.url, entry.fetched_at_secs, entry.body
    );
    fs::write(path, content)
        .map_err(|err| format!("failed to write cache '{}': {}", path.display(), err))
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
