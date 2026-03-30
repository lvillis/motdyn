use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
#[repr(C)]
pub(super) struct LinuxUtmpExitStatus {
    pub(super) e_termination: i16,
    pub(super) e_exit: i16,
}

#[cfg(target_os = "linux")]
#[repr(C)]
pub(super) struct LinuxUtmpTimeVal32 {
    pub(super) tv_sec: i32,
    pub(super) tv_usec: i32,
}

#[cfg(target_os = "linux")]
#[repr(C)]
pub(super) struct LinuxUtmpRecord {
    pub(super) ut_type: i16,
    pub(super) ut_pid: i32,
    pub(super) ut_line: [u8; 32],
    pub(super) ut_id: [u8; 4],
    pub(super) ut_user: [u8; 32],
    pub(super) ut_host: [u8; 256],
    pub(super) ut_exit: LinuxUtmpExitStatus,
    pub(super) ut_session: i32,
    pub(super) ut_tv: LinuxUtmpTimeVal32,
    pub(super) ut_addr_v6: [i32; 4],
    pub(super) __unused: [u8; 20],
}

pub(super) const DEFAULT_WELCOME: &str = "Welcome!";
pub(super) const DEFAULT_FAREWELL: &str = "Have a nice day!";
pub(super) const DEFAULT_WELCOME_TIMEOUT_MS: u64 = 250;
pub(super) const DEFAULT_WELCOME_CACHE_TTL_SECS: u64 = 300;
pub(super) const DEFAULT_WELCOME_CACHE_PATH: &str = "~/.cache/motdyn/welcome.txt";
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
    Configured,
    FallbackDefault,
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
    pub(super) disk_items: Vec<RenderedItem>,
    pub(super) last_login: String,
    pub(super) failed_login: String,
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
    RemoteFresh,
    CacheFresh,
    CacheRevalidated,
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
    SourceFailed(String),
    FileUrlUnsupportedHost(String),
    LocalFileRead { path: PathBuf, message: String },
    LocalFileEmpty { path: PathBuf },
    UnsupportedUrlScheme(String),
    HttpDisabled,
    EmbeddedCredentials,
    RemoteDisabled,
    CacheRead { path: PathBuf, message: String },
    CacheMalformed { path: PathBuf, reason: &'static str },
    CacheWrite { path: PathBuf, message: String },
    BuildClient(String),
    Fetch(String),
    HttpStatus(String),
    Decode(String),
    EmptyResponse,
}

impl fmt::Display for WelcomeIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceFailed(source) => {
                write!(
                    f,
                    "welcome source '{}' did not yield usable content",
                    source
                )
            }
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
            Self::UnsupportedUrlScheme(scheme) => write!(
                f,
                "unsupported URL scheme '{}'; using cached or default welcome",
                scheme
            ),
            Self::HttpDisabled => {
                write!(
                    f,
                    "HTTP welcome URLs are disabled; using cached or default welcome"
                )
            }
            Self::EmbeddedCredentials => write!(
                f,
                "URLs with embedded credentials are not supported; using cached or default welcome"
            ),
            Self::RemoteDisabled => {
                write!(
                    f,
                    "remote welcome fetch disabled; using cached or default welcome"
                )
            }
            Self::CacheRead { path, message } => {
                write!(f, "failed to read cache '{}': {}", path.display(), message)
            }
            Self::CacheMalformed { path, reason } => {
                write!(f, "cache '{}' is {}", path.display(), reason)
            }
            Self::CacheWrite { path, message } => {
                write!(f, "failed to write cache '{}': {}", path.display(), message)
            }
            Self::BuildClient(message) => {
                write!(f, "failed to build reqx client: {}", message)
            }
            Self::Fetch(message) => {
                write!(f, "failed to fetch remote welcome: {}", message)
            }
            Self::HttpStatus(status) => write!(
                f,
                "remote welcome returned HTTP {}; using cached or default welcome",
                status
            ),
            Self::Decode(message) => {
                write!(f, "failed to decode remote welcome: {}", message)
            }
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
    Cyan,
    Yellow,
    Green,
    Magenta,
}
