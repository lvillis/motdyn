use chrono::Local;
use colored::Colorize;
use reqwest::blocking::Client;
use reqwest::Url;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::config::MotdConfig;

#[cfg(unix)]
use libc::statvfs;
#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::mem::MaybeUninit;

pub fn render(verbose: bool, cfg: &MotdConfig) {
    let welcome_str = cfg
        .welcome
        .as_deref()
        .map(fetch_welcome_text)
        .unwrap_or_else(|| "Welcome!".to_string());

    println!();
    println!("{}", welcome_str);
    println!();

    let (os_name, os_version) = get_os_info();
    let now = Local::now();
    let now_str_with_tz = now.format("%Y-%m-%d %H:%M:%S %:z").to_string();

    let uptime_str = parse_uptime().unwrap_or_else(|| "unknown".to_string());

    let kernel_version = read_first_line("/proc/sys/kernel/osrelease")
        .unwrap_or_else(|| "Unknown kernel".to_string());
    let host_name =
        read_first_line("/proc/sys/kernel/hostname").unwrap_or_else(|| "Unknown host".to_string());

    let (cpu_brand, cpu_count) = parse_cpuinfo();
    let (mem_total, mem_free, swap_total, swap_free) = parse_meminfo();
    let (used_gb, total_gb, used_percent) = to_gb_and_ratio(mem_total, mem_free);
    let (swap_used_gb, swap_total_gb, swap_ratio) = to_gb_and_ratio(swap_total, swap_free);

    let (current_user, from_ip) = get_current_user_and_ip();
    let login_user_count = get_logged_in_user_count();

    let virt_info = detect_virtualization();

    let main_iface = get_default_interface().unwrap_or_else(|| "unknown".to_string());
    let main_ip = if main_iface == "unknown" {
        "unknown".to_string()
    } else {
        get_interface_ipv4(&main_iface).unwrap_or_else(|| "unknown".to_string())
    };

    let mut items = Vec::new();

    items.push(("Host name:", host_name.bright_yellow().to_string()));

    items.push((
        "Main NIC:",
        format!("{} ({})", main_iface.bright_cyan(), main_ip.bright_cyan()),
    ));

    items.push((
        "User info:",
        format!(
            "{} (from {}), {} user(s) logged in",
            current_user.bright_cyan(),
            from_ip.bright_cyan(),
            login_user_count.to_string().bright_cyan()
        ),
    ));

    items.push((
        "Current time (TZ):",
        now_str_with_tz.bright_yellow().to_string(),
    ));
    items.push(("System uptime:", uptime_str.bright_yellow().to_string()));
    items.push((
        "Operating system:",
        format!("{} {}", os_name, os_version)
            .bright_yellow()
            .to_string(),
    ));
    items.push(("Kernel version:", kernel_version.bright_green().to_string()));

    if let Some(ref virt) = virt_info {
        items.push(("Virtualization:", virt.bright_yellow().to_string()));
    }

    items.push((
        "CPU:",
        format!(
            "{} ({} cores)",
            cpu_brand.bright_magenta(),
            cpu_count.to_string().bright_magenta()
        ),
    ));

    items.push((
        "Memory used/total:",
        format!("{:.2}/{:.2} GB ({:.2}%)", used_gb, total_gb, used_percent),
    ));
    items.push((
        "Swap used/total:",
        format!(
            "{:.2}/{:.2} GB ({:.2}%)",
            swap_used_gb, swap_total_gb, swap_ratio
        ),
    ));

    print_aligned(&items);

    #[cfg(unix)]
    parse_and_print_disk_usage();

    if verbose {
        println!("{}", "Verbose mode: put extra info here.".bold().cyan());
    }

    println!();

    let farewell_text = match cfg.farewell.as_deref() {
        Some(s) if !s.trim().is_empty() => s,
        _ => "Have a nice day!",
    };
    println!("{}", farewell_text.bold().cyan());
}

fn fetch_welcome_text(value: &str) -> String {
    if let Ok(parsed_url) = Url::parse(value) {
        match Client::builder().timeout(Duration::from_secs(1)).build() {
            Ok(client) => match client.get(parsed_url).send().and_then(|r| r.text()) {
                Ok(resp_text) => {
                    let trimmed = resp_text.trim();
                    if trimmed.is_empty() {
                        "Welcome!".to_string()
                    } else {
                        resp_text
                    }
                }
                Err(e) => {
                    eprintln!("Failed to fetch welcome from '{}': {}", value, e);
                    "Welcome!".to_string()
                }
            },
            Err(e) => {
                eprintln!("Failed to build reqwest client: {}", e);
                "Welcome!".to_string()
            }
        }
    } else if value.trim().is_empty() {
        "Welcome!".to_string()
    } else {
        value.to_string()
    }
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

fn get_logged_in_user_count() -> usize {
    let output = match Command::new("who").arg("-q").output() {
        Ok(o) => o,
        Err(_) => return 0,
    };
    let out_str = String::from_utf8_lossy(&output.stdout);
    for line in out_str.lines() {
        if let Some(pos) = line.find("# users=") {
            return line[pos + 8..].trim().parse().unwrap_or(0);
        }
    }
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

fn print_aligned(items: &[(&str, String)]) {
    let max_key_len = items.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (key, value) in items {
        println!(
            "{:width$} {}",
            key.bright_white(),
            value,
            width = max_key_len
        );
    }
}

#[cfg(unix)]
fn parse_and_print_disk_usage() {
    let file = match File::open("/proc/mounts") {
        Ok(f) => f,
        Err(_) => {
            eprintln!("Failed to open /proc/mounts");
            return;
        }
    };

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        let mount_path = fields[1];
        let fstype = fields[2];

        if mount_path == "/" {
            print_disk_usage(mount_path, "Disk usage (root):");
        } else if matches!(fstype, "nfs" | "nfs4") {
            print_disk_usage(mount_path, "Disk usage (nfs):");
        }
    }
}

#[cfg(unix)]
fn print_disk_usage(mount_path: &str, label: &str) {
    if let Some((total_bytes, used_bytes)) = get_mount_usage(mount_path) {
        let (used_str, total_str, ratio) = human_readable_usage(used_bytes, total_bytes);
        println!(
            "{} {} => {}/{} ({:.2}%)",
            label.bright_white(),
            mount_path.bright_yellow(),
            used_str,
            total_str,
            ratio
        );
    }
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

    let block_size = s.f_frsize as u64;
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
    fn fetch_welcome_text_defaults_when_empty() {
        assert_eq!(fetch_welcome_text(""), "Welcome!");
    }

    #[test]
    fn fetch_welcome_text_returns_literal_strings() {
        assert_eq!(fetch_welcome_text("Plain text"), "Plain text");
    }

    #[test]
    fn to_gb_and_ratio_handles_zero_total() {
        let (used, total, ratio) = to_gb_and_ratio(0, 0);
        assert_eq!(used, 0.0);
        assert_eq!(total, 0.0);
        assert_eq!(ratio, 0.0);
    }

    #[test]
    fn to_gb_and_ratio_computes_expected_values() {
        let (used, total, ratio) = to_gb_and_ratio(2048, 1024);
        assert!((used - 0.0009765625).abs() < f64::EPSILON);
        assert!((total - 0.001953125).abs() < f64::EPSILON);
        assert!((ratio - 50.0).abs() < f64::EPSILON);
    }
}
