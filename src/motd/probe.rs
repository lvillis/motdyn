use chrono::Local;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::Command;

#[cfg(unix)]
use libc::{endutxent, getutxent, setutxent, statvfs, USER_PROCESS};
#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::mem::MaybeUninit;

use super::types::{
    LinuxUtmpRecord, ModuleKind, RenderedItem, SnapshotDiagnostics, SystemSnapshot, UsageSummary,
};

pub(super) fn collect_snapshot() -> SystemSnapshot {
    let mut diagnostics = SnapshotDiagnostics::default();
    let ((os_name, os_version), os_source) = get_os_info();
    diagnostics.os_source = os_source.to_string();
    let now_str_with_tz = Local::now().format("%Y-%m-%d %H:%M:%S %:z").to_string();
    let uptime_str = parse_uptime().unwrap_or_else(|| "unknown".to_string());
    let kernel_version = read_first_line("/proc/sys/kernel/osrelease")
        .unwrap_or_else(|| "Unknown kernel".to_string());
    let host_name =
        read_first_line("/proc/sys/kernel/hostname").unwrap_or_else(|| "Unknown host".to_string());
    let (cpu_brand, cpu_count) = parse_cpuinfo();
    let (mem_total, mem_free, swap_total, swap_free) = parse_meminfo();
    let (current_user, from_ip) = get_current_user_and_ip();
    let (login_user_count, login_user_count_source) = get_logged_in_user_count();
    diagnostics.login_user_count_source = login_user_count_source.to_string();
    let (virt_info, virtualization_source) = detect_virtualization();
    diagnostics.virtualization_source = virtualization_source.to_string();
    diagnostics.network_source = "ip route/ip addr".to_string();

    let main_iface = match get_default_interface() {
        Ok(iface) => iface,
        Err(err) => {
            diagnostics.degrade(ModuleKind::Network, format!("network: {}", err));
            "unknown".to_string()
        }
    };
    let main_ip = if main_iface == "unknown" {
        "unknown".to_string()
    } else {
        match get_interface_ipv4(&main_iface) {
            Ok(ip) => ip,
            Err(err) => {
                diagnostics.degrade(ModuleKind::Network, format!("network: {}", err));
                "unknown".to_string()
            }
        }
    };

    if uptime_str == "unknown" {
        diagnostics.degrade(
            ModuleKind::Uptime,
            "uptime: failed to read or parse /proc/uptime".to_string(),
        );
    }
    if host_name == "Unknown host" {
        diagnostics.degrade(
            ModuleKind::Host,
            "host: failed to read /proc/sys/kernel/hostname".to_string(),
        );
    }
    if kernel_version == "Unknown kernel" {
        diagnostics.degrade(
            ModuleKind::Kernel,
            "kernel: failed to read /proc/sys/kernel/osrelease".to_string(),
        );
    }
    if os_source == "/proc/sys/kernel/ostype" {
        diagnostics.degrade(
            ModuleKind::Os,
            "os: no release metadata found; using kernel fallback".to_string(),
        );
    }
    if cpu_brand == "Unknown CPU" || cpu_count == 0 {
        diagnostics.degrade(
            ModuleKind::Cpu,
            "cpu: /proc/cpuinfo did not yield a stable brand/core count".to_string(),
        );
    }
    if mem_total == 0 {
        diagnostics.degrade(
            ModuleKind::Memory,
            "memory: /proc/meminfo missing or unreadable".to_string(),
        );
    }
    if from_ip == "unknown" {
        diagnostics.note("user: SSH_CONNECTION missing; source IP shown as unknown".to_string());
    }

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
        diagnostics,
    }
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

    loop {
        let mut record = MaybeUninit::<LinuxUtmpRecord>::zeroed();
        let buffer = unsafe {
            std::slice::from_raw_parts_mut(
                record.as_mut_ptr().cast::<u8>(),
                std::mem::size_of::<LinuxUtmpRecord>(),
            )
        };

        match file.read_exact(buffer) {
            Ok(()) => {
                let record = unsafe { record.assume_init() };
                if record.ut_type == USER_PROCESS && record.ut_user[0] != 0 {
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

#[cfg(target_os = "linux")]
fn get_logged_in_user_count() -> (usize, &'static str) {
    if let Some(count) = count_logged_in_users_from_linux_utmp() {
        (count, "linux utmp")
    } else {
        (get_logged_in_user_count_via_libc(), "libc utmpx fallback")
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn get_logged_in_user_count() -> (usize, &'static str) {
    (get_logged_in_user_count_via_libc(), "libc utmpx")
}

#[cfg(not(unix))]
fn get_logged_in_user_count() -> (usize, &'static str) {
    (0, "unsupported")
}

#[cfg(unix)]
fn get_logged_in_user_count_via_libc() -> usize {
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

#[cfg(target_os = "linux")]
fn count_logged_in_users_from_linux_utmp() -> Option<usize> {
    ["/run/utmp", "/var/run/utmp"]
        .into_iter()
        .find_map(|path| count_logged_in_users_from_linux_utmp_file(Path::new(path)))
}

fn detect_virtualization() -> (Option<String>, &'static str) {
    if Path::new("/.dockerenv").exists() {
        return (Some("Docker".to_string()), "/.dockerenv");
    }

    if let Ok(content) = fs::read_to_string("/proc/1/cgroup") {
        if let Some(value) = detect_virtualization_from_cgroup(&content) {
            return (Some(value), "/proc/1/cgroup");
        }
    }

    if let Ok(output) = Command::new("systemd-detect-virt").output() {
        if output.status.success() {
            let virt_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if virt_str != "none" && !virt_str.is_empty() {
                return (Some(virt_str), "systemd-detect-virt");
            }
        }
    }

    (None, "not detected")
}

#[cfg(unix)]
fn collect_disk_usage_items() -> Vec<RenderedItem> {
    let file = match File::open("/proc/mounts") {
        Ok(file) => file,
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
            mount_path, used_str, total_str, ratio
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
    let stat = unsafe { stat.assume_init() };

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

fn get_default_interface() -> Result<String, String> {
    let output = Command::new("ip")
        .arg("route")
        .arg("show")
        .arg("default")
        .output()
        .map_err(|err| format!("failed to run 'ip route show default': {}", err))?;

    if !output.status.success() {
        return Err(format!(
            "'ip route show default' exited with status {}",
            output.status
        ));
    }

    parse_default_interface_output(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
        "no usable default route found in 'ip route show default' output".to_string()
    })
}

fn get_interface_ipv4(iface: &str) -> Result<String, String> {
    let output = Command::new("ip")
        .args(["-o", "-4", "addr", "show", "dev", iface])
        .output()
        .map_err(|err| format!("failed to run 'ip -o -4 addr show dev {}': {}", iface, err))?;

    if !output.status.success() {
        return Err(format!(
            "'ip -o -4 addr show dev {}' exited with status {}",
            iface, output.status
        ));
    }

    parse_interface_ipv4_output(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| format!("no IPv4 address found for interface '{}'", iface))
}
