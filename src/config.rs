use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RawConfig {
    pub welcome: Option<String>,
    pub farewell: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct MotdConfig {
    pub welcome: Option<String>,
    pub farewell: Option<String>,
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
    })
}

pub fn merge_config(sys_cfg: Option<MotdConfig>, usr_cfg: Option<MotdConfig>) -> MotdConfig {
    let mut final_cfg = sys_cfg.unwrap_or_default();
    if let Some(u) = usr_cfg {
        if let Some(art) = u.welcome {
            final_cfg.welcome = Some(art);
        }
        if let Some(fw) = u.farewell {
            final_cfg.farewell = Some(fw);
        }
    }
    final_cfg
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
        fs::write(&config_path, "welcome = \"hi\"\nfarewell = \"bye\"\n").unwrap();

        let cfg = load_config(&config_path).expect("config should parse");
        assert_eq!(cfg.welcome.as_deref(), Some("hi"));
        assert_eq!(cfg.farewell.as_deref(), Some("bye"));
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
        };
        let usr = MotdConfig {
            welcome: Some("user".into()),
            farewell: None,
        };

        let merged = merge_config(Some(sys), Some(usr));
        assert_eq!(merged.welcome.as_deref(), Some("user"));
        assert_eq!(merged.farewell.as_deref(), Some("sys bye"));
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
