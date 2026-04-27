use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::{self, Deserializer};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    welcome: Option<String>,
    #[serde(default)]
    welcome_sources: Option<Vec<String>>,
    farewell: Option<String>,
    #[serde(default, deserialize_with = "deserialize_module_list")]
    modules: Option<Vec<String>>,
    remote_welcome: Option<RemoteWelcomeConfig>,
    service_status: Option<ServiceStatusConfig>,
    output: Option<OutputConfig>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteWelcomeConfig {
    pub enabled: Option<bool>,
    pub timeout_ms: Option<u64>,
    pub cache_ttl_secs: Option<u64>,
    pub cache_path: Option<String>,
    pub follow_redirects: Option<bool>,
    pub allow_http: Option<bool>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceStatusConfig {
    pub services: Option<Vec<String>>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    pub compact: Option<bool>,
    pub plain: Option<bool>,
    pub section_headers: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_hidden_field_list")]
    pub hidden_fields: Option<Vec<String>>,
}

#[derive(Debug, Default, Clone)]
pub struct MotdConfig {
    pub welcome: Option<String>,
    pub welcome_sources: Option<Vec<String>>,
    pub farewell: Option<String>,
    pub modules: Option<Vec<String>>,
    pub remote_welcome: RemoteWelcomeConfig,
    pub service_status: ServiceStatusConfig,
    pub output: OutputConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigModuleName {
    Host,
    Network,
    User,
    Time,
    Uptime,
    Load,
    Os,
    Kernel,
    Virtualization,
    Cpu,
    Memory,
    Swap,
    Disk,
    LastLogin,
    FailedLogin,
    Services,
    Updates,
}

impl ConfigModuleName {
    fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "host" | "hostname" => Some(Self::Host),
            "network" | "net" | "nic" => Some(Self::Network),
            "user" | "users" | "login" => Some(Self::User),
            "time" | "clock" | "datetime" => Some(Self::Time),
            "uptime" => Some(Self::Uptime),
            "load" | "loadavg" | "load_average" => Some(Self::Load),
            "os" | "system" => Some(Self::Os),
            "kernel" => Some(Self::Kernel),
            "virtualization" | "virt" | "container" => Some(Self::Virtualization),
            "cpu" => Some(Self::Cpu),
            "memory" | "mem" => Some(Self::Memory),
            "swap" => Some(Self::Swap),
            "disk" | "disks" | "filesystem" | "fs" => Some(Self::Disk),
            "last_login" | "lastlogin" | "last" => Some(Self::LastLogin),
            "failed_login" | "failedlogin" | "failed" | "security" => Some(Self::FailedLogin),
            "services" | "service" | "systemd" => Some(Self::Services),
            "updates" | "update" | "packages" | "package_updates" => Some(Self::Updates),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Network => "network",
            Self::User => "user",
            Self::Time => "time",
            Self::Uptime => "uptime",
            Self::Load => "load",
            Self::Os => "os",
            Self::Kernel => "kernel",
            Self::Virtualization => "virtualization",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Swap => "swap",
            Self::Disk => "disk",
            Self::LastLogin => "last_login",
            Self::FailedLogin => "failed_login",
            Self::Services => "services",
            Self::Updates => "updates",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigHiddenFieldName {
    MainInterface,
    MainIpv4,
    SourceIp,
    LoginUserCount,
    Timezone,
    KernelVersion,
    Virtualization,
    Swap,
    NfsDisks,
}

impl ConfigHiddenFieldName {
    fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "main_interface" | "interface" | "iface" => Some(Self::MainInterface),
            "main_ipv4" | "main_ip" | "ipv4" | "ip" => Some(Self::MainIpv4),
            "source_ip" | "from_ip" => Some(Self::SourceIp),
            "login_user_count" | "logged_in_users" | "user_count" => Some(Self::LoginUserCount),
            "timezone" | "tz" => Some(Self::Timezone),
            "kernel_version" | "kernel" => Some(Self::KernelVersion),
            "virtualization" | "virt" => Some(Self::Virtualization),
            "swap" => Some(Self::Swap),
            "nfs_disks" | "nfs" => Some(Self::NfsDisks),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::MainInterface => "main_interface",
            Self::MainIpv4 => "main_ipv4",
            Self::SourceIp => "source_ip",
            Self::LoginUserCount => "login_user_count",
            Self::Timezone => "timezone",
            Self::KernelVersion => "kernel_version",
            Self::Virtualization => "virtualization",
            Self::Swap => "swap",
            Self::NfsDisks => "nfs_disks",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValidationError {
    RemoteWelcomeTimeoutZero,
    RemoteWelcomeCachePathEmpty,
}

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RemoteWelcomeTimeoutZero => {
                write!(f, "`remote_welcome.timeout_ms` must be greater than 0")
            }
            Self::RemoteWelcomeCachePathEmpty => {
                write!(f, "`remote_welcome.cache_path` must not be empty")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigLoadError {
    Read {
        path: PathBuf,
        message: String,
    },
    Parse {
        path: PathBuf,
        message: String,
    },
    Validation {
        path: PathBuf,
        issues: Vec<ConfigValidationError>,
    },
}

impl fmt::Display for ConfigLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, message } => {
                write!(f, "failed to read config '{}': {}", path.display(), message)
            }
            Self::Parse { path, message } => {
                write!(
                    f,
                    "failed to parse config '{}': {}",
                    path.display(),
                    message
                )
            }
            Self::Validation { path, issues } => {
                let joined = issues
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "invalid config '{}': {}", path.display(), joined)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigLoadStatus {
    Missing,
    Loaded,
    Invalid(ConfigLoadError),
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: Option<MotdConfig>,
    pub status: ConfigLoadStatus,
}

impl LoadedConfig {
    fn missing() -> Self {
        Self {
            config: None,
            status: ConfigLoadStatus::Missing,
        }
    }

    fn loaded(config: MotdConfig) -> Self {
        Self {
            config: Some(config),
            status: ConfigLoadStatus::Loaded,
        }
    }

    fn invalid(error: ConfigLoadError) -> Self {
        Self {
            config: None,
            status: ConfigLoadStatus::Invalid(error),
        }
    }

    pub fn status_label(&self) -> &'static str {
        match self.status {
            ConfigLoadStatus::Missing => "missing",
            ConfigLoadStatus::Loaded => "loaded",
            ConfigLoadStatus::Invalid(_) => "invalid",
        }
    }

    pub fn note(&self) -> Option<String> {
        match &self.status {
            ConfigLoadStatus::Invalid(error) => Some(error.to_string()),
            _ => None,
        }
    }
}

pub fn load_config(path: &Path) -> LoadedConfig {
    if !path.exists() {
        return LoadedConfig::missing();
    }

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) => {
            return LoadedConfig::invalid(ConfigLoadError::Read {
                path: path.to_path_buf(),
                message: err.to_string(),
            });
        }
    };

    let raw: RawConfig = match toml::from_str(&content) {
        Ok(raw) => raw,
        Err(err) => {
            return LoadedConfig::invalid(ConfigLoadError::Parse {
                path: path.to_path_buf(),
                message: err.to_string(),
            });
        }
    };

    match validate_and_normalize(raw, path) {
        Ok(config) => LoadedConfig::loaded(config),
        Err(err) => LoadedConfig::invalid(err),
    }
}

pub fn merge_config(sys_cfg: Option<MotdConfig>, usr_cfg: Option<MotdConfig>) -> MotdConfig {
    let mut final_cfg = sys_cfg.unwrap_or_default();
    if let Some(user_cfg) = usr_cfg {
        if let Some(welcome) = user_cfg.welcome {
            final_cfg.welcome = Some(welcome);
        }
        if let Some(welcome_sources) = user_cfg.welcome_sources {
            final_cfg.welcome_sources = Some(welcome_sources);
        }
        if let Some(farewell) = user_cfg.farewell {
            final_cfg.farewell = Some(farewell);
        }
        if let Some(modules) = user_cfg.modules {
            final_cfg.modules = Some(modules);
        }
        merge_remote_welcome(&mut final_cfg.remote_welcome, user_cfg.remote_welcome);
        merge_service_status(&mut final_cfg.service_status, user_cfg.service_status);
        merge_output(&mut final_cfg.output, user_cfg.output);
    }
    final_cfg
}

fn validate_and_normalize(raw: RawConfig, path: &Path) -> Result<MotdConfig, ConfigLoadError> {
    let mut issues = Vec::new();
    let remote_welcome =
        normalize_remote_welcome(raw.remote_welcome.unwrap_or_default(), &mut issues);
    let service_status = normalize_service_status(raw.service_status.unwrap_or_default());
    let output = normalize_output(raw.output.unwrap_or_default());
    let config = MotdConfig {
        welcome: normalize_optional_text(raw.welcome),
        welcome_sources: normalize_ordered_string_list(raw.welcome_sources),
        farewell: normalize_optional_text(raw.farewell),
        modules: raw.modules,
        remote_welcome,
        service_status,
        output,
    };

    if issues.is_empty() {
        Ok(config)
    } else {
        Err(ConfigLoadError::Validation {
            path: path.to_path_buf(),
            issues,
        })
    }
}

fn deserialize_module_list<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_canonical_list(deserializer, "module", |raw| {
        ConfigModuleName::parse(raw).map(ConfigModuleName::key)
    })
}

fn deserialize_hidden_field_list<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_canonical_list(deserializer, "hidden field", |raw| {
        ConfigHiddenFieldName::parse(raw).map(ConfigHiddenFieldName::key)
    })
}

fn deserialize_canonical_list<'de, D, F>(
    deserializer: D,
    kind: &'static str,
    normalize: F,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
    F: Fn(&str) -> Option<&'static str>,
{
    let raw_values = Option::<Vec<String>>::deserialize(deserializer)?;
    let Some(raw_values) = raw_values else {
        return Ok(None);
    };

    let mut normalized = Vec::new();
    for value in raw_values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let canonical = normalize(trimmed)
            .ok_or_else(|| de::Error::custom(format!("unsupported {} '{}'", kind, trimmed)))?;
        if !normalized.iter().any(|entry: &String| entry == canonical) {
            normalized.push(canonical.to_string());
        }
    }

    Ok(Some(normalized))
}

fn normalize_remote_welcome(
    mut config: RemoteWelcomeConfig,
    issues: &mut Vec<ConfigValidationError>,
) -> RemoteWelcomeConfig {
    if matches!(config.timeout_ms, Some(0)) {
        issues.push(ConfigValidationError::RemoteWelcomeTimeoutZero);
    }

    config.cache_path = match config.cache_path {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                issues.push(ConfigValidationError::RemoteWelcomeCachePathEmpty);
                Some(value)
            } else {
                Some(trimmed.to_string())
            }
        }
        None => None,
    };

    config
}

fn normalize_output(config: OutputConfig) -> OutputConfig {
    config
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

fn normalize_service_status(mut config: ServiceStatusConfig) -> ServiceStatusConfig {
    config.services = normalize_ordered_string_list(config.services);
    config
}

fn merge_service_status(target: &mut ServiceStatusConfig, source: ServiceStatusConfig) {
    if let Some(services) = source.services {
        target.services = Some(services);
    }
}

fn merge_output(target: &mut OutputConfig, source: OutputConfig) {
    if let Some(compact) = source.compact {
        target.compact = Some(compact);
    }
    if let Some(plain) = source.plain {
        target.plain = Some(plain);
    }
    if let Some(section_headers) = source.section_headers {
        target.section_headers = Some(section_headers);
    }
    if let Some(hidden_fields) = source.hidden_fields {
        target.hidden_fields = Some(hidden_fields);
    }
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_ordered_string_list(values: Option<Vec<String>>) -> Option<Vec<String>> {
    values.map(|values| {
        let mut normalized = Vec::new();
        for value in values {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !normalized.iter().any(|entry: &String| entry == trimmed) {
                normalized.push(trimmed.to_string());
            }
        }
        normalized
    })
}

pub fn expand_tilde(path_str: &str) -> PathBuf {
    expand_tilde_with_home(path_str, env::var_os("HOME").as_deref())
}

fn expand_tilde_with_home(path_str: &str, home: Option<&OsStr>) -> PathBuf {
    if !path_str.starts_with('~') {
        return PathBuf::from(path_str);
    }
    if let Some(home) = home {
        return PathBuf::from(path_str.replacen('~', &home.to_string_lossy(), 1));
    }
    PathBuf::from(path_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use tempfile::tempdir;

    #[test]
    fn load_config_reads_expected_fields() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(
            &config_path,
            "welcome = \"hi\"\nfarewell = \"bye\"\nmodules = [\"host\", \"time\"]\n[remote_welcome]\ntimeout_ms = 250\n[service_status]\nservices = [\"sshd\", \"chronyd\"]\n[output]\ncompact = true\nhidden_fields = [\"source_ip\"]\n",
        )
        .unwrap();

        let loaded = load_config(&config_path);
        assert_eq!(loaded.status, ConfigLoadStatus::Loaded);
        let cfg = loaded.config.expect("config should parse");
        assert_eq!(cfg.welcome.as_deref(), Some("hi"));
        assert_eq!(cfg.farewell.as_deref(), Some("bye"));
        assert_eq!(
            cfg.modules.as_deref(),
            Some(&["host".to_string(), "time".to_string()][..])
        );
        assert_eq!(cfg.remote_welcome.timeout_ms, Some(250));
        assert_eq!(
            cfg.service_status.services.as_deref(),
            Some(&["sshd".to_string(), "chronyd".to_string()][..])
        );
        assert_eq!(cfg.output.compact, Some(true));
        assert_eq!(
            cfg.output.hidden_fields.as_deref(),
            Some(&["source_ip".to_string()][..])
        );
    }

    #[test]
    fn load_config_returns_missing_for_absent_file() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("absent.toml");
        let loaded = load_config(&missing);

        assert_eq!(loaded.status, ConfigLoadStatus::Missing);
        assert!(loaded.config.is_none());
    }

    #[test]
    fn load_config_reports_validation_errors() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(
            &config_path,
            "[remote_welcome]\ntimeout_ms = 0\ncache_path = \"   \"\n",
        )
        .unwrap();

        let loaded = load_config(&config_path);
        match loaded.status {
            ConfigLoadStatus::Invalid(ConfigLoadError::Validation { issues, .. }) => {
                assert_eq!(
                    issues,
                    vec![
                        ConfigValidationError::RemoteWelcomeTimeoutZero,
                        ConfigValidationError::RemoteWelcomeCachePathEmpty,
                    ]
                );
            }
            other => panic!("unexpected status: {other:?}"),
        }
        assert!(loaded.config.is_none());
    }

    #[test]
    fn load_config_normalizes_text_and_lists() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(
            &config_path,
            "welcome = \"  hi  \"\nwelcome_sources = [\" ./motd.txt \", \"\", \"https://example.com/motd.txt\", \"./motd.txt\"]\nfarewell = \"  \"\nmodules = [\" host \", \"hostname\", \"\", \"time\"]\n[service_status]\nservices = [\" sshd \", \"\", \"chronyd\", \"sshd\"]\n[output]\nhidden_fields = [\" source_ip \", \"from_ip\", \"  \", \"nfs\"]\n",
        )
        .unwrap();

        let loaded = load_config(&config_path);
        let cfg = loaded.config.expect("config should load");
        assert_eq!(cfg.welcome.as_deref(), Some("hi"));
        assert_eq!(
            cfg.welcome_sources.as_deref(),
            Some(
                &[
                    "./motd.txt".to_string(),
                    "https://example.com/motd.txt".to_string()
                ][..]
            )
        );
        assert_eq!(cfg.farewell, None);
        assert_eq!(
            cfg.modules.as_deref(),
            Some(&["host".to_string(), "time".to_string()][..])
        );
        assert_eq!(
            cfg.service_status.services.as_deref(),
            Some(&["sshd".to_string(), "chronyd".to_string()][..])
        );
        assert_eq!(
            cfg.output.hidden_fields.as_deref(),
            Some(&["source_ip".to_string(), "nfs_disks".to_string()][..])
        );
    }

    #[test]
    fn load_config_rejects_unknown_top_level_fields() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(&config_path, "unknown = true\n").unwrap();

        let loaded = load_config(&config_path);
        match loaded.status {
            ConfigLoadStatus::Invalid(ConfigLoadError::Parse { message, .. }) => {
                assert!(message.contains("unknown field"));
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn load_config_rejects_invalid_module_names() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(&config_path, "modules = [\"bogus\"]\n").unwrap();

        let loaded = load_config(&config_path);
        match loaded.status {
            ConfigLoadStatus::Invalid(ConfigLoadError::Parse { message, .. }) => {
                assert!(message.contains("unsupported module 'bogus'"));
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn load_config_rejects_invalid_hidden_fields() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(&config_path, "[output]\nhidden_fields = [\"bogus\"]\n").unwrap();

        let loaded = load_config(&config_path);
        match loaded.status {
            ConfigLoadStatus::Invalid(ConfigLoadError::Parse { message, .. }) => {
                assert!(message.contains("unsupported hidden field 'bogus'"));
            }
            other => panic!("unexpected status: {other:?}"),
        }
    }

    #[test]
    fn merge_config_prefers_user_values() {
        let sys = MotdConfig {
            welcome: Some("system".into()),
            welcome_sources: Some(vec![
                "https://system.example/motd.txt".into(),
                "./system-banner.txt".into(),
            ]),
            farewell: Some("sys bye".into()),
            modules: Some(vec!["host".into(), "memory".into()]),
            remote_welcome: RemoteWelcomeConfig {
                timeout_ms: Some(500),
                allow_http: Some(false),
                ..RemoteWelcomeConfig::default()
            },
            service_status: ServiceStatusConfig {
                services: Some(vec!["sshd".into(), "chronyd".into()]),
            },
            output: OutputConfig::default(),
        };
        let usr = MotdConfig {
            welcome: Some("user".into()),
            welcome_sources: Some(vec![
                "./user-banner.txt".into(),
                "https://user.example/motd.txt".into(),
            ]),
            farewell: None,
            modules: Some(vec!["time".into(), "disk".into()]),
            remote_welcome: RemoteWelcomeConfig {
                cache_ttl_secs: Some(60),
                allow_http: Some(true),
                ..RemoteWelcomeConfig::default()
            },
            service_status: ServiceStatusConfig {
                services: Some(vec!["docker".into()]),
            },
            output: OutputConfig {
                compact: Some(true),
                ..OutputConfig::default()
            },
        };

        let merged = merge_config(Some(sys), Some(usr));
        assert_eq!(merged.welcome.as_deref(), Some("user"));
        assert_eq!(
            merged.welcome_sources.as_deref(),
            Some(
                &[
                    "./user-banner.txt".to_string(),
                    "https://user.example/motd.txt".to_string()
                ][..]
            )
        );
        assert_eq!(merged.farewell.as_deref(), Some("sys bye"));
        assert_eq!(
            merged.modules.as_deref(),
            Some(&["time".to_string(), "disk".to_string()][..])
        );
        assert_eq!(merged.remote_welcome.timeout_ms, Some(500));
        assert_eq!(merged.remote_welcome.cache_ttl_secs, Some(60));
        assert_eq!(merged.remote_welcome.allow_http, Some(true));
        assert_eq!(
            merged.service_status.services.as_deref(),
            Some(&["docker".to_string()][..])
        );
        assert_eq!(merged.output.compact, Some(true));
    }

    #[test]
    fn expand_tilde_uses_home_env() {
        let temp_home = tempdir().unwrap();

        let expanded =
            expand_tilde_with_home("~/motdyn/config.toml", Some(temp_home.path().as_os_str()));
        assert!(expanded.starts_with(temp_home.path()));
        assert!(expanded.ends_with(Path::new("motdyn").join("config.toml")));
    }
}
