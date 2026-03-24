use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RawConfig {
    pub welcome: Option<String>,
    pub farewell: Option<String>,
    pub modules: Option<Vec<String>>,
    pub remote_welcome: Option<RemoteWelcomeConfig>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct RemoteWelcomeConfig {
    pub enabled: Option<bool>,
    pub timeout_ms: Option<u64>,
    pub cache_ttl_secs: Option<u64>,
    pub cache_path: Option<String>,
    pub follow_redirects: Option<bool>,
    pub allow_http: Option<bool>,
}

#[derive(Debug, Default, Clone)]
pub struct MotdConfig {
    pub welcome: Option<String>,
    pub farewell: Option<String>,
    pub modules: Option<Vec<String>>,
    pub remote_welcome: RemoteWelcomeConfig,
}

pub fn load_config(path: &Path) -> Option<MotdConfig> {
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    let raw: RawConfig = toml::from_str(&content).ok()?;

    Some(MotdConfig {
        welcome: raw.welcome,
        farewell: raw.farewell,
        modules: raw.modules,
        remote_welcome: raw.remote_welcome.unwrap_or_default(),
    })
}

pub fn merge_config(sys_cfg: Option<MotdConfig>, usr_cfg: Option<MotdConfig>) -> MotdConfig {
    let mut final_cfg = sys_cfg.unwrap_or_default();
    if let Some(u) = usr_cfg {
        if let Some(welcome) = u.welcome {
            final_cfg.welcome = Some(welcome);
        }
        if let Some(farewell) = u.farewell {
            final_cfg.farewell = Some(farewell);
        }
        if let Some(modules) = u.modules {
            final_cfg.modules = Some(modules);
        }
        merge_remote_welcome(&mut final_cfg.remote_welcome, u.remote_welcome);
    }
    final_cfg
}

fn merge_remote_welcome(target: &mut RemoteWelcomeConfig, source: RemoteWelcomeConfig) {
    if let Some(enabled) = source.enabled {
        target.enabled = Some(enabled);
    }
    if let Some(timeout_ms) = source.timeout_ms {
        target.timeout_ms = Some(timeout_ms);
    }
    if let Some(cache_ttl_secs) = source.cache_ttl_secs {
        target.cache_ttl_secs = Some(cache_ttl_secs);
    }
    if let Some(cache_path) = source.cache_path {
        target.cache_path = Some(cache_path);
    }
    if let Some(follow_redirects) = source.follow_redirects {
        target.follow_redirects = Some(follow_redirects);
    }
    if let Some(allow_http) = source.allow_http {
        target.allow_http = Some(allow_http);
    }
}

pub fn expand_tilde(path_str: &str) -> PathBuf {
    if !path_str.starts_with('~') {
        return PathBuf::from(path_str);
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(path_str.replacen('~', &home.to_string_lossy(), 1));
    }
    PathBuf::from(path_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    #[test]
    fn load_config_reads_expected_fields() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(
            &config_path,
            "welcome = \"hi\"\nfarewell = \"bye\"\nmodules = [\"host\", \"time\"]\n[remote_welcome]\ntimeout_ms = 250\n",
        )
        .unwrap();

        let cfg = load_config(&config_path).expect("config should parse");
        assert_eq!(cfg.welcome.as_deref(), Some("hi"));
        assert_eq!(cfg.farewell.as_deref(), Some("bye"));
        assert_eq!(
            cfg.modules.as_deref(),
            Some(&["host".to_string(), "time".to_string()][..])
        );
        assert_eq!(cfg.remote_welcome.timeout_ms, Some(250));
    }

    #[test]
    fn load_config_returns_none_for_missing_file() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("absent.toml");
        assert!(load_config(&missing).is_none());
    }

    #[test]
    fn merge_config_prefers_user_values() {
        let sys = MotdConfig {
            welcome: Some("system".into()),
            farewell: Some("sys bye".into()),
            modules: Some(vec!["host".into(), "memory".into()]),
            remote_welcome: RemoteWelcomeConfig {
                timeout_ms: Some(500),
                allow_http: Some(false),
                ..RemoteWelcomeConfig::default()
            },
        };
        let usr = MotdConfig {
            welcome: Some("user".into()),
            farewell: None,
            modules: Some(vec!["time".into(), "disk".into()]),
            remote_welcome: RemoteWelcomeConfig {
                cache_ttl_secs: Some(60),
                allow_http: Some(true),
                ..RemoteWelcomeConfig::default()
            },
        };

        let merged = merge_config(Some(sys), Some(usr));
        assert_eq!(merged.welcome.as_deref(), Some("user"));
        assert_eq!(merged.farewell.as_deref(), Some("sys bye"));
        assert_eq!(
            merged.modules.as_deref(),
            Some(&["time".to_string(), "disk".to_string()][..])
        );
        assert_eq!(merged.remote_welcome.timeout_ms, Some(500));
        assert_eq!(merged.remote_welcome.cache_ttl_secs, Some(60));
        assert_eq!(merged.remote_welcome.allow_http, Some(true));
    }

    #[test]
    fn expand_tilde_uses_home_env() {
        let temp_home = tempdir().unwrap();
        let original = env::var_os("HOME");
        env::set_var("HOME", temp_home.path());

        let expanded = expand_tilde("~/motdyn/config.toml");
        assert!(expanded.starts_with(temp_home.path()));
        assert!(expanded.ends_with(Path::new("motdyn").join("config.toml")));

        match original {
            Some(val) => env::set_var("HOME", val),
            None => env::remove_var("HOME"),
        }
    }
}
