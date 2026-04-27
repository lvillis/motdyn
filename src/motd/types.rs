use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
pub(super) const LINUX_UTMP_RECORD_SIZE: usize = 384;
#[cfg(target_os = "linux")]
pub(super) const LINUX_UTMP_TYPE_OFFSET: usize = 0;
#[cfg(target_os = "linux")]
pub(super) const LINUX_UTMP_USER_OFFSET: usize = 44;
#[cfg(target_os = "linux")]
pub(super) const LINUX_UTMP_USER_LEN: usize = 32;
#[cfg(target_os = "linux")]
pub(super) const LINUX_USER_PROCESS: i16 = 7;

pub(super) const DEFAULT_WELCOME: &str = "Welcome!";
pub(super) const DEFAULT_FAREWELL: &str = "Have a nice day!";
pub(super) const DEFAULT_WELCOME_TIMEOUT_MS: u64 = 250;
pub(super) const DEFAULT_WELCOME_CACHE_TTL_SECS: u64 = 300;
pub(super) const DEFAULT_WELCOME_CACHE_PATH: &str = "~/.cache/motdyn/welcome.txt";
#[cfg(feature = "remote-welcome")]
pub(super) const MAX_WELCOME_BODY_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct RenderContext {
    pub system_config_path: String,
    pub system_config_status: String,
    pub user_config_path: String,
    pub user_config_status: String,
    pub config_notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ModuleKind {
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

impl ModuleKind {
    pub(super) fn key(self) -> &'static str {
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
pub(super) enum ModuleSource {
    Default,
    RoleProfile,
    ProfileFull,
    ProfileBasic,
    Configured,
    FallbackDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleProfile {
    Auto,
    Full,
    Basic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewerRole {
    Root,
    User,
}

#[derive(Debug, Clone)]
pub(super) struct ModuleSelection {
    pub(super) modules: Vec<ModuleKind>,
    pub(super) ignored: Vec<String>,
    pub(super) source: ModuleSource,
}

#[derive(Debug, Clone)]
pub(super) struct RenderedItem {
    pub(super) label: String,
    pub(super) value: String,
}

#[derive(Debug, Clone)]
pub(super) struct SystemSnapshot {
    pub(super) host_name: String,
    pub(super) main_iface: String,
    pub(super) main_ip: String,
    pub(super) current_user: String,
    pub(super) from_ip: String,
    pub(super) login_user_count: usize,
    pub(super) now_str_with_tz: String,
    pub(super) uptime_str: String,
    pub(super) load_average: String,
    pub(super) os_name: String,
    pub(super) os_version: String,
    pub(super) kernel_version: String,
    pub(super) virt_info: Option<String>,
    pub(super) cpu_brand: String,
    pub(super) cpu_count: usize,
    pub(super) memory: UsageSummary,
    pub(super) swap: UsageSummary,
    pub(super) root_disk: Option<UsageSummary>,
    pub(super) disk_items: Vec<RenderedItem>,
    pub(super) last_login: LastLoginInfo,
    pub(super) failed_login: FailedLoginInfo,
    pub(super) service_items: Vec<RenderedItem>,
    pub(super) update_summary: String,
    pub(super) diagnostics: SnapshotDiagnostics,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct UsageSummary {
    pub(super) used_gb: f64,
    pub(super) total_gb: f64,
    pub(super) ratio: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoginSessionKind {
    Ssh,
    Console,
    Unknown,
}

impl LoginSessionKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::Console => "console",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceRelation {
    Same,
    Different,
    Unknown,
}

impl SourceRelation {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Same => "same source as current session",
            Self::Different => "different from current session",
            Self::Unknown => "source relation unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LastLoginInfo {
    Unavailable,
    NeverRecorded,
    Recorded(LastLoginRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LastLoginRecord {
    pub(super) when: String,
    pub(super) from: Option<String>,
    pub(super) via: Option<String>,
    pub(super) kind: LoginSessionKind,
    pub(super) age: Option<String>,
    pub(super) source_relation: SourceRelation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FailedLoginInfo {
    Unavailable,
    None,
    Summary(FailedLoginSummary),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FailedLoginSeverity {
    Low,
    Warn,
    High,
}

impl FailedLoginSeverity {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Warn => "warn",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FailedLoginBucket {
    pub(super) value: String,
    pub(super) count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FailedLoginSummary {
    pub(super) total: usize,
    pub(super) count_24h: usize,
    pub(super) count_7d: usize,
    pub(super) last_when: Option<String>,
    pub(super) last_from: Option<String>,
    pub(super) last_via: Option<String>,
    pub(super) top_sources: Vec<FailedLoginBucket>,
    pub(super) top_vias: Vec<FailedLoginBucket>,
    pub(super) unique_sources: usize,
    pub(super) severity: FailedLoginSeverity,
    pub(super) current_source_seen: bool,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteWelcomeSettings {
    pub(super) enabled: bool,
    pub(super) timeout_ms: u64,
    pub(super) cache_ttl_secs: u64,
    pub(super) cache_path: PathBuf,
    pub(super) follow_redirects: bool,
    pub(super) allow_http: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WelcomeSource {
    Default,
    Literal,
    LocalFile,
    #[cfg(feature = "remote-welcome")]
    RemoteFresh,
    #[cfg(feature = "remote-welcome")]
    CacheFresh,
    #[cfg(feature = "remote-welcome")]
    CacheRevalidated,
    #[cfg(feature = "remote-welcome")]
    CacheStale,
}

#[derive(Debug, Clone)]
pub(super) struct WelcomeResolution {
    pub(super) text: String,
    pub(super) source: WelcomeSource,
    pub(super) source_detail: String,
    pub(super) url: Option<String>,
    pub(super) settings: RemoteWelcomeSettings,
    pub(super) warnings: Vec<WelcomeIssue>,
}

#[cfg(feature = "remote-welcome")]
#[derive(Debug, Clone)]
pub(super) struct WelcomeCacheEntry {
    pub(super) url: String,
    pub(super) fetched_at_secs: u64,
    pub(super) etag: Option<String>,
    pub(super) last_modified: Option<String>,
    pub(super) body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WelcomeIssue {
    #[cfg(feature = "remote-welcome")]
    SourceFailed(String),
    #[cfg(feature = "remote-welcome")]
    FileUrlUnsupportedHost(String),
    LocalFileRead {
        path: PathBuf,
        message: String,
    },
    LocalFileEmpty {
        path: PathBuf,
    },
    #[cfg(feature = "remote-welcome")]
    UnsupportedUrlScheme(String),
    #[cfg(not(feature = "remote-welcome"))]
    UrlSupportDisabled(String),
    #[cfg(feature = "remote-welcome")]
    HttpDisabled,
    #[cfg(feature = "remote-welcome")]
    EmbeddedCredentials,
    #[cfg(feature = "remote-welcome")]
    RemoteDisabled,
    #[cfg(feature = "remote-welcome")]
    CacheRead {
        path: PathBuf,
        message: String,
    },
    #[cfg(feature = "remote-welcome")]
    CacheMalformed {
        path: PathBuf,
        reason: &'static str,
    },
    #[cfg(feature = "remote-welcome")]
    CacheWrite {
        path: PathBuf,
        message: String,
    },
    #[cfg(feature = "remote-welcome")]
    BuildClient(String),
    #[cfg(feature = "remote-welcome")]
    Fetch(String),
    #[cfg(feature = "remote-welcome")]
    HttpStatus(String),
    #[cfg(feature = "remote-welcome")]
    Decode(String),
    #[cfg(feature = "remote-welcome")]
    EmptyResponse,
}

impl fmt::Display for WelcomeIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(feature = "remote-welcome")]
            Self::SourceFailed(source) => {
                write!(
                    f,
                    "welcome source '{}' did not yield usable content",
                    source
                )
            }
            #[cfg(feature = "remote-welcome")]
            Self::FileUrlUnsupportedHost(host) => {
                write!(f, "file URL host '{}' is not supported", host)
            }
            Self::LocalFileRead { path, message } => {
                write!(
                    f,
                    "failed to read local welcome file '{}': {}",
                    path.display(),
                    message
                )
            }
            Self::LocalFileEmpty { path } => {
                write!(f, "local welcome file '{}' was empty", path.display())
            }
            #[cfg(feature = "remote-welcome")]
            Self::UnsupportedUrlScheme(scheme) => write!(
                f,
                "unsupported URL scheme '{}'; using cached or default welcome",
                scheme
            ),
            #[cfg(not(feature = "remote-welcome"))]
            Self::UrlSupportDisabled(scheme) => write!(
                f,
                "welcome URL scheme '{}' requires the remote-welcome feature; using default welcome",
                scheme
            ),
            #[cfg(feature = "remote-welcome")]
            Self::HttpDisabled => {
                write!(
                    f,
                    "HTTP welcome URLs are disabled; using cached or default welcome"
                )
            }
            #[cfg(feature = "remote-welcome")]
            Self::EmbeddedCredentials => write!(
                f,
                "URLs with embedded credentials are not supported; using cached or default welcome"
            ),
            #[cfg(feature = "remote-welcome")]
            Self::RemoteDisabled => {
                write!(
                    f,
                    "remote welcome fetch disabled; using cached or default welcome"
                )
            }
            #[cfg(feature = "remote-welcome")]
            Self::CacheRead { path, message } => {
                write!(f, "failed to read cache '{}': {}", path.display(), message)
            }
            #[cfg(feature = "remote-welcome")]
            Self::CacheMalformed { path, reason } => {
                write!(f, "cache '{}' is {}", path.display(), reason)
            }
            #[cfg(feature = "remote-welcome")]
            Self::CacheWrite { path, message } => {
                write!(f, "failed to write cache '{}': {}", path.display(), message)
            }
            #[cfg(feature = "remote-welcome")]
            Self::BuildClient(message) => {
                write!(f, "failed to build reqx client: {}", message)
            }
            #[cfg(feature = "remote-welcome")]
            Self::Fetch(message) => {
                write!(f, "failed to fetch remote welcome: {}", message)
            }
            #[cfg(feature = "remote-welcome")]
            Self::HttpStatus(status) => write!(
                f,
                "remote welcome returned HTTP {}; using cached or default welcome",
                status
            ),
            #[cfg(feature = "remote-welcome")]
            Self::Decode(message) => {
                write!(f, "failed to decode remote welcome: {}", message)
            }
            #[cfg(feature = "remote-welcome")]
            Self::EmptyResponse => write!(f, "remote welcome response was empty"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NetworkProbeError {
    DefaultRouteCommand(String),
    DefaultRouteStatus(String),
    DefaultRouteParse,
    InterfaceIpv4Command { iface: String, message: String },
    InterfaceIpv4Status { iface: String, status: String },
    InterfaceIpv4Parse { iface: String },
}

impl fmt::Display for NetworkProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefaultRouteCommand(message) => {
                write!(f, "failed to run 'ip route show default': {}", message)
            }
            Self::DefaultRouteStatus(status) => {
                write!(f, "'ip route show default' exited with status {}", status)
            }
            Self::DefaultRouteParse => {
                write!(
                    f,
                    "no usable default route found in 'ip route show default' output"
                )
            }
            Self::InterfaceIpv4Command { iface, message } => write!(
                f,
                "failed to run 'ip -o -4 addr show dev {}': {}",
                iface, message
            ),
            Self::InterfaceIpv4Status { iface, status } => write!(
                f,
                "'ip -o -4 addr show dev {}' exited with status {}",
                iface, status
            ),
            Self::InterfaceIpv4Parse { iface } => {
                write!(f, "no IPv4 address found for interface '{}'", iface)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProbeIssue {
    Network(NetworkProbeError),
    UptimeReadFailed,
    LoadAverageReadFailed,
    HostReadFailed,
    KernelReadFailed,
    OsMetadataMissing,
    VirtualizationProbeFailed(String),
    CpuInfoUnstable,
    MemoryInfoMissing,
    SshConnectionMissing,
    LastLoginProbeFailed(String),
    FailedLoginProbeFailed(String),
    ServiceStatusProbeFailed(String),
    UpdateProbeFailed(String),
}

impl fmt::Display for ProbeIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(err) => write!(f, "network: {}", err),
            Self::UptimeReadFailed => {
                write!(f, "uptime: failed to read or parse /proc/uptime")
            }
            Self::LoadAverageReadFailed => {
                write!(f, "load: failed to read or parse /proc/loadavg")
            }
            Self::HostReadFailed => {
                write!(f, "host: failed to read /proc/sys/kernel/hostname")
            }
            Self::KernelReadFailed => {
                write!(f, "kernel: failed to read /proc/sys/kernel/osrelease")
            }
            Self::OsMetadataMissing => {
                write!(f, "os: no release metadata found; using kernel fallback")
            }
            Self::VirtualizationProbeFailed(message) => {
                write!(f, "virtualization: {}", message)
            }
            Self::CpuInfoUnstable => {
                write!(
                    f,
                    "cpu: /proc/cpuinfo did not yield a stable brand/core count"
                )
            }
            Self::MemoryInfoMissing => {
                write!(f, "memory: /proc/meminfo missing or unreadable")
            }
            Self::SshConnectionMissing => {
                write!(
                    f,
                    "user: SSH_CONNECTION missing; source IP shown as unknown"
                )
            }
            Self::LastLoginProbeFailed(message) => {
                write!(f, "last_login: {}", message)
            }
            Self::FailedLoginProbeFailed(message) => {
                write!(f, "failed_login: {}", message)
            }
            Self::ServiceStatusProbeFailed(message) => {
                write!(f, "services: {}", message)
            }
            Self::UpdateProbeFailed(message) => {
                write!(f, "updates: {}", message)
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct SnapshotDiagnostics {
    pub(super) degraded_modules: Vec<ModuleKind>,
    pub(super) issues: Vec<ProbeIssue>,
    pub(super) os_source: String,
    pub(super) network_source: String,
    pub(super) login_user_count_source: String,
    pub(super) virtualization_source: String,
    pub(super) load_source: String,
    pub(super) last_login_source: String,
    pub(super) failed_login_source: String,
    pub(super) service_status_source: String,
    pub(super) updates_source: String,
}

impl SnapshotDiagnostics {
    pub(super) fn degrade(&mut self, module: ModuleKind, issue: ProbeIssue) {
        if !self.degraded_modules.contains(&module) {
            self.degraded_modules.push(module);
        }
        self.issues.push(issue);
    }

    pub(super) fn note(&mut self, issue: ProbeIssue) {
        self.issues.push(issue);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum HiddenField {
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

#[derive(Debug, Clone)]
pub(super) struct OutputSettings {
    pub(super) compact: bool,
    pub(super) plain: bool,
    pub(super) section_headers: bool,
    pub(super) hidden_fields: HashSet<HiddenField>,
    pub(super) ignored_hidden_fields: Vec<String>,
}

impl OutputSettings {
    pub(super) fn hidden(&self, field: HiddenField) -> bool {
        self.hidden_fields.contains(&field)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SectionKind {
    Identity,
    Runtime,
    System,
    Storage,
    Operations,
}

impl SectionKind {
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Identity => "Identity",
            Self::Runtime => "Runtime",
            Self::System => "System",
            Self::Storage => "Storage",
            Self::Operations => "Operations",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum PaintKind {
    Label,
    Header,
    Dim,
    Cyan,
    Yellow,
    Red,
    Green,
    Magenta,
}
