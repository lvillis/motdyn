use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, Local, LocalResult, NaiveDateTime, TimeZone,
};
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(unix)]
use rustix::fs::statvfs;

use crate::config::MotdConfig;

use super::types::{
    FailedLoginBucket, FailedLoginInfo, FailedLoginSeverity, LastLoginInfo, LastLoginRecord,
    LoginSessionKind, ModuleKind, NetworkProbeError, ProbeIssue, RenderedItem, SnapshotDiagnostics,
    SourceRelation, SystemSnapshot, UsageSummary,
};

#[cfg(target_os = "linux")]
use super::types::{
    LINUX_USER_PROCESS, LINUX_UTMP_RECORD_SIZE, LINUX_UTMP_TYPE_OFFSET, LINUX_UTMP_USER_LEN,
    LINUX_UTMP_USER_OFFSET,
};

const CORE_PROBE_TIMEOUT_MS: u64 = 120;
const OPTIONAL_PROBE_TIMEOUT_MS: u64 = 150;
const UPDATES_PROBE_TIMEOUT_MS: u64 = 250;
const COMMAND_POLL_INTERVAL_MS: u64 = 10;

#[derive(Debug)]
pub(super) struct TimedCommandOutput {
    pub(super) status: ExitStatus,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedLastLoginRecord {
    pub(super) when: String,
    pub(super) from: Option<String>,
    pub(super) via: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FailedLoginEvent {
    pub(super) when: Option<DateTime<Local>>,
    pub(super) when_raw: String,
    pub(super) from: Option<String>,
    pub(super) via: Option<String>,
}

pub(super) fn collect_snapshot(
    requested_modules: &[ModuleKind],
    cfg: &MotdConfig,
) -> SystemSnapshot {
    let mut diagnostics = SnapshotDiagnostics::default();
    let ((os_name, os_version), os_source) = get_os_info();
    diagnostics.os_source = os_source.to_string();
    let now = Local::now();
    let now_str_with_tz = now.format("%Y-%m-%d %H:%M:%S %:z").to_string();
    let uptime_str = parse_uptime().unwrap_or_else(|| "unknown".to_string());
    let load_average = if module_enabled(requested_modules, ModuleKind::Load) {
        diagnostics.load_source = "/proc/loadavg".to_string();
        match parse_load_average() {
            Some(value) => value,
            None => {
                diagnostics.degrade(ModuleKind::Load, ProbeIssue::LoadAverageReadFailed);
                "unknown".to_string()
            }
        }
    } else {
        String::new()
    };
    let kernel_version = read_first_line("/proc/sys/kernel/osrelease")
        .unwrap_or_else(|| "Unknown kernel".to_string());
    let host_name =
        read_first_line("/proc/sys/kernel/hostname").unwrap_or_else(|| "Unknown host".to_string());
    let (cpu_brand, cpu_count) = parse_cpuinfo();
    let (mem_total, mem_free, swap_total, swap_free) = parse_meminfo();
    let (current_user, from_ip) = get_current_user_and_ip();
    let (login_user_count, login_user_count_source) = get_logged_in_user_count();
    diagnostics.login_user_count_source = login_user_count_source.to_string();
    let (virt_info, virtualization_source, virtualization_issue) = detect_virtualization();
    diagnostics.virtualization_source = virtualization_source;
    diagnostics.network_source = "ip route/ip addr".to_string();
    let last_login = if module_enabled(requested_modules, ModuleKind::LastLogin) {
        diagnostics.last_login_source = "lastlog".to_string();
        match probe_last_login(&current_user, &from_ip, now) {
            Ok(value) => value,
            Err(err) => {
                diagnostics.degrade(ModuleKind::LastLogin, ProbeIssue::LastLoginProbeFailed(err));
                LastLoginInfo::Unavailable
            }
        }
    } else {
        LastLoginInfo::Unavailable
    };
    let failed_login = if module_enabled(requested_modules, ModuleKind::FailedLogin) {
        diagnostics.failed_login_source = "lastb".to_string();
        match probe_failed_login(&current_user, &from_ip, now) {
            Ok(value) => value,
            Err(err) => {
                diagnostics.degrade(
                    ModuleKind::FailedLogin,
                    ProbeIssue::FailedLoginProbeFailed(err),
                );
                FailedLoginInfo::Unavailable
            }
        }
    } else {
        FailedLoginInfo::Unavailable
    };
    let service_items = if module_enabled(requested_modules, ModuleKind::Services) {
        diagnostics.service_status_source = "systemctl is-active".to_string();
        match probe_service_statuses(cfg.service_status.services.as_deref().unwrap_or(&[])) {
            Ok(items) => items,
            Err(err) => {
                diagnostics.degrade(
                    ModuleKind::Services,
                    ProbeIssue::ServiceStatusProbeFailed(err),
                );
                vec![RenderedItem {
                    label: "Service status:".to_string(),
                    value: "unavailable".to_string(),
                }]
            }
        }
    } else {
        Vec::new()
    };
    let update_summary = if module_enabled(requested_modules, ModuleKind::Updates) {
        match probe_package_updates() {
            Ok((summary, source)) => {
                diagnostics.updates_source = source;
                summary
            }
            Err(err) => {
                diagnostics.degrade(ModuleKind::Updates, ProbeIssue::UpdateProbeFailed(err));
                "unavailable".to_string()
            }
        }
    } else {
        String::new()
    };

    let main_iface = match get_default_interface() {
        Ok(iface) => iface,
        Err(err) => {
            diagnostics.degrade(ModuleKind::Network, ProbeIssue::Network(err));
            "unknown".to_string()
        }
    };
    let main_ip = if main_iface == "unknown" {
        "unknown".to_string()
    } else {
        match get_interface_ipv4(&main_iface) {
            Ok(ip) => ip,
            Err(err) => {
                diagnostics.degrade(ModuleKind::Network, ProbeIssue::Network(err));
                "unknown".to_string()
            }
        }
    };

    if uptime_str == "unknown" {
        diagnostics.degrade(ModuleKind::Uptime, ProbeIssue::UptimeReadFailed);
    }
    if host_name == "Unknown host" {
        diagnostics.degrade(ModuleKind::Host, ProbeIssue::HostReadFailed);
    }
    if kernel_version == "Unknown kernel" {
        diagnostics.degrade(ModuleKind::Kernel, ProbeIssue::KernelReadFailed);
    }
    if os_source == "/proc/sys/kernel/ostype" {
        diagnostics.degrade(ModuleKind::Os, ProbeIssue::OsMetadataMissing);
    }
    if let Some(issue) = virtualization_issue {
        diagnostics.degrade(ModuleKind::Virtualization, issue);
    }
    if cpu_brand == "Unknown CPU" || cpu_count == 0 {
        diagnostics.degrade(ModuleKind::Cpu, ProbeIssue::CpuInfoUnstable);
    }
    if mem_total == 0 {
        diagnostics.degrade(ModuleKind::Memory, ProbeIssue::MemoryInfoMissing);
    }
    if from_ip == "unknown" {
        diagnostics.note(ProbeIssue::SshConnectionMissing);
    }

    let (root_disk, disk_items) = collect_disk_usage_items();

    SystemSnapshot {
        host_name,
        main_iface,
        main_ip,
        current_user,
        from_ip,
        login_user_count,
        now_str_with_tz,
        uptime_str,
        load_average,
        os_name,
        os_version,
        kernel_version,
        virt_info,
        cpu_brand,
        cpu_count,
        memory: usage_summary(mem_total, mem_free),
        swap: usage_summary(swap_total, swap_free),
        root_disk,
        disk_items,
        last_login,
        failed_login,
        service_items,
        update_summary,
        diagnostics,
    }
}

pub(super) fn parse_loadavg_content(content: &str) -> Option<String> {
    let mut parts = content.split_whitespace();
    let one = parts.next()?;
    let five = parts.next()?;
    let fifteen = parts.next()?;
    Some(format!("{} {} {}", one, five, fifteen))
}

pub(super) fn parse_lastlog_output(output: &str) -> Option<Option<ParsedLastLoginRecord>> {
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next()?;
    let data = lines.next()?;

    if data.contains("**Never logged in**") {
        return Some(None);
    }

    let port_start = header.find("Port")?;
    let from_start = header.find("From")?;
    let latest_start = header.find("Latest")?;
    if !(port_start < from_start && from_start < latest_start) {
        return None;
    }

    let via = normalize_detail_field(&slice_column(data, port_start, from_start));
    let from = normalize_detail_field(&slice_column(data, from_start, latest_start));
    let latest = data.get(latest_start..)?.trim();
    if latest.is_empty() {
        return None;
    }

    Some(Some(ParsedLastLoginRecord {
        when: latest.to_string(),
        from,
        via,
    }))
}

pub(super) fn parse_lastb_output(
    output: &str,
    now: DateTime<Local>,
) -> Option<Vec<FailedLoginEvent>> {
    let entries = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("btmp begins"))
        .collect::<Vec<_>>();

    if entries.is_empty() {
        return Some(Vec::new());
    }

    let mut parsed = Vec::new();
    for line in entries {
        let mut parts = line.split_whitespace();
        let _user = parts.next()?;
        let via = normalize_detail_field(parts.next().unwrap_or_default());
        let from = normalize_detail_field(parts.next().unwrap_or_default());
        let when_tokens = parts.take(4).collect::<Vec<_>>();
        if when_tokens.len() < 4 {
            return None;
        }

        let when_raw = when_tokens.join(" ");
        parsed.push(FailedLoginEvent {
            when: parse_lastb_timestamp(&when_raw, now),
            when_raw,
            from,
            via,
        });
    }

    Some(parsed)
}

pub(super) fn parse_apt_upgradable_output(output: &str) -> usize {
    output
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("[upgradable from:"))
        .count()
}

pub(super) fn parse_dnf_check_update_output(output: &str) -> usize {
    output
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("Last metadata expiration check:")
                && line.split_whitespace().count() >= 3
        })
        .count()
}

pub(super) fn format_uptime(mut secs: u64) -> String {
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

pub(super) fn parse_os_release_content(content: &str) -> Option<(String, String)> {
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
        (Some(name), Some(version)) => Some((name, version)),
        _ => None,
    }
}

pub(super) fn parse_redhat_release_content(content: &str) -> Option<(String, String)> {
    let line = content.trim();
    let needle = " release ";
    let pos = line.find(needle)?;
    let os_name = &line[..pos];
    let ver_str = &line[pos + needle.len()..];
    Some((os_name.to_string(), ver_str.to_string()))
}

pub(super) fn parse_meminfo_content(content: &str) -> (u64, u64, u64, u64) {
    let mut mem_total = 0;
    let mut mem_available = 0;
    let mut mem_free = 0;
    let mut swap_total = 0;
    let mut swap_free = 0;

    for line in content.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        match parts[0] {
            "MemTotal:" => mem_total = parts[1].parse().unwrap_or(0),
            "MemAvailable:" => mem_available = parts[1].parse().unwrap_or(0),
            "MemFree:" => mem_free = parts[1].parse().unwrap_or(0),
            "SwapTotal:" => swap_total = parts[1].parse().unwrap_or(0),
            "SwapFree:" => swap_free = parts[1].parse().unwrap_or(0),
            _ => {}
        }
    }

    (
        mem_total,
        mem_available.max(mem_free),
        swap_total,
        swap_free,
    )
}

pub(super) fn parse_cpuinfo_content(content: &str) -> (String, usize) {
    let mut brand = "Unknown CPU".to_string();
    let mut core_count = 0;

    let mut cpu_implementer = String::new();
    let mut cpu_part = String::new();

    for line in content.lines() {
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
        } else if let Some(part_str) = line.strip_prefix("CPU part")
            && let Some(val) = part_str.split(':').nth(1)
        {
            cpu_part = val.trim().to_lowercase();
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

pub(super) fn detect_virtualization_from_cgroup(content: &str) -> Option<String> {
    if content.contains("docker") {
        Some("Docker".to_string())
    } else if content.contains("lxc") {
        Some("LXC".to_string())
    } else {
        None
    }
}

pub(super) fn parse_uptime_content(line: &str) -> Option<String> {
    let parts: Vec<_> = line.split_whitespace().collect();
    let total_seconds = parts.first()?.parse::<f64>().ok()? as u64;

    Some(format_uptime(total_seconds))
}

pub(super) fn parse_ssh_connection_ip(ssh_connection: &str) -> String {
    ssh_connection
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

pub(super) fn parse_default_interface_output(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() > 4 && parts[0] == "default" && parts[1] == "via" && parts[3] == "dev" {
            return Some(parts[4].to_string());
        }
    }
    None
}

pub(super) fn parse_interface_ipv4_output(stdout: &str) -> Option<String> {
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

pub(super) fn to_gb_and_ratio(total_kb: u64, free_kb: u64) -> (f64, f64, f64) {
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

#[cfg(target_os = "linux")]
pub(super) fn count_logged_in_users_from_linux_utmp_file(path: &Path) -> Option<usize> {
    let mut file = File::open(path).ok()?;
    let mut count = 0;
    let mut record = [0_u8; LINUX_UTMP_RECORD_SIZE];

    loop {
        match file.read_exact(&mut record) {
            Ok(()) => {
                let ut_type = i16::from_ne_bytes([
                    record[LINUX_UTMP_TYPE_OFFSET],
                    record[LINUX_UTMP_TYPE_OFFSET + 1],
                ]);
                let user =
                    &record[LINUX_UTMP_USER_OFFSET..LINUX_UTMP_USER_OFFSET + LINUX_UTMP_USER_LEN];
                if ut_type == LINUX_USER_PROCESS && user[0] != 0 {
                    count += 1;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(_) => return None,
        }
    }

    Some(count)
}

fn usage_summary(total_kb: u64, free_kb: u64) -> UsageSummary {
    let (used_gb, total_gb, ratio) = to_gb_and_ratio(total_kb, free_kb);
    UsageSummary {
        used_gb,
        total_gb,
        ratio,
    }
}

fn parse_uptime() -> Option<String> {
    let line = fs::read_to_string("/proc/uptime").ok()?;
    parse_uptime_content(&line)
}

fn parse_load_average() -> Option<String> {
    let content = fs::read_to_string("/proc/loadavg").ok()?;
    parse_loadavg_content(&content)
}

fn get_os_info() -> ((String, String), &'static str) {
    if let Some(result) = parse_redhat_release() {
        return (result, "/etc/redhat-release");
    }
    if let Some(result) = parse_os_release() {
        return (result, "/etc/os-release");
    }
    let fallback_os = read_first_line("/proc/sys/kernel/ostype").unwrap_or("Linux".to_string());
    (
        ("Linux".to_string(), fallback_os),
        "/proc/sys/kernel/ostype",
    )
}

fn parse_redhat_release() -> Option<(String, String)> {
    let content = fs::read_to_string("/etc/redhat-release").ok()?;
    parse_redhat_release_content(&content)
}

fn parse_os_release() -> Option<(String, String)> {
    let content = fs::read_to_string("/etc/os-release").ok()?;
    parse_os_release_content(&content)
}

fn parse_meminfo() -> (u64, u64, u64, u64) {
    let content = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    parse_meminfo_content(&content)
}

fn parse_cpuinfo() -> (String, usize) {
    let content = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    parse_cpuinfo_content(&content)
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
    let from_ip = parse_ssh_connection_ip(&env::var("SSH_CONNECTION").unwrap_or_default());
    (user, from_ip)
}

fn module_enabled(requested_modules: &[ModuleKind], module: ModuleKind) -> bool {
    requested_modules.contains(&module)
}

fn probe_last_login(
    user: &str,
    current_source_ip: &str,
    now: DateTime<Local>,
) -> Result<LastLoginInfo, String> {
    if user == "unknown" {
        return Err("current user is unknown".to_string());
    }

    let output = run_command_with_timeout(
        "lastlog",
        &["-u", user],
        &[("LC_ALL", "C")],
        OPTIONAL_PROBE_TIMEOUT_MS,
    )?;

    if !output.status.success() {
        let stderr = output.stderr.trim().to_string();
        let detail = if stderr.is_empty() {
            output.status.to_string()
        } else {
            stderr
        };
        return Err(format!("'lastlog -u {}' exited with {}", user, detail));
    }

    match parse_lastlog_output(&output.stdout)
        .ok_or_else(|| "unexpected 'lastlog' output".to_string())?
    {
        Some(record) => Ok(LastLoginInfo::Recorded(enrich_last_login_record(
            record,
            current_source_ip,
            now,
        ))),
        None => Ok(LastLoginInfo::NeverRecorded),
    }
}

fn probe_failed_login(
    user: &str,
    current_source_ip: &str,
    now: DateTime<Local>,
) -> Result<FailedLoginInfo, String> {
    if user == "unknown" {
        return Err("current user is unknown".to_string());
    }

    let output = run_command_with_timeout(
        "lastb",
        &["-w", "-n", "20", user],
        &[("LC_ALL", "C")],
        OPTIONAL_PROBE_TIMEOUT_MS,
    )?;

    if !output.status.success() {
        let stderr = output.stderr.trim().to_string();
        let detail = if stderr.is_empty() {
            output.status.to_string()
        } else {
            stderr
        };
        return Err(format!("'lastb -w -n 20 {}' exited with {}", user, detail));
    }

    let events = parse_lastb_output(&output.stdout, now)
        .ok_or_else(|| "unexpected 'lastb' output".to_string())?;
    Ok(summarize_failed_login_events(
        &events,
        current_source_ip,
        now,
    ))
}

fn enrich_last_login_record(
    record: ParsedLastLoginRecord,
    current_source_ip: &str,
    now: DateTime<Local>,
) -> LastLoginRecord {
    let kind = classify_login_kind(record.via.as_deref(), record.from.as_deref());
    let age =
        parse_lastlog_timestamp(&record.when).map(|timestamp| format_relative_age(now, timestamp));
    let source_relation = compare_sources(record.from.as_deref(), current_source_ip);

    LastLoginRecord {
        when: record.when,
        from: record.from,
        via: record.via,
        kind,
        age,
        source_relation,
    }
}

pub(super) fn summarize_failed_login_events(
    events: &[FailedLoginEvent],
    current_source_ip: &str,
    now: DateTime<Local>,
) -> FailedLoginInfo {
    if events.is_empty() {
        return FailedLoginInfo::None;
    }

    let count_24h = events
        .iter()
        .filter_map(|event| event.when)
        .filter(|when| is_within_window(now, *when, ChronoDuration::hours(24)))
        .count();
    let count_7d = events
        .iter()
        .filter_map(|event| event.when)
        .filter(|when| is_within_window(now, *when, ChronoDuration::days(7)))
        .count();
    let top_sources = top_buckets(events.iter().filter_map(|event| event.from.as_deref()), 3);
    let top_vias = top_buckets(events.iter().filter_map(|event| event.via.as_deref()), 3);
    let current_source_seen = current_source_ip != "unknown"
        && events
            .iter()
            .any(|event| event.from.as_deref() == Some(current_source_ip));
    let unique_sources = events
        .iter()
        .filter_map(|event| event.from.as_ref())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let severity =
        classify_failed_login_severity(count_24h, count_7d, unique_sources, current_source_seen);
    let last_event = &events[0];

    FailedLoginInfo::Summary(super::types::FailedLoginSummary {
        total: events.len(),
        count_24h,
        count_7d,
        last_when: Some(last_event.when_raw.clone()),
        last_from: last_event.from.clone(),
        last_via: last_event.via.clone(),
        top_sources,
        top_vias,
        unique_sources,
        severity,
        current_source_seen,
    })
}

fn classify_failed_login_severity(
    count_24h: usize,
    count_7d: usize,
    unique_sources: usize,
    current_source_seen: bool,
) -> FailedLoginSeverity {
    if current_source_seen || count_24h >= 10 || (count_24h >= 5 && unique_sources >= 3) {
        FailedLoginSeverity::High
    } else if count_24h >= 3 || count_7d >= 10 {
        FailedLoginSeverity::Warn
    } else {
        FailedLoginSeverity::Low
    }
}

fn top_buckets<'a>(values: impl Iterator<Item = &'a str>, limit: usize) -> Vec<FailedLoginBucket> {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for value in values {
        *counts.entry(value.to_string()).or_default() += 1;
    }

    let mut buckets = counts
        .into_iter()
        .map(|(value, count)| FailedLoginBucket { value, count })
        .collect::<Vec<_>>();
    buckets.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.value.cmp(&right.value))
    });
    buckets.truncate(limit);
    buckets
}

fn normalize_detail_field(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || matches!(trimmed, "**" | "***" | "unknown") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn classify_login_kind(via: Option<&str>, from: Option<&str>) -> LoginSessionKind {
    let via = via.unwrap_or_default().to_ascii_lowercase();
    if via.starts_with("pts/") || via.contains("ssh") {
        return LoginSessionKind::Ssh;
    }
    if via.starts_with("tty") || via.contains("console") {
        return LoginSessionKind::Console;
    }

    let from = from.unwrap_or_default().to_ascii_lowercase();
    if !from.is_empty() && from != "localhost" && from != ":0" {
        LoginSessionKind::Ssh
    } else {
        LoginSessionKind::Unknown
    }
}

fn compare_sources(previous: Option<&str>, current_source_ip: &str) -> SourceRelation {
    if current_source_ip == "unknown" {
        return SourceRelation::Unknown;
    }

    match previous {
        Some(previous) if previous == current_source_ip => SourceRelation::Same,
        Some(_) => SourceRelation::Different,
        None => SourceRelation::Unknown,
    }
}

fn parse_lastlog_timestamp(value: &str) -> Option<DateTime<Local>> {
    if let Ok(timestamp) = DateTime::parse_from_str(value, "%a %b %e %H:%M:%S %z %Y") {
        return Some(timestamp.with_timezone(&Local));
    }

    let fallback = strip_weekday(value)?;
    DateTime::parse_from_str(&fallback, "%b %e %H:%M:%S %z %Y")
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Local))
}

fn parse_lastb_timestamp(value: &str, now: DateTime<Local>) -> Option<DateTime<Local>> {
    let year = now.year();
    let naive = parse_lastb_naive(value, year)?;
    let mut timestamp = resolve_local_datetime(naive)?;
    if timestamp > now + ChronoDuration::hours(24) {
        let previous_year = parse_lastb_naive(value, year - 1)?;
        timestamp = resolve_local_datetime(previous_year)?;
    }
    Some(timestamp)
}

fn parse_lastb_naive(value: &str, year: i32) -> Option<NaiveDateTime> {
    let with_weekday = format!("{} {}", value, year);
    if let Ok(parsed) = NaiveDateTime::parse_from_str(&with_weekday, "%a %b %e %H:%M %Y") {
        return Some(parsed);
    }

    let without_weekday = format!("{} {}", strip_weekday(value)?, year);
    NaiveDateTime::parse_from_str(&without_weekday, "%b %e %H:%M %Y").ok()
}

fn strip_weekday(value: &str) -> Option<String> {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() >= 4 {
        Some(parts[1..].join(" "))
    } else {
        None
    }
}

fn resolve_local_datetime(value: NaiveDateTime) -> Option<DateTime<Local>> {
    match Local.from_local_datetime(&value) {
        LocalResult::Single(timestamp) => Some(timestamp),
        LocalResult::Ambiguous(earliest, _) => Some(earliest),
        LocalResult::None => None,
    }
}

fn format_relative_age(now: DateTime<Local>, then: DateTime<Local>) -> String {
    let delta = now.signed_duration_since(then);
    if delta < ChronoDuration::minutes(1) {
        "<1m ago".to_string()
    } else if delta < ChronoDuration::hours(1) {
        format!("{}m ago", delta.num_minutes())
    } else if delta < ChronoDuration::days(1) {
        format!("{}h ago", delta.num_hours())
    } else if delta < ChronoDuration::days(30) {
        format!("{}d ago", delta.num_days())
    } else if delta < ChronoDuration::days(365) {
        format!("{}mo ago", delta.num_days() / 30)
    } else {
        format!("{}y ago", delta.num_days() / 365)
    }
}

fn is_within_window(now: DateTime<Local>, then: DateTime<Local>, window: ChronoDuration) -> bool {
    let delta = now.signed_duration_since(then);
    delta >= ChronoDuration::zero() && delta <= window
}

fn probe_service_statuses(services: &[String]) -> Result<Vec<RenderedItem>, String> {
    if services.is_empty() {
        return Ok(vec![RenderedItem {
            label: "Service status:".to_string(),
            value: "no services configured".to_string(),
        }]);
    }

    let mut items = Vec::with_capacity(services.len());
    for service in services {
        let output = run_command_with_timeout(
            "systemctl",
            &["is-active", service],
            &[("LC_ALL", "C")],
            OPTIONAL_PROBE_TIMEOUT_MS,
        )?;

        let status = output.stdout.trim().to_string();
        let fallback = output.stderr.trim().to_string();
        let value = if status.is_empty() {
            if fallback.is_empty() {
                output.status.to_string()
            } else {
                fallback
            }
        } else {
            status
        };

        items.push(RenderedItem {
            label: format!("Service {}:", service),
            value,
        });
    }

    Ok(items)
}

fn probe_package_updates() -> Result<(String, String), String> {
    if command_exists("apt") {
        let output = run_command_with_timeout(
            "apt",
            &["list", "--upgradable"],
            &[("LC_ALL", "C")],
            UPDATES_PROBE_TIMEOUT_MS,
        )?;

        if !output.status.success() {
            let stderr = output.stderr.trim().to_string();
            let detail = if stderr.is_empty() {
                output.status.to_string()
            } else {
                stderr
            };
            return Err(format!("'apt list --upgradable' exited with {}", detail));
        }

        let count = parse_apt_upgradable_output(&output.stdout);
        let summary = if count == 0 {
            "none (apt)".to_string()
        } else {
            format!("{} package(s) via apt", count)
        };
        return Ok((summary, "apt list --upgradable".to_string()));
    }

    if command_exists("dnf") {
        let output = run_command_with_timeout(
            "dnf",
            &["-q", "check-update", "--cacheonly"],
            &[("LC_ALL", "C")],
            UPDATES_PROBE_TIMEOUT_MS,
        )?;

        match output.status.code() {
            Some(0) | Some(100) => {
                let count = parse_dnf_check_update_output(&output.stdout);
                let summary = if count == 0 {
                    "none (dnf)".to_string()
                } else {
                    format!("{} package(s) via dnf", count)
                };
                return Ok((summary, "dnf check-update --cacheonly".to_string()));
            }
            _ => {
                let stderr = output.stderr.trim().to_string();
                let detail = if stderr.is_empty() {
                    output.status.to_string()
                } else {
                    stderr
                };
                return Err(format!(
                    "'dnf -q check-update --cacheonly' exited with {}",
                    detail
                ));
            }
        }
    }

    Err("no supported package manager found".to_string())
}

#[cfg(target_os = "linux")]
fn get_logged_in_user_count() -> (usize, &'static str) {
    if let Some(count) = count_logged_in_users_from_linux_utmp() {
        (count, "linux utmp")
    } else {
        (0, "linux utmp unavailable")
    }
}

#[cfg(not(target_os = "linux"))]
fn get_logged_in_user_count() -> (usize, &'static str) {
    (0, "unsupported")
}

#[cfg(target_os = "linux")]
fn count_logged_in_users_from_linux_utmp() -> Option<usize> {
    ["/run/utmp", "/var/run/utmp"]
        .into_iter()
        .find_map(|path| count_logged_in_users_from_linux_utmp_file(Path::new(path)))
}

pub(super) fn run_command_with_timeout(
    program: &str,
    args: &[&str],
    envs: &[(&str, &str)],
    timeout_ms: u64,
) -> Result<TimedCommandOutput, String> {
    let command_line = format_command(program, args);
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }

    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to run '{}': {}", command_line, err))?;
    let stdout_handle = child.stdout.take().map(spawn_reader_thread);
    let stderr_handle = child.stderr.take().map(spawn_reader_thread);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(TimedCommandOutput {
                    status,
                    stdout: collect_reader_output(stdout_handle),
                    stderr: collect_reader_output(stderr_handle),
                });
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(COMMAND_POLL_INTERVAL_MS));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = collect_reader_output(stdout_handle);
                let _ = collect_reader_output(stderr_handle);
                return Err(format!(
                    "'{}' timed out after {}ms",
                    command_line, timeout_ms
                ));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = collect_reader_output(stdout_handle);
                let _ = collect_reader_output(stderr_handle);
                return Err(format!(
                    "failed while waiting for '{}': {}",
                    command_line, err
                ));
            }
        }
    }
}

fn slice_column(line: &str, start: usize, end: usize) -> String {
    line.get(start..line.len().min(end))
        .unwrap_or("")
        .trim()
        .to_string()
}

fn format_command(program: &str, args: &[&str]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{} {}", program, args.join(" "))
    }
}

fn spawn_reader_thread<R>(mut reader: R) -> JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        buf
    })
}

fn collect_reader_output(handle: Option<JoinHandle<Vec<u8>>>) -> String {
    let bytes = handle
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).to_string()
}

fn command_exists(command: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|dir| dir.join(command).is_file())
}

fn detect_virtualization() -> (Option<String>, String, Option<ProbeIssue>) {
    if Path::new("/.dockerenv").exists() {
        return (Some("Docker".to_string()), "/.dockerenv".to_string(), None);
    }

    if let Ok(content) = fs::read_to_string("/proc/1/cgroup")
        && let Some(value) = detect_virtualization_from_cgroup(&content)
    {
        return (Some(value), "/proc/1/cgroup".to_string(), None);
    }

    match run_command_with_timeout("systemd-detect-virt", &[], &[], CORE_PROBE_TIMEOUT_MS) {
        Ok(output) if output.status.success() => {
            let virt_str = output.stdout.trim().to_string();
            if virt_str != "none" && !virt_str.is_empty() {
                return (Some(virt_str), "systemd-detect-virt".to_string(), None);
            }
        }
        Ok(_) => {}
        Err(err) => {
            return (
                None,
                "systemd-detect-virt".to_string(),
                Some(ProbeIssue::VirtualizationProbeFailed(err)),
            );
        }
    }

    (None, "not detected".to_string(), None)
}

#[cfg(unix)]
fn collect_disk_usage_items() -> (Option<UsageSummary>, Vec<RenderedItem>) {
    let file = match File::open("/proc/mounts") {
        Ok(file) => file,
        Err(_) => return (None, Vec::new()),
    };

    let mut root_disk = None;
    let mut items = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        let mount_path = fields[1];
        let fstype = fields[2];

        if mount_path == "/" {
            if let Some((summary, item)) = disk_usage_item(mount_path, "Disk usage (root):") {
                root_disk = Some(summary);
                items.push(item);
            }
        } else if matches!(fstype, "nfs" | "nfs4")
            && let Some((_, item)) = disk_usage_item(mount_path, "Disk usage (nfs):")
        {
            items.push(item);
        }
    }

    (root_disk, items)
}

#[cfg(not(unix))]
fn collect_disk_usage_items() -> (Option<UsageSummary>, Vec<RenderedItem>) {
    (None, Vec::new())
}

#[cfg(unix)]
fn disk_usage_item(mount_path: &str, label: &str) -> Option<(UsageSummary, RenderedItem)> {
    let (total_bytes, used_bytes) = get_mount_usage(mount_path)?;
    let (used_str, total_str, ratio) = human_readable_usage(used_bytes, total_bytes);
    let summary = UsageSummary {
        used_gb: bytes_to_gb(used_bytes),
        total_gb: bytes_to_gb(total_bytes),
        ratio,
    };
    Some((
        summary,
        RenderedItem {
            label: label.to_string(),
            value: format!(
                "{} => {}/{} ({:.2}%)",
                mount_path, used_str, total_str, ratio
            ),
        },
    ))
}

#[cfg(unix)]
fn get_mount_usage(mountpoint: &str) -> Option<(u64, u64)> {
    let stat = statvfs(mountpoint).ok()?;

    let block_size = stat.f_frsize;
    let blocks_used = stat.f_blocks.saturating_sub(stat.f_bfree);
    let total_bytes = block_size.saturating_mul(stat.f_blocks);
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
fn bytes_to_gb(value: u64) -> f64 {
    value as f64 / 1024.0 / 1024.0 / 1024.0
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

fn kb_to_gb(kb: u64) -> f64 {
    kb as f64 / 1024.0 / 1024.0
}

fn get_default_interface() -> Result<String, NetworkProbeError> {
    let output = run_command_with_timeout(
        "ip",
        &["route", "show", "default"],
        &[],
        CORE_PROBE_TIMEOUT_MS,
    )
    .map_err(NetworkProbeError::DefaultRouteCommand)?;

    if !output.status.success() {
        return Err(NetworkProbeError::DefaultRouteStatus(
            output.status.to_string(),
        ));
    }

    parse_default_interface_output(&output.stdout).ok_or(NetworkProbeError::DefaultRouteParse)
}

fn get_interface_ipv4(iface: &str) -> Result<String, NetworkProbeError> {
    let output = run_command_with_timeout(
        "ip",
        &["-o", "-4", "addr", "show", "dev", iface],
        &[],
        CORE_PROBE_TIMEOUT_MS,
    )
    .map_err(|message| NetworkProbeError::InterfaceIpv4Command {
        iface: iface.to_string(),
        message,
    })?;

    if !output.status.success() {
        return Err(NetworkProbeError::InterfaceIpv4Status {
            iface: iface.to_string(),
            status: output.status.to_string(),
        });
    }

    parse_interface_ipv4_output(&output.stdout).ok_or_else(|| {
        NetworkProbeError::InterfaceIpv4Parse {
            iface: iface.to_string(),
        }
    })
}
