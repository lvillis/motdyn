use std::collections::HashSet;
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
pub(super) const DEFAULT_WELCOME_TIMEOUT_MS: u64 = 1_000;
pub(super) const DEFAULT_WELCOME_CACHE_TTL_SECS: u64 = 300;
pub(super) const DEFAULT_WELCOME_CACHE_PATH: &str = "~/.cache/motdyn/welcome.txt";
pub(super) const MAX_WELCOME_BODY_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct RenderContext {
    pub system_config_path: String,
    pub system_config_loaded: bool,
    pub user_config_path: String,
    pub user_config_loaded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ModuleKind {
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
    pub(super) fn key(self) -> &'static str {
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
    pub(super) os_name: String,
    pub(super) os_version: String,
    pub(super) kernel_version: String,
    pub(super) virt_info: Option<String>,
    pub(super) cpu_brand: String,
    pub(super) cpu_count: usize,
    pub(super) memory: UsageSummary,
    pub(super) swap: UsageSummary,
    pub(super) disk_items: Vec<RenderedItem>,
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
    RemoteFresh,
    CacheFresh,
    CacheStale,
}

#[derive(Debug, Clone)]
pub(super) struct WelcomeResolution {
    pub(super) text: String,
    pub(super) source: WelcomeSource,
    pub(super) source_detail: String,
    pub(super) url: Option<String>,
    pub(super) settings: RemoteWelcomeSettings,
    pub(super) warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct WelcomeCacheEntry {
    pub(super) url: String,
    pub(super) fetched_at_secs: u64,
    pub(super) body: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct SnapshotDiagnostics {
    pub(super) degraded_modules: Vec<ModuleKind>,
    pub(super) notes: Vec<String>,
    pub(super) os_source: String,
    pub(super) network_source: String,
    pub(super) login_user_count_source: String,
    pub(super) virtualization_source: String,
}

impl SnapshotDiagnostics {
    pub(super) fn degrade(&mut self, module: ModuleKind, note: impl Into<String>) {
        if !self.degraded_modules.contains(&module) {
            self.degraded_modules.push(module);
        }
        self.notes.push(note.into());
    }

    pub(super) fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
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
}

impl SectionKind {
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Identity => "Identity",
            Self::Runtime => "Runtime",
            Self::System => "System",
            Self::Storage => "Storage",
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
