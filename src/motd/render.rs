use std::collections::HashSet;
use std::env;

#[cfg(feature = "color")]
use colored::Colorize;
#[cfg(unix)]
use rustix::process::geteuid;

use crate::config::MotdConfig;

use super::types::{
    FailedLoginBucket, FailedLoginInfo, FailedLoginSeverity, HiddenField, LastLoginInfo,
    LastLoginRecord, ModuleKind, ModuleProfile, ModuleSelection, ModuleSource, OutputSettings,
    PaintKind, RenderContext, RenderedItem, SectionKind, SourceRelation, SystemSnapshot,
    UsageSummary, ViewerRole, WelcomeResolution, WelcomeSource,
};

pub(super) fn build_verbose_items(
    cfg: &MotdConfig,
    ctx: &RenderContext,
    selection: &ModuleSelection,
    welcome: &WelcomeResolution,
    snapshot: &SystemSnapshot,
    output: &OutputSettings,
) -> Vec<RenderedItem> {
    let mut items = vec![
        RenderedItem {
            label: "System config:".to_string(),
            value: format!("{} ({})", ctx.system_config_path, ctx.system_config_status),
        },
        RenderedItem {
            label: "User config:".to_string(),
            value: format!("{} ({})", ctx.user_config_path, ctx.user_config_status),
        },
        RenderedItem {
            label: "Module source:".to_string(),
            value: match selection.source {
                ModuleSource::Default => "automatic full profile".to_string(),
                ModuleSource::RoleProfile => "automatic basic profile".to_string(),
                ModuleSource::ProfileFull => "forced full profile".to_string(),
                ModuleSource::ProfileBasic => "forced basic profile".to_string(),
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
            label: "Output mode:".to_string(),
            value: format!(
                "compact={}, plain={}, section_headers={}",
                output.compact, output.plain, output.section_headers
            ),
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
                WelcomeSource::LocalFile => "local file".to_string(),
                #[cfg(feature = "remote-welcome")]
                WelcomeSource::RemoteFresh => "remote fetch".to_string(),
                #[cfg(feature = "remote-welcome")]
                WelcomeSource::CacheFresh => "fresh cache".to_string(),
                #[cfg(feature = "remote-welcome")]
                WelcomeSource::CacheRevalidated => "cache revalidated".to_string(),
                #[cfg(feature = "remote-welcome")]
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
        RenderedItem {
            label: "OS source:".to_string(),
            value: snapshot.diagnostics.os_source.clone(),
        },
        RenderedItem {
            label: "Network source:".to_string(),
            value: snapshot.diagnostics.network_source.clone(),
        },
        RenderedItem {
            label: "Login count source:".to_string(),
            value: snapshot.diagnostics.login_user_count_source.clone(),
        },
        RenderedItem {
            label: "Virt source:".to_string(),
            value: snapshot.diagnostics.virtualization_source.clone(),
        },
    ];

    if !snapshot.diagnostics.load_source.is_empty() {
        items.push(RenderedItem {
            label: "Load source:".to_string(),
            value: snapshot.diagnostics.load_source.clone(),
        });
    }
    if !snapshot.diagnostics.last_login_source.is_empty() {
        items.push(RenderedItem {
            label: "Last login source:".to_string(),
            value: snapshot.diagnostics.last_login_source.clone(),
        });
    }
    if !snapshot.diagnostics.failed_login_source.is_empty() {
        items.push(RenderedItem {
            label: "Failed login source:".to_string(),
            value: snapshot.diagnostics.failed_login_source.clone(),
        });
    }
    if !snapshot.diagnostics.service_status_source.is_empty() {
        items.push(RenderedItem {
            label: "Service source:".to_string(),
            value: snapshot.diagnostics.service_status_source.clone(),
        });
    }
    if !snapshot.diagnostics.updates_source.is_empty() {
        items.push(RenderedItem {
            label: "Update source:".to_string(),
            value: snapshot.diagnostics.updates_source.clone(),
        });
    }

    if selection.modules.contains(&ModuleKind::LastLogin) {
        match &snapshot.last_login {
            LastLoginInfo::Recorded(record) => {
                items.push(RenderedItem {
                    label: "Last login record:".to_string(),
                    value: format_last_login_verbose(record),
                });
                items.push(RenderedItem {
                    label: "Last login kind:".to_string(),
                    value: record.kind.label().to_string(),
                });
                if let Some(age) = &record.age {
                    items.push(RenderedItem {
                        label: "Last login age:".to_string(),
                        value: age.clone(),
                    });
                }
                if let Some(from) = &record.from {
                    items.push(RenderedItem {
                        label: "Last login source IP:".to_string(),
                        value: from.clone(),
                    });
                }
                if let Some(via) = &record.via {
                    items.push(RenderedItem {
                        label: "Last login via:".to_string(),
                        value: via.clone(),
                    });
                }
                if !matches!(record.source_relation, SourceRelation::Unknown) {
                    items.push(RenderedItem {
                        label: "Last login relation:".to_string(),
                        value: record.source_relation.label().to_string(),
                    });
                }
            }
            LastLoginInfo::NeverRecorded => items.push(RenderedItem {
                label: "Last login status:".to_string(),
                value: "never recorded".to_string(),
            }),
            LastLoginInfo::Unavailable => {}
        }
    }

    if selection.modules.contains(&ModuleKind::FailedLogin) {
        match &snapshot.failed_login {
            FailedLoginInfo::Summary(summary) => {
                items.push(RenderedItem {
                    label: "Failed login severity:".to_string(),
                    value: summary.severity.label().to_string(),
                });
                items.push(RenderedItem {
                    label: "Failed login window:".to_string(),
                    value: format!(
                        "24h={}, 7d={}, total={}",
                        summary.count_24h, summary.count_7d, summary.total
                    ),
                });
                if let Some(last_when) = &summary.last_when {
                    items.push(RenderedItem {
                        label: "Failed login last seen:".to_string(),
                        value: last_when.clone(),
                    });
                }
                if !summary.top_sources.is_empty() {
                    items.push(RenderedItem {
                        label: "Failed login top sources:".to_string(),
                        value: format_buckets(&summary.top_sources),
                    });
                }
                if !summary.top_vias.is_empty() {
                    items.push(RenderedItem {
                        label: "Failed login top entrypoints:".to_string(),
                        value: format_buckets(&summary.top_vias),
                    });
                }
                if summary.current_source_seen {
                    items.push(RenderedItem {
                        label: "Failed login relation:".to_string(),
                        value: "current source appears in recent failures".to_string(),
                    });
                }
            }
            FailedLoginInfo::None => items.push(RenderedItem {
                label: "Failed login status:".to_string(),
                value: "none".to_string(),
            }),
            FailedLoginInfo::Unavailable => {}
        }
    }

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

    if !output.ignored_hidden_fields.is_empty() {
        items.push(RenderedItem {
            label: "Ignored fields:".to_string(),
            value: output.ignored_hidden_fields.join(", "),
        });
    }

    if !snapshot.diagnostics.degraded_modules.is_empty() {
        items.push(RenderedItem {
            label: "Degraded modules:".to_string(),
            value: snapshot
                .diagnostics
                .degraded_modules
                .iter()
                .map(|module| module.key())
                .collect::<Vec<_>>()
                .join(", "),
        });
    }

    if !welcome.warnings.is_empty() {
        items.push(RenderedItem {
            label: "Welcome notes:".to_string(),
            value: welcome
                .warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | "),
        });
    }

    if !snapshot.diagnostics.issues.is_empty() {
        items.push(RenderedItem {
            label: "Probe notes:".to_string(),
            value: snapshot
                .diagnostics
                .issues
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" | "),
        });
    }

    if !ctx.config_notes.is_empty() {
        items.push(RenderedItem {
            label: "Config notes:".to_string(),
            value: ctx.config_notes.join(" | "),
        });
    }

    items
}

pub(super) fn render_module_lines(
    modules: &[ModuleKind],
    snapshot: &SystemSnapshot,
    settings: &OutputSettings,
) -> Vec<String> {
    let combine_usage_bar = should_combine_usage_bar(modules, snapshot, settings);
    let mut groups: Vec<(SectionKind, Vec<RenderedItem>)> = Vec::new();

    for module in modules {
        let items = render_module_items(*module, snapshot, settings, combine_usage_bar);
        if items.is_empty() {
            continue;
        }

        let section = module_section(*module);
        if let Some((current_section, current_items)) = groups.last_mut()
            && *current_section == section
        {
            current_items.extend(items);
            continue;
        }
        groups.push((section, items));
    }

    if settings.compact {
        return format_compact_groups(groups, settings);
    }

    if !settings.section_headers {
        let items = groups
            .into_iter()
            .flat_map(|(_, items)| items)
            .collect::<Vec<_>>();
        return format_aligned_items(&items, settings);
    }

    let mut lines = Vec::new();
    for (idx, (section, items)) in groups.into_iter().enumerate() {
        if idx > 0 && !settings.compact {
            lines.push(String::new());
        }
        lines.push(paint(section.title(), PaintKind::Header, settings));
        lines.extend(format_aligned_items(&items, settings));
    }
    lines
}

pub(super) fn format_aligned_items(
    items: &[RenderedItem],
    settings: &OutputSettings,
) -> Vec<String> {
    let width = items.iter().map(|item| item.label.len()).max().unwrap_or(0);
    items
        .iter()
        .map(|item| {
            format!(
                "{:width$} {}",
                paint(item.label.clone(), PaintKind::Label, settings),
                item.value,
                width = width
            )
        })
        .collect()
}

fn format_compact_groups(
    groups: Vec<(SectionKind, Vec<RenderedItem>)>,
    settings: &OutputSettings,
) -> Vec<String> {
    let mut lines = Vec::new();

    for (idx, (section, items)) in groups.into_iter().enumerate() {
        if items.is_empty() {
            continue;
        }

        if settings.section_headers {
            if idx > 0 {
                lines.push(String::new());
            }
            lines.push(paint(section.title(), PaintKind::Header, settings));
            lines.extend(format_compact_section_body(
                None,
                &compact_clauses(section, &items),
                settings,
            ));
            continue;
        }

        let prefix = format!("{:<11}", format!("{}:", section.title()));
        lines.extend(format_compact_section_body(
            Some(paint(prefix, PaintKind::Header, settings)),
            &compact_clauses(section, &items),
            settings,
        ));
    }

    lines
}

fn format_compact_section_body(
    prefix: Option<String>,
    clauses: &[String],
    _settings: &OutputSettings,
) -> Vec<String> {
    let width = terminal_width_hint();
    let separator = "; ";
    let mut lines = Vec::new();
    let prefix_width = prefix
        .as_ref()
        .map(|value| visible_width(value))
        .unwrap_or(0);
    let continuation_prefix = " ".repeat(prefix_width);
    let mut current = prefix.unwrap_or_default();
    let mut current_width = visible_width(&current);

    for clause in clauses {
        let clause_width = visible_width(clause);
        let separator_width = if current_width == prefix_width {
            0
        } else {
            separator.len()
        };

        if current_width > prefix_width && current_width + separator_width + clause_width > width {
            lines.push(current);
            current = continuation_prefix.clone();
            current_width = prefix_width;
        }

        if current_width > prefix_width {
            current.push_str(separator);
            current_width += separator_width;
        } else if prefix_width > 0 {
            current.push(' ');
            current_width += 1;
        }

        current.push_str(clause);
        current_width += clause_width;
    }

    if !current.trim().is_empty() {
        lines.push(current);
    }

    lines
}

pub(super) fn resolve_output_settings(cfg: &MotdConfig) -> OutputSettings {
    let mut hidden_fields = HashSet::new();
    let mut ignored_hidden_fields = Vec::new();

    if let Some(fields) = cfg.output.hidden_fields.as_ref() {
        for raw in fields {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            match normalize_hidden_field_name(trimmed) {
                Some(field) => {
                    hidden_fields.insert(field);
                }
                None => ignored_hidden_fields.push(trimmed.to_string()),
            }
        }
    }

    OutputSettings {
        compact: cfg.output.compact.unwrap_or(false),
        plain: cfg.output.plain.unwrap_or(false),
        section_headers: cfg.output.section_headers.unwrap_or(false),
        hidden_fields,
        ignored_hidden_fields,
    }
}

pub(super) fn resolve_modules(
    cfg: &MotdConfig,
    viewer_role: ViewerRole,
    profile: ModuleProfile,
) -> ModuleSelection {
    let Some(configured) = cfg.modules.as_ref() else {
        return match profile {
            ModuleProfile::Auto => match viewer_role {
                ViewerRole::Root => ModuleSelection {
                    modules: default_modules(),
                    ignored: Vec::new(),
                    source: ModuleSource::Default,
                },
                ViewerRole::User => ModuleSelection {
                    modules: basic_modules(),
                    ignored: Vec::new(),
                    source: ModuleSource::RoleProfile,
                },
            },
            ModuleProfile::Full => ModuleSelection {
                modules: default_modules(),
                ignored: Vec::new(),
                source: ModuleSource::ProfileFull,
            },
            ModuleProfile::Basic => ModuleSelection {
                modules: basic_modules(),
                ignored: Vec::new(),
                source: ModuleSource::ProfileBasic,
            },
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

pub(super) fn default_modules() -> Vec<ModuleKind> {
    vec![
        ModuleKind::Host,
        ModuleKind::Network,
        ModuleKind::User,
        ModuleKind::Time,
        ModuleKind::Uptime,
        ModuleKind::Load,
        ModuleKind::Os,
        ModuleKind::Kernel,
        ModuleKind::Virtualization,
        ModuleKind::Cpu,
        ModuleKind::Memory,
        ModuleKind::Swap,
        ModuleKind::Disk,
        ModuleKind::LastLogin,
        ModuleKind::FailedLogin,
    ]
}

pub(super) fn basic_modules() -> Vec<ModuleKind> {
    vec![
        ModuleKind::Host,
        ModuleKind::Network,
        ModuleKind::User,
        ModuleKind::Time,
        ModuleKind::Uptime,
        ModuleKind::Load,
    ]
}

pub(super) fn current_viewer_role() -> ViewerRole {
    #[cfg(unix)]
    {
        if geteuid().is_root() {
            return ViewerRole::Root;
        }
    }

    ViewerRole::User
}

pub(super) fn paint(text: impl Into<String>, kind: PaintKind, settings: &OutputSettings) -> String {
    let text = text.into();
    if settings.plain {
        return text;
    }

    #[cfg(feature = "color")]
    {
        match kind {
            PaintKind::Label => text.bright_white().to_string(),
            PaintKind::Header => text.bold().cyan().to_string(),
            PaintKind::Dim => text.white().to_string(),
            PaintKind::Cyan => text.bright_cyan().to_string(),
            PaintKind::Yellow => text.bright_yellow().to_string(),
            PaintKind::Red => text.bright_red().to_string(),
            PaintKind::Green => text.bright_green().to_string(),
            PaintKind::Magenta => text.bright_magenta().to_string(),
        }
    }

    #[cfg(not(feature = "color"))]
    {
        let _ = kind;
        text
    }
}

fn render_module_items(
    module: ModuleKind,
    snapshot: &SystemSnapshot,
    settings: &OutputSettings,
    combine_usage_bar: bool,
) -> Vec<RenderedItem> {
    match module {
        ModuleKind::Host => vec![RenderedItem {
            label: "Host name:".to_string(),
            value: paint(snapshot.host_name.clone(), PaintKind::Yellow, settings),
        }],
        ModuleKind::Network => render_network_items(snapshot, settings),
        ModuleKind::User => render_user_items(snapshot, settings),
        ModuleKind::Time => render_time_items(snapshot, settings),
        ModuleKind::Uptime => vec![RenderedItem {
            label: "System uptime:".to_string(),
            value: paint(snapshot.uptime_str.clone(), PaintKind::Yellow, settings),
        }],
        ModuleKind::Load => vec![RenderedItem {
            label: "Load average:".to_string(),
            value: paint(snapshot.load_average.clone(), PaintKind::Yellow, settings),
        }],
        ModuleKind::Os => vec![RenderedItem {
            label: "Operating system:".to_string(),
            value: paint(
                format!("{} {}", snapshot.os_name, snapshot.os_version),
                PaintKind::Yellow,
                settings,
            ),
        }],
        ModuleKind::Kernel => {
            if settings.hidden(HiddenField::KernelVersion) {
                Vec::new()
            } else {
                vec![RenderedItem {
                    label: "Kernel version:".to_string(),
                    value: paint(snapshot.kernel_version.clone(), PaintKind::Green, settings),
                }]
            }
        }
        ModuleKind::Virtualization => {
            if settings.hidden(HiddenField::Virtualization) {
                Vec::new()
            } else if let Some(virt) = &snapshot.virt_info {
                vec![RenderedItem {
                    label: "Virtualization:".to_string(),
                    value: paint(virt.clone(), PaintKind::Yellow, settings),
                }]
            } else {
                Vec::new()
            }
        }
        ModuleKind::Cpu => vec![RenderedItem {
            label: "CPU:".to_string(),
            value: paint(
                format!("{} ({} cores)", snapshot.cpu_brand, snapshot.cpu_count),
                PaintKind::Magenta,
                settings,
            ),
        }],
        ModuleKind::Memory => render_memory_items(snapshot, settings, combine_usage_bar),
        ModuleKind::Swap => {
            if settings.hidden(HiddenField::Swap) || is_empty_usage(snapshot.swap) {
                Vec::new()
            } else {
                vec![RenderedItem {
                    label: "Swap used/total:".to_string(),
                    value: format_usage(snapshot.swap),
                }]
            }
        }
        ModuleKind::Disk => render_disk_items(snapshot, settings, !combine_usage_bar),
        ModuleKind::LastLogin => vec![RenderedItem {
            label: "Last login:".to_string(),
            value: render_last_login_value(&snapshot.last_login, settings),
        }],
        ModuleKind::FailedLogin => vec![RenderedItem {
            label: "Failed login:".to_string(),
            value: render_failed_login_value(&snapshot.failed_login, settings),
        }],
        ModuleKind::Services => render_service_items(snapshot, settings),
        ModuleKind::Updates => vec![RenderedItem {
            label: "Pending updates:".to_string(),
            value: paint(snapshot.update_summary.clone(), PaintKind::Yellow, settings),
        }],
    }
}

fn render_network_items(snapshot: &SystemSnapshot, settings: &OutputSettings) -> Vec<RenderedItem> {
    let mut parts = Vec::new();
    if !settings.hidden(HiddenField::MainInterface) {
        parts.push(paint(
            snapshot.main_iface.clone(),
            PaintKind::Cyan,
            settings,
        ));
    }
    if !settings.hidden(HiddenField::MainIpv4) {
        let ip = paint(snapshot.main_ip.clone(), PaintKind::Cyan, settings);
        if parts.is_empty() {
            parts.push(ip);
        } else {
            parts.push(format!("({})", ip));
        }
    }

    if parts.is_empty() {
        Vec::new()
    } else {
        vec![RenderedItem {
            label: "Main NIC:".to_string(),
            value: parts.join(" "),
        }]
    }
}

fn render_user_items(snapshot: &SystemSnapshot, settings: &OutputSettings) -> Vec<RenderedItem> {
    let mut value = paint(snapshot.current_user.clone(), PaintKind::Cyan, settings);

    if !settings.hidden(HiddenField::SourceIp) {
        value.push_str(&format!(
            " (from {})",
            paint(snapshot.from_ip.clone(), PaintKind::Cyan, settings)
        ));
    }
    if !settings.hidden(HiddenField::LoginUserCount) {
        value.push_str(&format!(
            ", {} user(s) logged in",
            paint(
                snapshot.login_user_count.to_string(),
                PaintKind::Cyan,
                settings,
            )
        ));
    }

    vec![RenderedItem {
        label: "User info:".to_string(),
        value,
    }]
}

fn render_time_items(snapshot: &SystemSnapshot, settings: &OutputSettings) -> Vec<RenderedItem> {
    let value = if settings.hidden(HiddenField::Timezone) {
        snapshot
            .now_str_with_tz
            .rsplit_once(' ')
            .map(|(time, _)| time.to_string())
            .unwrap_or_else(|| snapshot.now_str_with_tz.clone())
    } else {
        snapshot.now_str_with_tz.clone()
    };

    vec![RenderedItem {
        label: "Current time (TZ):".to_string(),
        value: paint(value, PaintKind::Yellow, settings),
    }]
}

fn render_memory_items(
    snapshot: &SystemSnapshot,
    _settings: &OutputSettings,
    combine_usage_bar: bool,
) -> Vec<RenderedItem> {
    if combine_usage_bar {
        return snapshot
            .root_disk
            .map(|root_disk| RenderedItem {
                label: "Resource use:".to_string(),
                value: format_combined_usage_bar(snapshot.memory, root_disk, _settings),
            })
            .into_iter()
            .collect();
    }

    vec![RenderedItem {
        label: "Memory used/total:".to_string(),
        value: format_usage(snapshot.memory),
    }]
}

fn render_disk_items(
    snapshot: &SystemSnapshot,
    settings: &OutputSettings,
    include_root: bool,
) -> Vec<RenderedItem> {
    snapshot
        .disk_items
        .iter()
        .filter(|item| {
            (include_root || !item.label.eq_ignore_ascii_case("Disk usage (root):"))
                && !(settings.hidden(HiddenField::NfsDisks)
                    && item.label.eq_ignore_ascii_case("Disk usage (nfs):"))
        })
        .cloned()
        .collect()
}

fn render_service_items(snapshot: &SystemSnapshot, settings: &OutputSettings) -> Vec<RenderedItem> {
    snapshot
        .service_items
        .iter()
        .map(|item| RenderedItem {
            label: item.label.clone(),
            value: paint_service_state(item.value.clone(), settings),
        })
        .collect()
}

fn module_section(module: ModuleKind) -> SectionKind {
    match module {
        ModuleKind::Host | ModuleKind::Network | ModuleKind::User => SectionKind::Identity,
        ModuleKind::Time | ModuleKind::Uptime | ModuleKind::Load => SectionKind::Runtime,
        ModuleKind::Os
        | ModuleKind::Kernel
        | ModuleKind::Virtualization
        | ModuleKind::Cpu
        | ModuleKind::Memory
        | ModuleKind::Swap => SectionKind::System,
        ModuleKind::Disk => SectionKind::Storage,
        ModuleKind::LastLogin
        | ModuleKind::FailedLogin
        | ModuleKind::Services
        | ModuleKind::Updates => SectionKind::Operations,
    }
}

fn normalize_hidden_field_name(name: &str) -> Option<HiddenField> {
    match name.to_ascii_lowercase().as_str() {
        "main_interface" | "interface" | "iface" => Some(HiddenField::MainInterface),
        "main_ipv4" | "main_ip" | "ipv4" | "ip" => Some(HiddenField::MainIpv4),
        "source_ip" | "from_ip" => Some(HiddenField::SourceIp),
        "login_user_count" | "logged_in_users" | "user_count" => Some(HiddenField::LoginUserCount),
        "timezone" | "tz" => Some(HiddenField::Timezone),
        "kernel_version" | "kernel" => Some(HiddenField::KernelVersion),
        "virtualization" | "virt" => Some(HiddenField::Virtualization),
        "swap" => Some(HiddenField::Swap),
        "nfs_disks" | "nfs" => Some(HiddenField::NfsDisks),
        _ => None,
    }
}

fn normalize_module_name(name: &str) -> Option<ModuleKind> {
    match name.to_ascii_lowercase().as_str() {
        "host" | "hostname" => Some(ModuleKind::Host),
        "network" | "net" | "nic" => Some(ModuleKind::Network),
        "user" | "users" | "login" => Some(ModuleKind::User),
        "time" | "clock" | "datetime" => Some(ModuleKind::Time),
        "uptime" => Some(ModuleKind::Uptime),
        "load" | "loadavg" | "load_average" => Some(ModuleKind::Load),
        "os" | "system" => Some(ModuleKind::Os),
        "kernel" => Some(ModuleKind::Kernel),
        "virtualization" | "virt" | "container" => Some(ModuleKind::Virtualization),
        "cpu" => Some(ModuleKind::Cpu),
        "memory" | "mem" => Some(ModuleKind::Memory),
        "swap" => Some(ModuleKind::Swap),
        "disk" | "disks" | "filesystem" | "fs" => Some(ModuleKind::Disk),
        "last_login" | "lastlogin" | "last" => Some(ModuleKind::LastLogin),
        "failed_login" | "failedlogin" | "failed" | "security" => Some(ModuleKind::FailedLogin),
        "services" | "service" | "systemd" => Some(ModuleKind::Services),
        "updates" | "update" | "packages" | "package_updates" => Some(ModuleKind::Updates),
        _ => None,
    }
}

fn format_usage(summary: UsageSummary) -> String {
    format!(
        "{:.2}/{:.2} GB ({:.2}%)",
        summary.used_gb, summary.total_gb, summary.ratio
    )
}

fn format_combined_usage_bar(
    memory: UsageSummary,
    root_disk: UsageSummary,
    settings: &OutputSettings,
) -> String {
    format!(
        "{}  {}",
        format_usage_meter("mem", memory.ratio, settings),
        format_usage_meter("disk", root_disk.ratio, settings)
    )
}

fn format_percent(value: f64) -> String {
    format!("{:>3.0}%", value.clamp(0.0, 100.0))
}

fn format_bar(value: f64, width: usize, settings: &OutputSettings) -> String {
    let clamped = value.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);

    let (filled_char, empty_char) = if settings.plain {
        ('#', '-')
    } else {
        ('█', '░')
    };

    std::iter::repeat_n(filled_char, filled)
        .chain(std::iter::repeat_n(
            empty_char,
            width.saturating_sub(filled),
        ))
        .collect()
}

fn format_usage_meter(label: &str, ratio: f64, settings: &OutputSettings) -> String {
    let label = paint(label, PaintKind::Dim, settings);
    let meter = match usage_status_label(ratio) {
        Some(status) => format!(
            "{} {} {}",
            format_percent(ratio),
            status,
            format_bar(ratio, 10, settings)
        ),
        None => format!(
            "{} {}",
            format_percent(ratio),
            format_bar(ratio, 10, settings)
        ),
    };
    format!(
        "{} {}",
        label,
        paint(meter, usage_paint_kind(ratio), settings)
    )
}

fn usage_paint_kind(ratio: f64) -> PaintKind {
    let _ = ratio;
    PaintKind::Dim
}

fn usage_status_label(ratio: f64) -> Option<&'static str> {
    if ratio >= 95.0 {
        Some("critical")
    } else if ratio >= 85.0 {
        Some("high")
    } else {
        None
    }
}

fn is_empty_usage(summary: UsageSummary) -> bool {
    summary.total_gb <= f64::EPSILON
}

fn should_combine_usage_bar(
    modules: &[ModuleKind],
    snapshot: &SystemSnapshot,
    settings: &OutputSettings,
) -> bool {
    !settings.compact
        && modules.contains(&ModuleKind::Memory)
        && modules.contains(&ModuleKind::Disk)
        && snapshot.root_disk.is_some()
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn command_exists(command: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|dir| dir.join(command).is_file())
}

fn paint_service_state(value: String, settings: &OutputSettings) -> String {
    match value.as_str() {
        "active" => paint(value, PaintKind::Green, settings),
        "inactive" | "failed" | "unavailable" => paint(value, PaintKind::Yellow, settings),
        _ => value,
    }
}

fn render_last_login_value(info: &LastLoginInfo, settings: &OutputSettings) -> String {
    match info {
        LastLoginInfo::Unavailable => paint("unavailable", PaintKind::Yellow, settings),
        LastLoginInfo::NeverRecorded => paint("never recorded", PaintKind::Green, settings),
        LastLoginInfo::Recorded(record) => {
            let value = if settings.compact {
                format_last_login_compact(record)
            } else {
                format_last_login_full(record)
            };
            paint(value, PaintKind::Yellow, settings)
        }
    }
}

fn render_failed_login_value(info: &FailedLoginInfo, settings: &OutputSettings) -> String {
    match info {
        FailedLoginInfo::Unavailable => paint("unavailable", PaintKind::Yellow, settings),
        FailedLoginInfo::None => paint("none", PaintKind::Green, settings),
        FailedLoginInfo::Summary(summary) => {
            let value = if settings.compact {
                format_failed_login_compact(summary)
            } else {
                format_failed_login_full(summary)
            };
            paint_failed_login(value, summary.severity, settings)
        }
    }
}

fn format_last_login_full(record: &LastLoginRecord) -> String {
    let mut value = record.age.clone().unwrap_or_else(|| record.when.clone());
    if let Some(from) = &record.from {
        value.push_str(&format!(" from {}", from));
    }
    if let Some(via) = &record.via {
        value.push_str(&format!(" via {}", via));
    }
    if matches!(record.source_relation, SourceRelation::Different) {
        value.push_str(", different source");
    }
    value
}

fn format_last_login_verbose(record: &LastLoginRecord) -> String {
    let mut parts = vec![record.when.clone()];
    if let Some(age) = &record.age {
        parts.push(age.clone());
    }
    if let Some(from) = &record.from {
        parts.push(format!("from {}", from));
    }
    if let Some(via) = &record.via {
        parts.push(format!("via {}", via));
    }
    if !matches!(record.kind, super::types::LoginSessionKind::Unknown) {
        parts.push(record.kind.label().to_string());
    }
    if matches!(record.source_relation, SourceRelation::Different) {
        parts.push(record.source_relation.label().to_string());
    }
    parts.join(", ")
}

fn format_last_login_compact(record: &LastLoginRecord) -> String {
    let mut value = record.age.clone().unwrap_or_else(|| record.when.clone());
    if let Some(from) = &record.from {
        value.push_str(&format!(" from {}", from));
    }
    if let Some(via) = &record.via {
        value.push_str(&format!(" via {}", via));
    }
    if matches!(record.source_relation, SourceRelation::Different) {
        value.push_str(", different source");
    }
    value
}

fn format_failed_login_full(summary: &super::types::FailedLoginSummary) -> String {
    let mut parts = vec![summary.severity.label().to_string()];
    if summary.count_24h > 0 || summary.count_7d > 0 {
        parts.push(format!(
            "{}/24h, {}/7d",
            summary.count_24h, summary.count_7d
        ));
    } else {
        parts.push(format_failure_total(summary.total));
    }
    if let Some(top_source) = summary.top_sources.first() {
        if summary.top_sources.len() > 1 || top_source.count > 1 {
            parts.push(format!("top {} ({})", top_source.value, top_source.count));
        } else {
            parts.push(format!("from {}", top_source.value));
        }
    } else if let Some(last_from) = &summary.last_from {
        parts.push(format!("from {}", last_from));
    }
    if summary.current_source_seen {
        parts.push("current source seen".to_string());
    }
    parts.join(", ")
}

fn format_failed_login_compact(summary: &super::types::FailedLoginSummary) -> String {
    let mut parts = vec![summary.severity.label().to_string()];
    if summary.count_24h > 0 {
        parts.push(format!("{}/24h", summary.count_24h));
    } else if summary.count_7d > 0 {
        parts.push(format!("{}/7d", summary.count_7d));
    } else {
        parts.push(format_failure_total(summary.total));
    }
    if let Some(top_source) = summary.top_sources.first() {
        parts.push(format!("top {} ({})", top_source.value, top_source.count));
    } else if let Some(last_from) = &summary.last_from {
        parts.push(format!("from {}", last_from));
    }
    if summary.current_source_seen {
        parts.push("current source".to_string());
    }
    parts.join(", ")
}

fn format_failure_total(total: usize) -> String {
    if total == 1 {
        "1 older failure".to_string()
    } else {
        format!("{} older failures", total)
    }
}

fn paint_failed_login(
    value: String,
    severity: FailedLoginSeverity,
    settings: &OutputSettings,
) -> String {
    match severity {
        FailedLoginSeverity::Low => paint(value, PaintKind::Yellow, settings),
        FailedLoginSeverity::Warn => paint(value, PaintKind::Magenta, settings),
        FailedLoginSeverity::High => paint(value, PaintKind::Red, settings),
    }
}

fn terminal_width_hint() -> usize {
    env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(72, 160))
        .unwrap_or(100)
}

fn visible_width(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut width = 0usize;
    let mut idx = 0usize;

    while idx < bytes.len() {
        if bytes[idx] == 0x1b {
            idx += 1;
            if idx < bytes.len() && bytes[idx] == b'[' {
                idx += 1;
                while idx < bytes.len() {
                    let byte = bytes[idx];
                    idx += 1;
                    if byte.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
        }

        if let Some(ch) = text[idx..].chars().next() {
            width += 1;
            idx += ch.len_utf8();
        } else {
            break;
        }
    }

    width
}

fn compact_clauses(section: SectionKind, items: &[RenderedItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| compact_clause(section, item))
        .collect()
}

fn compact_clause(section: SectionKind, item: &RenderedItem) -> String {
    match item.label.as_str() {
        "Host name:" => item.value.clone(),
        "Main NIC:" => item.value.clone(),
        "User info:" => item.value.clone(),
        "Current time (TZ):" => item.value.clone(),
        "System uptime:" => format!("up {}", item.value),
        "Load average:" => format!("load {}", item.value),
        "Operating system:" => item.value.clone(),
        "Kernel version:" => format!("kernel {}", item.value),
        "Virtualization:" => format!("virt {}", item.value),
        "CPU:" => item.value.clone(),
        "Memory used/total:" => format!("mem {}", item.value),
        "Swap used/total:" => format!("swap {}", item.value),
        "Disk usage (root):" => format!("root {}", normalize_disk_compact_value(&item.value)),
        "Disk usage (nfs):" => format!("nfs {}", normalize_disk_compact_value(&item.value)),
        "Last login:" => format!("last {}", item.value),
        "Failed login:" => format!("failed {}", item.value),
        "Pending updates:" => format!("updates {}", item.value),
        label if label.starts_with("Service ") => {
            format!(
                "{} {}",
                label.trim_start_matches("Service ").trim_end_matches(':'),
                item.value
            )
        }
        _ => match section {
            SectionKind::Identity
            | SectionKind::Runtime
            | SectionKind::System
            | SectionKind::Storage
            | SectionKind::Operations => item.value.clone(),
        },
    }
}

fn normalize_disk_compact_value(value: &str) -> String {
    value
        .split_once("=>")
        .map(|(_, rest)| rest.trim().to_string())
        .unwrap_or_else(|| value.to_string())
}

fn format_buckets(buckets: &[FailedLoginBucket]) -> String {
    buckets
        .iter()
        .map(|bucket| format!("{} ({})", bucket.value, bucket.count))
        .collect::<Vec<_>>()
        .join(", ")
}
