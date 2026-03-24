use chrono::Local;
use colored::Colorize;
use reqx::blocking::Client;
use reqx::prelude::RedirectPolicy;
use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::{Position, Url};

use crate::config::{expand_tilde, MotdConfig};

#[cfg(unix)]
use libc::{endutxent, getutxent, setutxent, statvfs, USER_PROCESS};
#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::mem::MaybeUninit;

const DEFAULT_WELCOME: &str = "Welcome!";
const DEFAULT_FAREWELL: &str = "Have a nice day!";
const DEFAULT_WELCOME_TIMEOUT_MS: u64 = 1_000;
const DEFAULT_WELCOME_CACHE_TTL_SECS: u64 = 300;
const DEFAULT_WELCOME_CACHE_PATH: &str = "~/.cache/motdyn/welcome.txt";
const MAX_WELCOME_BODY_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct RenderContext {
    pub system_config_path: String,
    pub system_config_loaded: bool,
    pub user_config_path: String,
    pub user_config_loaded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ModuleKind {
    Host,
    Network,
    User,
    Time,
    Uptime,
    Os,
    Kernel,
    Virtualization,
    Cpu,
    Memory,
    Swap,
    Disk,
}

impl ModuleKind {
    fn key(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Network => "network",
            Self::User => "user",
            Self::Time => "time",
            Self::Uptime => "uptime",
            Self::Os => "os",
            Self::Kernel => "kernel",
            Self::Virtualization => "virtualization",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Swap => "swap",
            Self::Disk => "disk",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleSource {
    Default,
    Configured,
    FallbackDefault,
}

#[derive(Debug, Clone)]
struct ModuleSelection {
    modules: Vec<ModuleKind>,
    ignored: Vec<String>,
    source: ModuleSource,
}

#[derive(Debug, Clone)]
struct RenderedItem {
    label: String,
    value: String,
}

#[derive(Debug, Clone)]
struct SystemSnapshot {
    host_name: String,
    main_iface: String,
    main_ip: String,
    current_user: String,
    from_ip: String,
    login_user_count: usize,
    now_str_with_tz: String,
    uptime_str: String,
    os_name: String,
    os_version: String,
    kernel_version: String,
    virt_info: Option<String>,
    cpu_brand: String,
    cpu_count: usize,
    memory: UsageSummary,
    swap: UsageSummary,
    disk_items: Vec<RenderedItem>,
}

#[derive(Debug, Clone, Copy)]
struct UsageSummary {
    used_gb: f64,
    total_gb: f64,
    ratio: f64,
}

#[derive(Debug, Clone)]
struct RemoteWelcomeSettings {
    enabled: bool,
    timeout_ms: u64,
    cache_ttl_secs: u64,
    cache_path: PathBuf,
    follow_redirects: bool,
    allow_http: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WelcomeSource {
    Default,
    Literal,
    RemoteFresh,
    CacheFresh,
    CacheStale,
}

#[derive(Debug, Clone)]
struct WelcomeResolution {
    text: String,
    source: WelcomeSource,
    source_detail: String,
    url: Option<String>,
    settings: RemoteWelcomeSettings,
    warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct WelcomeCacheEntry {
    url: String,
    fetched_at_secs: u64,
    body: String,
}

pub fn render(verbose: bool, cfg: &MotdConfig, ctx: &RenderContext) {
    for line in build_output(verbose, cfg, ctx) {
        println!("{}", line);
    }
}

fn build_output(verbose: bool, cfg: &MotdConfig, ctx: &RenderContext) -> Vec<String> {
    let welcome = resolve_welcome_text(cfg);
    let selection = resolve_modules(cfg);
    let snapshot = collect_snapshot();
    let mut lines = Vec::new();

    lines.push(String::new());
    lines.push(welcome.text.clone());
    lines.push(String::new());
    lines.extend(format_aligned_items(&render_module_items(
        &selection.modules,
        &snapshot,
    )));

    if verbose {
        lines.push(String::new());
        lines.push("Verbose details:".bold().cyan().to_string());
        lines.extend(format_aligned_items(&build_verbose_items(
            cfg, ctx, &selection, &welcome,
        )));
    }

    lines.push(String::new());
    lines.push(resolve_farewell_text(cfg).bold().cyan().to_string());
    lines
}

fn resolve_farewell_text(cfg: &MotdConfig) -> String {
    match cfg.farewell.as_deref() {
        Some(text) if !text.trim().is_empty() => text.to_string(),
        _ => DEFAULT_FAREWELL.to_string(),
    }
}

fn collect_snapshot() -> SystemSnapshot {
    let (os_name, os_version) = get_os_info();
    let now_str_with_tz = Local::now().format("%Y-%m-%d %H:%M:%S %:z").to_string();
    let uptime_str = parse_uptime().unwrap_or_else(|| "unknown".to_string());
    let kernel_version = read_first_line("/proc/sys/kernel/osrelease")
        .unwrap_or_else(|| "Unknown kernel".to_string());
    let host_name =
        read_first_line("/proc/sys/kernel/hostname").unwrap_or_else(|| "Unknown host".to_string());
    let (cpu_brand, cpu_count) = parse_cpuinfo();
    let (mem_total, mem_free, swap_total, swap_free) = parse_meminfo();
    let (current_user, from_ip) = get_current_user_and_ip();
    let login_user_count = get_logged_in_user_count();
    let virt_info = detect_virtualization();
    let main_iface = get_default_interface().unwrap_or_else(|| "unknown".to_string());
    let main_ip = if main_iface == "unknown" {
        "unknown".to_string()
    } else {
        get_interface_ipv4(&main_iface).unwrap_or_else(|| "unknown".to_string())
    };

    SystemSnapshot {
        host_name,
        main_iface,
        main_ip,
        current_user,
        from_ip,
        login_user_count,
        now_str_with_tz,
        uptime_str,
        os_name,
        os_version,
        kernel_version,
        virt_info,
        cpu_brand,
        cpu_count,
        memory: usage_summary(mem_total, mem_free),
        swap: usage_summary(swap_total, swap_free),
        disk_items: collect_disk_usage_items(),
    }
}

fn render_module_items(modules: &[ModuleKind], snapshot: &SystemSnapshot) -> Vec<RenderedItem> {
    let mut items = Vec::new();

    for module in modules {
        match module {
            ModuleKind::Host => items.push(RenderedItem {
                label: "Host name:".to_string(),
                value: snapshot.host_name.bright_yellow().to_string(),
            }),
            ModuleKind::Network => items.push(RenderedItem {
                label: "Main NIC:".to_string(),
                value: format!(
                    "{} ({})",
                    snapshot.main_iface.bright_cyan(),
                    snapshot.main_ip.bright_cyan()
                ),
            }),
            ModuleKind::User => items.push(RenderedItem {
                label: "User info:".to_string(),
                value: format!(
                    "{} (from {}), {} user(s) logged in",
                    snapshot.current_user.bright_cyan(),
                    snapshot.from_ip.bright_cyan(),
                    snapshot.login_user_count.to_string().bright_cyan()
                ),
            }),
            ModuleKind::Time => items.push(RenderedItem {
                label: "Current time (TZ):".to_string(),
                value: snapshot.now_str_with_tz.bright_yellow().to_string(),
            }),
            ModuleKind::Uptime => items.push(RenderedItem {
                label: "System uptime:".to_string(),
                value: snapshot.uptime_str.bright_yellow().to_string(),
            }),
            ModuleKind::Os => items.push(RenderedItem {
                label: "Operating system:".to_string(),
                value: format!("{} {}", snapshot.os_name, snapshot.os_version)
                    .bright_yellow()
                    .to_string(),
            }),
            ModuleKind::Kernel => items.push(RenderedItem {
                label: "Kernel version:".to_string(),
                value: snapshot.kernel_version.bright_green().to_string(),
            }),
            ModuleKind::Virtualization => {
                if let Some(virt) = &snapshot.virt_info {
                    items.push(RenderedItem {
                        label: "Virtualization:".to_string(),
                        value: virt.bright_yellow().to_string(),
                    });
                }
            }
            ModuleKind::Cpu => items.push(RenderedItem {
                label: "CPU:".to_string(),
                value: format!(
                    "{} ({} cores)",
                    snapshot.cpu_brand.bright_magenta(),
                    snapshot.cpu_count.to_string().bright_magenta()
                ),
            }),
            ModuleKind::Memory => items.push(RenderedItem {
                label: "Memory used/total:".to_string(),
                value: format_usage(snapshot.memory),
            }),
            ModuleKind::Swap => items.push(RenderedItem {
                label: "Swap used/total:".to_string(),
                value: format_usage(snapshot.swap),
            }),
            ModuleKind::Disk => items.extend(snapshot.disk_items.clone()),
        }
    }

    items
}

fn build_verbose_items(
    cfg: &MotdConfig,
    ctx: &RenderContext,
    selection: &ModuleSelection,
    welcome: &WelcomeResolution,
) -> Vec<RenderedItem> {
    let mut items = vec![
        RenderedItem {
            label: "System config:".to_string(),
            value: format!(
                "{} ({})",
                ctx.system_config_path,
                if ctx.system_config_loaded {
                    "loaded"
                } else {
                    "missing"
                }
            ),
        },
        RenderedItem {
            label: "User config:".to_string(),
            value: format!(
                "{} ({})",
                ctx.user_config_path,
                if ctx.user_config_loaded {
                    "loaded"
                } else {
                    "missing"
                }
            ),
        },
        RenderedItem {
            label: "Module source:".to_string(),
            value: match selection.source {
                ModuleSource::Default => "default order".to_string(),
                ModuleSource::Configured => "configured order".to_string(),
                ModuleSource::FallbackDefault => {
                    "configured list had no valid modules; using defaults".to_string()
                }
            },
        },
        RenderedItem {
            label: "Active modules:".to_string(),
            value: selection
                .modules
                .iter()
                .map(|module| module.key())
                .collect::<Vec<_>>()
                .join(", "),
        },
        RenderedItem {
            label: "Welcome source:".to_string(),
            value: welcome.source_detail.clone(),
        },
        RenderedItem {
            label: "Welcome mode:".to_string(),
            value: match welcome.source {
                WelcomeSource::Default => "default".to_string(),
                WelcomeSource::Literal => "literal".to_string(),
                WelcomeSource::RemoteFresh => "remote fetch".to_string(),
                WelcomeSource::CacheFresh => "fresh cache".to_string(),
                WelcomeSource::CacheStale => "stale cache fallback".to_string(),
            },
        },
        RenderedItem {
            label: "Remote welcome:".to_string(),
            value: format!(
                "enabled={}, timeout={}ms, ttl={}s",
                welcome.settings.enabled,
                welcome.settings.timeout_ms,
                welcome.settings.cache_ttl_secs
            ),
        },
        RenderedItem {
            label: "Remote policy:".to_string(),
            value: format!(
                "follow_redirects={}, allow_http={}",
                welcome.settings.follow_redirects, welcome.settings.allow_http
            ),
        },
        RenderedItem {
            label: "Remote cache:".to_string(),
            value: welcome.settings.cache_path.display().to_string(),
        },
        RenderedItem {
            label: "External tools:".to_string(),
            value: format!(
                "ip={}, systemd-detect-virt={}",
                yes_no(command_exists("ip")),
                yes_no(command_exists("systemd-detect-virt"))
            ),
        },
    ];

    if let Some(url) = &welcome.url {
        items.push(RenderedItem {
            label: "Welcome URL:".to_string(),
            value: url.clone(),
        });
    }

    if let Some(modules) = &cfg.modules {
        items.push(RenderedItem {
            label: "Configured modules:".to_string(),
            value: if modules.is_empty() {
                "(empty list)".to_string()
            } else {
                modules.join(", ")
            },
        });
    }

    if !selection.ignored.is_empty() {
        items.push(RenderedItem {
            label: "Ignored modules:".to_string(),
            value: selection.ignored.join(", "),
        });
    }

    if !welcome.warnings.is_empty() {
        items.push(RenderedItem {
            label: "Welcome notes:".to_string(),
            value: welcome.warnings.join(" | "),
        });
    }

    items
}

fn format_aligned_items(items: &[RenderedItem]) -> Vec<String> {
    let width = items.iter().map(|item| item.label.len()).max().unwrap_or(0);
    items
        .iter()
        .map(|item| {
            format!(
                "{:width$} {}",
                item.label.bright_white(),
                item.value,
                width = width
            )
        })
        .collect()
}

fn format_usage(summary: UsageSummary) -> String {
    format!(
        "{:.2}/{:.2} GB ({:.2}%)",
        summary.used_gb, summary.total_gb, summary.ratio
    )
}

fn usage_summary(total_kb: u64, free_kb: u64) -> UsageSummary {
    let (used_gb, total_gb, ratio) = to_gb_and_ratio(total_kb, free_kb);
    UsageSummary {
        used_gb,
        total_gb,
        ratio,
    }
}

fn resolve_modules(cfg: &MotdConfig) -> ModuleSelection {
    let Some(configured) = cfg.modules.as_ref() else {
        return ModuleSelection {
            modules: default_modules(),
            ignored: Vec::new(),
            source: ModuleSource::Default,
        };
    };

    if configured.is_empty() {
        return ModuleSelection {
            modules: Vec::new(),
            ignored: Vec::new(),
            source: ModuleSource::Configured,
        };
    }

    let mut modules = Vec::new();
    let mut ignored = Vec::new();
    let mut seen = HashSet::new();

    for raw in configured {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        match normalize_module_name(trimmed) {
            Some(module) if seen.insert(module) => modules.push(module),
            Some(_) => {}
            None => ignored.push(trimmed.to_string()),
        }
    }

    if modules.is_empty() {
        ModuleSelection {
            modules: default_modules(),
            ignored,
            source: ModuleSource::FallbackDefault,
        }
    } else {
        ModuleSelection {
            modules,
            ignored,
            source: ModuleSource::Configured,
        }
    }
}

fn default_modules() -> Vec<ModuleKind> {
    vec![
        ModuleKind::Host,
        ModuleKind::Network,
        ModuleKind::User,
        ModuleKind::Time,
        ModuleKind::Uptime,
        ModuleKind::Os,
        ModuleKind::Kernel,
        ModuleKind::Virtualization,
        ModuleKind::Cpu,
        ModuleKind::Memory,
        ModuleKind::Swap,
        ModuleKind::Disk,
    ]
}

fn normalize_module_name(name: &str) -> Option<ModuleKind> {
    match name.to_ascii_lowercase().as_str() {
        "host" | "hostname" => Some(ModuleKind::Host),
        "network" | "net" | "nic" => Some(ModuleKind::Network),
        "user" | "users" | "login" => Some(ModuleKind::User),
        "time" | "clock" | "datetime" => Some(ModuleKind::Time),
        "uptime" => Some(ModuleKind::Uptime),
        "os" | "system" => Some(ModuleKind::Os),
        "kernel" => Some(ModuleKind::Kernel),
        "virtualization" | "virt" | "container" => Some(ModuleKind::Virtualization),
        "cpu" => Some(ModuleKind::Cpu),
        "memory" | "mem" => Some(ModuleKind::Memory),
        "swap" => Some(ModuleKind::Swap),
        "disk" | "disks" | "filesystem" | "fs" => Some(ModuleKind::Disk),
        _ => None,
    }
}

fn resolve_welcome_text(cfg: &MotdConfig) -> WelcomeResolution {
    let settings = resolve_remote_welcome_settings(cfg);
    let Some(raw_value) = cfg.welcome.as_deref() else {
        return WelcomeResolution {
            text: DEFAULT_WELCOME.to_string(),
            source: WelcomeSource::Default,
            source_detail: "default welcome".to_string(),
            url: None,
            settings,
            warnings: Vec::new(),
        };
    };

    if raw_value.trim().is_empty() {
        return WelcomeResolution {
            text: DEFAULT_WELCOME.to_string(),
            source: WelcomeSource::Default,
            source_detail: "default welcome".to_string(),
            url: None,
            settings,
            warnings: Vec::new(),
        };
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

fn resolve_remote_welcome_settings(cfg: &MotdConfig) -> RemoteWelcomeSettings {
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

fn read_welcome_cache(path: &Path) -> Result<Option<WelcomeCacheEntry>, String> {
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

fn write_welcome_cache(path: &Path, entry: &WelcomeCacheEntry) -> Result<(), String> {
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

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn command_exists(command: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|dir| dir.join(command).is_file())
}

fn detect_virtualization() -> Option<String> {
    if Path::new("/.dockerenv").exists() {
        return Some("Docker".to_string());
    }

    if let Ok(content) = fs::read_to_string("/proc/1/cgroup") {
        if content.contains("docker") {
            return Some("Docker".to_string());
        }
        if content.contains("lxc") {
            return Some("LXC".to_string());
        }
    }

    if let Ok(output) = Command::new("systemd-detect-virt").output() {
        if output.status.success() {
            let virt_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if virt_str != "none" && !virt_str.is_empty() {
                return Some(virt_str);
            }
        }
    }
    None
}

fn parse_uptime() -> Option<String> {
    let line = std::fs::read_to_string("/proc/uptime").ok()?;
    let parts: Vec<_> = line.split_whitespace().collect();
    let total_seconds = parts.first()?.parse::<f64>().ok()? as u64;

    Some(format_uptime(total_seconds))
}

fn format_uptime(mut secs: u64) -> String {
    let days = secs / 86400;
    secs %= 86400;
    let hours = secs / 3600;
    secs %= 3600;
    let minutes = secs / 60;
    secs %= 60;

    if days > 0 {
        format!("{} days, {:02}:{:02}:{:02}", days, hours, minutes, secs)
    } else {
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    }
}

fn get_os_info() -> (String, String) {
    if let Some(r) = parse_redhat_release() {
        return r;
    }
    if let Some(r) = parse_os_release() {
        return r;
    }
    let fallback_os = read_first_line("/proc/sys/kernel/ostype").unwrap_or("Linux".to_string());
    ("Linux".to_string(), fallback_os)
}

fn parse_redhat_release() -> Option<(String, String)> {
    let content = fs::read_to_string("/etc/redhat-release").ok()?;
    let line = content.trim();
    let needle = " release ";
    let pos = line.find(needle)?;
    let os_name = &line[..pos];
    let ver_str = &line[pos + needle.len()..];
    Some((os_name.to_string(), ver_str.to_string()))
}

fn parse_os_release() -> Option<(String, String)> {
    let content = fs::read_to_string("/etc/os-release").ok()?;
    let mut os_name: Option<String> = None;
    let mut os_version: Option<String> = None;

    for line in content.lines() {
        if let Some(stripped) = line.strip_prefix("NAME=") {
            os_name = Some(stripped.trim().trim_matches('"').to_string());
        } else if let Some(stripped) = line.strip_prefix("VERSION_ID=") {
            os_version = Some(stripped.trim().trim_matches('"').to_string());
        }
    }
    match (os_name, os_version) {
        (Some(n), Some(v)) => Some((n, v)),
        _ => None,
    }
}

fn parse_meminfo() -> (u64, u64, u64, u64) {
    let file = match File::open("/proc/meminfo") {
        Ok(f) => f,
        Err(_) => return (0, 0, 0, 0),
    };
    let reader = BufReader::new(file);

    let mut mem_total = 0;
    let mut mem_free = 0;
    let mut swap_total = 0;
    let mut swap_free = 0;

    for line in reader.lines().map_while(Result::ok) {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        match parts[0] {
            "MemTotal:" => mem_total = parts[1].parse().unwrap_or(0),
            "MemAvailable:" => mem_free = parts[1].parse().unwrap_or(0),
            "SwapTotal:" => swap_total = parts[1].parse().unwrap_or(0),
            "SwapFree:" => swap_free = parts[1].parse().unwrap_or(0),
            _ => {}
        }
    }

    if mem_free == 0 {
        mem_free = fallback_mem_free().unwrap_or(0);
    }
    (mem_total, mem_free, swap_total, swap_free)
}

fn fallback_mem_free() -> Option<u64> {
    let file = File::open("/proc/meminfo").ok()?;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Some(stripped) = line.strip_prefix("MemFree:") {
            let val = stripped.split_whitespace().next()?;
            return val.parse::<u64>().ok();
        }
    }
    None
}

fn parse_cpuinfo() -> (String, usize) {
    let file = match File::open("/proc/cpuinfo") {
        Ok(f) => f,
        Err(_) => return ("Unknown CPU".to_string(), 0),
    };
    let reader = BufReader::new(file);

    let mut brand = "Unknown CPU".to_string();
    let mut core_count = 0;

    let mut cpu_implementer = String::new();
    let mut cpu_part = String::new();

    for line in reader.lines().map_while(Result::ok) {
        if line.starts_with("processor") {
            core_count += 1;
        } else if let Some(model_str) = line.strip_prefix("model name") {
            let parts: Vec<_> = model_str.split(':').collect();
            if parts.len() > 1 && brand == "Unknown CPU" {
                brand = parts[1].trim().to_string();
            }
        } else if let Some(imp_str) = line.strip_prefix("CPU implementer") {
            if let Some(val) = imp_str.split(':').nth(1) {
                cpu_implementer = val.trim().to_lowercase();
            }
        } else if let Some(part_str) = line.strip_prefix("CPU part") {
            if let Some(val) = part_str.split(':').nth(1) {
                cpu_part = val.trim().to_lowercase();
            }
        }
    }

    if brand == "Unknown CPU" {
        if cpu_implementer == "0x41" {
            match cpu_part.as_str() {
                "0xd03" => brand = "ARM Cortex-A53".to_string(),
                "0xd07" => brand = "ARM Cortex-A57".to_string(),
                "0xd08" => brand = "ARM Cortex-A72".to_string(),
                _ => brand = format!("ARM CPU (part={}, implementer=0x41)", cpu_part),
            }
        } else if !cpu_implementer.is_empty() {
            brand = format!(
                "ARM CPU (part={}, implementer={})",
                cpu_part, cpu_implementer
            );
        }
    }

    (brand, core_count)
}

fn read_first_line(path: &str) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut buf = String::new();
    if reader.read_line(&mut buf).ok()? > 0 {
        Some(buf.trim().to_string())
    } else {
        None
    }
}

fn get_current_user_and_ip() -> (String, String) {
    let user = env::var("USER")
        .or_else(|_| env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    let ssh_connection = env::var("SSH_CONNECTION").unwrap_or_default();
    let from_ip = ssh_connection
        .split_whitespace()
        .next()
        .unwrap_or("unknown")
        .to_string();
    (user, from_ip)
}

#[cfg(unix)]
fn get_logged_in_user_count() -> usize {
    let mut count = 0;

    unsafe {
        setutxent();
        loop {
            let entry = getutxent();
            if entry.is_null() {
                break;
            }

            let record = &*entry;
            if record.ut_type == USER_PROCESS && record.ut_user[0] != 0 {
                count += 1;
            }
        }
        endutxent();
    }

    count
}

#[cfg(not(unix))]
fn get_logged_in_user_count() -> usize {
    0
}

fn to_gb_and_ratio(total_kb: u64, free_kb: u64) -> (f64, f64, f64) {
    let used_kb = total_kb.saturating_sub(free_kb);
    let total_gb = kb_to_gb(total_kb);
    let used_gb = kb_to_gb(used_kb);
    let ratio = if total_gb > 0.0 {
        (used_gb / total_gb) * 100.0
    } else {
        0.0
    };
    (used_gb, total_gb, ratio)
}

fn kb_to_gb(kb: u64) -> f64 {
    kb as f64 / 1024.0 / 1024.0
}

#[cfg(unix)]
fn collect_disk_usage_items() -> Vec<RenderedItem> {
    let file = match File::open("/proc/mounts") {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut items = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        let mount_path = fields[1];
        let fstype = fields[2];

        if mount_path == "/" {
            if let Some(item) = disk_usage_item(mount_path, "Disk usage (root):") {
                items.push(item);
            }
        } else if matches!(fstype, "nfs" | "nfs4") {
            if let Some(item) = disk_usage_item(mount_path, "Disk usage (nfs):") {
                items.push(item);
            }
        }
    }
    items
}

#[cfg(not(unix))]
fn collect_disk_usage_items() -> Vec<RenderedItem> {
    Vec::new()
}

#[cfg(unix)]
fn disk_usage_item(mount_path: &str, label: &str) -> Option<RenderedItem> {
    let (total_bytes, used_bytes) = get_mount_usage(mount_path)?;
    let (used_str, total_str, ratio) = human_readable_usage(used_bytes, total_bytes);
    Some(RenderedItem {
        label: label.to_string(),
        value: format!(
            "{} => {}/{} ({:.2}%)",
            mount_path.bright_yellow(),
            used_str,
            total_str,
            ratio
        ),
    })
}

#[cfg(unix)]
fn get_mount_usage(mountpoint: &str) -> Option<(u64, u64)> {
    let c_path = CString::new(mountpoint).ok()?;
    let mut stat = MaybeUninit::<statvfs>::uninit();
    let ret = unsafe { statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
    if ret != 0 {
        return None;
    }
    let s = unsafe { stat.assume_init() };

    let block_size = s.f_frsize;
    let blocks_used = s.f_blocks.saturating_sub(s.f_bfree);
    let total_bytes = block_size.saturating_mul(s.f_blocks);
    let used_bytes = block_size.saturating_mul(blocks_used);
    Some((total_bytes, used_bytes))
}

#[cfg(unix)]
fn human_readable_usage(used: u64, total: u64) -> (String, String, f64) {
    let bigger = std::cmp::max(used, total);
    let (scale, suffix) = best_unit_scale(bigger as f64);

    let used_f = used as f64 / scale;
    let total_f = total as f64 / scale;
    let ratio = if total_f > 0.0 {
        used_f / total_f * 100.0
    } else {
        0.0
    };

    let used_str = format!("{:.2} {}", used_f, suffix);
    let total_str = format!("{:.2} {}", total_f, suffix);
    (used_str, total_str, ratio)
}

#[cfg(unix)]
fn best_unit_scale(bytes: f64) -> (f64, &'static str) {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const TIB: f64 = 1024.0 * 1024.0 * 1024.0 * 1024.0;
    const PIB: f64 = 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0;

    if bytes >= PIB {
        (PIB, "PB")
    } else if bytes >= TIB {
        (TIB, "TB")
    } else if bytes >= GIB {
        (GIB, "GB")
    } else if bytes >= MIB {
        (MIB, "MB")
    } else if bytes >= KIB {
        (KIB, "KB")
    } else {
        (1.0, "B")
    }
}

fn get_default_interface() -> Option<String> {
    let output = Command::new("ip")
        .arg("route")
        .arg("show")
        .arg("default")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() > 4 && parts[0] == "default" && parts[1] == "via" && parts[3] == "dev" {
            return Some(parts[4].to_string());
        }
    }
    None
}

fn get_interface_ipv4(iface: &str) -> Option<String> {
    let output = Command::new("ip")
        .args(["-o", "-4", "addr", "show", "dev", iface])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() > 3 && parts[2] == "inet" {
            let ip_slash = parts[3];
            let ip_only = ip_slash.split('/').next().unwrap_or("unknown");
            return Some(ip_only.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use tempfile::tempdir;

    #[test]
    fn format_uptime_formats_days() {
        let value = format_uptime(172800 + 3661);
        assert_eq!(value, "2 days, 01:01:01");
    }

    #[test]
    fn format_uptime_formats_without_days() {
        let value = format_uptime(3661);
        assert_eq!(value, "01:01:01");
    }

    #[test]
    fn resolve_modules_uses_defaults_when_unset() {
        let selection = resolve_modules(&MotdConfig::default());
        assert_eq!(selection.source, ModuleSource::Default);
        assert_eq!(selection.modules, default_modules());
    }

    #[test]
    fn resolve_modules_normalizes_aliases_and_skips_unknowns() {
        let cfg = MotdConfig {
            modules: Some(vec![
                "HOST".into(),
                "nic".into(),
                "virt".into(),
                "bogus".into(),
                "network".into(),
            ]),
            ..MotdConfig::default()
        };

        let selection = resolve_modules(&cfg);
        assert_eq!(selection.source, ModuleSource::Configured);
        assert_eq!(
            selection.modules,
            vec![
                ModuleKind::Host,
                ModuleKind::Network,
                ModuleKind::Virtualization
            ]
        );
        assert_eq!(selection.ignored, vec!["bogus".to_string()]);
    }

    #[test]
    fn fetch_welcome_text_defaults_when_empty() {
        let resolution = resolve_welcome_text(&MotdConfig::default());
        assert_eq!(resolution.source, WelcomeSource::Default);
        assert_eq!(resolution.text, DEFAULT_WELCOME);
    }

    #[test]
    fn fetch_welcome_text_returns_literal_strings() {
        let cfg = MotdConfig {
            welcome: Some("Plain text".into()),
            ..MotdConfig::default()
        };
        let resolution = resolve_welcome_text(&cfg);
        assert_eq!(resolution.source, WelcomeSource::Literal);
        assert_eq!(resolution.text, "Plain text");
    }

    #[test]
    fn fetch_welcome_text_defaults_for_unsupported_url_scheme() {
        let cfg = MotdConfig {
            welcome: Some("mailto:test@example.com".into()),
            ..MotdConfig::default()
        };
        let resolution = resolve_welcome_text(&cfg);
        assert_eq!(resolution.source, WelcomeSource::Default);
        assert_eq!(resolution.text, DEFAULT_WELCOME);
    }

    #[test]
    fn write_and_read_welcome_cache_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("welcome.cache");
        let entry = WelcomeCacheEntry {
            url: "https://example.com/motd.txt".into(),
            fetched_at_secs: 123,
            body: "hello\nworld".into(),
        };

        write_welcome_cache(&path, &entry).unwrap();
        let restored = read_welcome_cache(&path).unwrap().unwrap();

        assert_eq!(restored.url, entry.url);
        assert_eq!(restored.fetched_at_secs, entry.fetched_at_secs);
        assert_eq!(restored.body, entry.body);
    }

    #[test]
    fn resolve_welcome_uses_fresh_cache_before_fetch() {
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("welcome.cache");
        write_welcome_cache(
            &cache_path,
            &WelcomeCacheEntry {
                url: "https://example.com/motd.txt".into(),
                fetched_at_secs: current_unix_secs(),
                body: "cached welcome".into(),
            },
        )
        .unwrap();

        let cfg = MotdConfig {
            welcome: Some("https://example.com/motd.txt".into()),
            remote_welcome: crate::config::RemoteWelcomeConfig {
                cache_path: Some(cache_path.display().to_string()),
                ..crate::config::RemoteWelcomeConfig::default()
            },
            ..MotdConfig::default()
        };

        let resolution = resolve_welcome_text(&cfg);
        assert_eq!(resolution.source, WelcomeSource::CacheFresh);
        assert_eq!(resolution.text, "cached welcome");
    }

    #[test]
    fn resolve_welcome_uses_stale_cache_when_remote_disabled() {
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("welcome.cache");
        write_welcome_cache(
            &cache_path,
            &WelcomeCacheEntry {
                url: "https://example.com/motd.txt".into(),
                fetched_at_secs: 1,
                body: "stale welcome".into(),
            },
        )
        .unwrap();

        let cfg = MotdConfig {
            welcome: Some("https://example.com/motd.txt".into()),
            remote_welcome: crate::config::RemoteWelcomeConfig {
                enabled: Some(false),
                cache_path: Some(cache_path.display().to_string()),
                cache_ttl_secs: Some(0),
                ..crate::config::RemoteWelcomeConfig::default()
            },
            ..MotdConfig::default()
        };

        let resolution = resolve_welcome_text(&cfg);
        assert_eq!(resolution.source, WelcomeSource::CacheStale);
        assert_eq!(resolution.text, "stale welcome");
        assert!(resolution
            .warnings
            .iter()
            .any(|warning| warning.contains("disabled")));
    }

    #[test]
    fn resolve_welcome_reports_malformed_cache() {
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("welcome.cache");
        fs::write(&cache_path, "broken cache").unwrap();

        let cfg = MotdConfig {
            welcome: Some("https://example.com".into()),
            remote_welcome: crate::config::RemoteWelcomeConfig {
                enabled: Some(false),
                cache_path: Some(cache_path.display().to_string()),
                ..crate::config::RemoteWelcomeConfig::default()
            },
            ..MotdConfig::default()
        };

        let resolution = resolve_welcome_text(&cfg);
        assert_eq!(resolution.source, WelcomeSource::Default);
        assert!(resolution
            .warnings
            .iter()
            .any(|warning| warning.contains("malformed")));
    }

    #[test]
    fn resolve_welcome_cache_matches_normalized_url() {
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("welcome.cache");
        write_welcome_cache(
            &cache_path,
            &WelcomeCacheEntry {
                url: "https://example.com/".into(),
                fetched_at_secs: current_unix_secs(),
                body: "cached normalized welcome".into(),
            },
        )
        .unwrap();

        let cfg = MotdConfig {
            welcome: Some("https://example.com:443/#section".into()),
            remote_welcome: crate::config::RemoteWelcomeConfig {
                cache_path: Some(cache_path.display().to_string()),
                ..crate::config::RemoteWelcomeConfig::default()
            },
            ..MotdConfig::default()
        };

        let resolution = resolve_welcome_text(&cfg);
        assert_eq!(resolution.source, WelcomeSource::CacheFresh);
        assert_eq!(resolution.text, "cached normalized welcome");
    }

    #[test]
    fn remote_welcome_rejects_non_success_status_without_cache() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0_u8; 512];
                let _ = stream.read(&mut buf);
                let response = concat!(
                    "HTTP/1.1 404 Not Found\r\n",
                    "Content-Length: 9\r\n",
                    "Connection: close\r\n",
                    "\r\n",
                    "not found"
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });

        let cfg = MotdConfig {
            welcome: Some(format!("http://{}/motd.txt", addr)),
            remote_welcome: crate::config::RemoteWelcomeConfig {
                allow_http: Some(true),
                timeout_ms: Some(500),
                cache_path: Some(
                    tempdir()
                        .unwrap()
                        .path()
                        .join("welcome.cache")
                        .display()
                        .to_string(),
                ),
                ..crate::config::RemoteWelcomeConfig::default()
            },
            ..MotdConfig::default()
        };

        let resolution = resolve_welcome_text(&cfg);
        server.join().unwrap();

        assert_eq!(resolution.source, WelcomeSource::Default);
        assert_eq!(resolution.text, DEFAULT_WELCOME);
        assert!(resolution
            .warnings
            .iter()
            .any(|warning| warning.contains("HTTP 404")));
    }

    #[test]
    fn to_gb_and_ratio_handles_zero_total() {
        let (used, total, ratio) = to_gb_and_ratio(0, 0);
        assert_eq!(used, 0.0);
        assert_eq!(total, 0.0);
        assert_eq!(ratio, 0.0);
    }
}
