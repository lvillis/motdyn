use chrono::Local;
use clap::{Parser, Subcommand};
use colored::*;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
use libc::statvfs;
#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::mem::MaybeUninit;

/// Represents the raw config loaded from `config.toml`.
#[derive(Debug, serde::Deserialize)]
struct RawConfig {
    /// Multi-line ASCII art string.
    ascii_art: Option<String>,
    /// Custom farewell text.
    farewell: Option<String>,
}

/// Represents the final merged config used by motdyn.
#[derive(Debug, Default, Clone)]
struct MotdConfig {
    /// ASCII art to display at the beginning.
    ascii_art: Option<String>,
    /// Farewell text to display at the end.
    farewell: Option<String>,
}

/// CLI entry point. Supports subcommands for install/uninstall/status, or runs MOTD if none.
#[derive(Parser, Debug)]
#[command(
    name = "motdyn",
    version = "0.1.0",
    author = "Your Name",
    about = "MOTD with optional sysinfo, plus install/uninstall/status subcommands"
)]
struct Cli {
    /// Show more detailed info when printing
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Subcommand: install, uninstall, status, or none => run MOTD
    #[command(subcommand)]
    cmd: Option<Commands>,
}

/// Defines subcommands that can be invoked from the CLI.
#[derive(Subcommand, Debug)]
enum Commands {
    /// Install motdyn so that it prints on SSH login
    Install,
    /// Uninstall motdyn from system so it won't print on SSH login
    Uninstall,
    /// Check whether motdyn is installed (i.e., /etc/profile.d/motdyn.sh exists)
    Status,
}

fn main() {
    let cli = Cli::parse();

    match cli.cmd {
        Some(Commands::Install) => {
            if let Err(e) = do_install() {
                eprintln!("Install failed: {}", e);
                std::process::exit(1);
            }
            println!("Install successful!");
        }
        Some(Commands::Uninstall) => {
            if let Err(e) = do_uninstall() {
                eprintln!("Uninstall failed: {}", e);
                std::process::exit(1);
            }
            println!("Uninstall successful!");
        }
        Some(Commands::Status) => {
            if let Err(e) = do_status() {
                eprintln!("Status check failed: {}", e);
                std::process::exit(1);
            }
        }
        None => {
            // No subcommand => run the MOTD logic
            run_motd(cli.verbose);
        }
    }
}

/// Creates a small shell script in `/etc/profile.d/motdyn.sh`,
/// so that on system-wide login, `motdyn` runs automatically.
fn do_install() -> Result<(), Box<dyn std::error::Error>> {
    let profile_dir = "/etc/profile.d";
    if !Path::new(profile_dir).exists() {
        return Err(format!(
            "Directory '{}' not found, cannot install system-wide script.",
            profile_dir
        )
        .into());
    }

    let script_path = format!("{}/motdyn.sh", profile_dir);
    let mut file = File::create(&script_path)?;

    // Writes a simple shell script that calls `motdyn`.
    writeln!(
        file,
        "#!/bin/sh
# This script is auto-generated by 'motdyn install'.
# It will run 'motdyn' on login.
if [ -x \"$(command -v motdyn)\" ]; then
    motdyn
fi
"
    )?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file.metadata()?.permissions();
        perms.set_mode(0o755);
        file.set_permissions(perms)?;
    }

    Ok(())
}

/// Removes the `/etc/profile.d/motdyn.sh` script if it exists.
fn do_uninstall() -> Result<(), Box<dyn std::error::Error>> {
    let script_path = "/etc/profile.d/motdyn.sh";
    if Path::new(script_path).exists() {
        fs::remove_file(script_path)?;
    }
    Ok(())
}

/// Checks if `/etc/profile.d/motdyn.sh` exists, printing the result.
fn do_status() -> Result<(), Box<dyn std::error::Error>> {
    let script_path = "/etc/profile.d/motdyn.sh";
    if Path::new(script_path).exists() {
        println!(
            "The system IS installed with motdyn script at {}",
            script_path
        );
    } else {
        println!(
            "The system is NOT installed with motdyn (no {}).",
            script_path
        );
    }
    Ok(())
}

/// Called when no subcommand is given, merges configs and prints MOTD.
fn run_motd(verbose: bool) {
    let sys_cfg = load_config(Path::new("/etc/motdyn/config.toml"));
    let usr_cfg = load_config(&expand_tilde("~/.config/motdyn/config.toml"));
    let merged_cfg = merge_config(sys_cfg, usr_cfg);

    print_motdyn(verbose, &merged_cfg);
}

/// Loads config from a TOML file, returning `Some(MotdConfig)` or `None` if fail.
fn load_config(path: &Path) -> Option<MotdConfig> {
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    let raw: RawConfig = toml::from_str(&content).ok()?;

    Some(MotdConfig {
        ascii_art: raw.ascii_art,
        farewell: raw.farewell,
    })
}

/// Merges system-level and user-level configs, letting user override fields if present.
fn merge_config(sys_cfg: Option<MotdConfig>, usr_cfg: Option<MotdConfig>) -> MotdConfig {
    let mut final_cfg = sys_cfg.unwrap_or_default();
    if let Some(u) = usr_cfg {
        if let Some(art) = u.ascii_art {
            final_cfg.ascii_art = Some(art);
        }
        if let Some(fw) = u.farewell {
            final_cfg.farewell = Some(fw);
        }
    }
    final_cfg
}

/// Expands `~` to `$HOME` if present, returning a `PathBuf`.
fn expand_tilde(path_str: &str) -> PathBuf {
    if !path_str.starts_with('~') {
        return PathBuf::from(path_str);
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(path_str.replacen('~', &home.to_string_lossy(), 1));
    }
    PathBuf::from(path_str)
}

/// Prints MOTD using the merged config, optionally showing verbose details.
fn print_motdyn(verbose: bool, cfg: &MotdConfig) {
    // If there's ASCII art, print it first.
    if let Some(ref art) = cfg.ascii_art {
        println!();
        println!("{}", art);
        println!();
    }

    let (os_name, os_version) = get_os_info();
    let now = Local::now();

    // Now includes timezone info in the time string
    // e.g. "2024-12-27 17:36:25 +08:00"
    let now_str_with_tz = now.format("%Y-%m-%d %H:%M:%S %:z").to_string();

    // Parse uptime from /proc/uptime
    let uptime_str = match parse_uptime() {
        Some(up_str) => up_str,
        None => "unknown".to_string(),
    };

    let kernel_version = match read_first_line("/proc/sys/kernel/osrelease") {
        Some(s) => s,
        None => "Unknown kernel".to_string(),
    };
    let host_name = match read_first_line("/proc/sys/kernel/hostname") {
        Some(s) => s,
        None => "Unknown host".to_string(),
    };

    let (cpu_brand, cpu_count) = parse_cpuinfo();
    let (mem_total, mem_free, swap_total, swap_free) = parse_meminfo();
    let (used_gb, total_gb, used_percent) = to_gb_and_ratio(mem_total, mem_free);
    let (swap_used_gb, swap_total_gb, swap_ratio) = to_gb_and_ratio(swap_total, swap_free);

    let (current_user, from_ip) = get_current_user_and_ip();
    let login_user_count = get_logged_in_user_count();

    println!("{}", "Welcome!".bold().cyan());
    println!();

    let mut items = Vec::new();

    // Show current time with timezone
    items.push((
        "Current time (TZ):",
        now_str_with_tz.bright_yellow().to_string(),
    ));

    // Show system uptime
    items.push(("System uptime:", uptime_str.bright_yellow().to_string()));

    let os_val = format!("{} {}", os_name, os_version)
        .bright_yellow()
        .to_string();
    items.push(("Operating system:", os_val));

    items.push(("Kernel version:", kernel_version.bright_green().to_string()));
    items.push(("Host name:", host_name.bright_yellow().to_string()));

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

    items.push((
        "Current user:",
        format!(
            "{} (from {})",
            current_user.bright_cyan(),
            from_ip.bright_cyan()
        ),
    ));

    items.push((
        "Login user count:",
        login_user_count.to_string().bright_cyan().to_string(),
    ));

    print_aligned(&items);

    #[cfg(unix)]
    parse_and_print_disk_usage();

    if verbose {
        println!("{}", "Verbose mode: put extra info here.".bold().cyan());
    }

    println!();

    // If farewell is empty or missing, use default
    let farewell_text = match cfg.farewell.as_deref() {
        Some(s) if !s.trim().is_empty() => s,
        _ => "Have a nice day!",
    };
    println!("{}", farewell_text.bold().cyan());
}

/// Reads and parses system uptime from /proc/uptime, returning a string like "2 days, 05:13:42".
fn parse_uptime() -> Option<String> {
    let line = std::fs::read_to_string("/proc/uptime").ok()?;
    // /proc/uptime format: "25333.53 1022.3"
    // first float is total seconds
    let parts: Vec<_> = line.split_whitespace().collect();
    let total_seconds = parts.get(0)?.parse::<f64>().ok()? as u64;

    Some(format_uptime(total_seconds))
}

/// Converts total uptime seconds to "X days, HH:MM:SS".
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

/// Returns (os_name, os_version) if found, or else uses fallback approach.
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

/// Returns (os_name, ver_str) if it finds " release " in /etc/redhat-release.
fn parse_redhat_release() -> Option<(String, String)> {
    let content = fs::read_to_string("/etc/redhat-release").ok()?;
    let line = content.trim();
    let needle = " release ";
    let pos = line.find(needle)?;
    let os_name = &line[..pos];
    let ver_str = &line[pos + needle.len()..];
    Some((os_name.to_string(), ver_str.to_string()))
}

/// Returns (os_name, os_version) from /etc/os-release if found.
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

/// Reads /proc/meminfo and returns (mem_total_kb, mem_free_kb, swap_total_kb, swap_free_kb).
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

    for line in reader.lines().flatten() {
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

/// If MemAvailable not found, fallback to MemFree.
fn fallback_mem_free() -> Option<u64> {
    let file = File::open("/proc/meminfo").ok()?;
    for line in BufReader::new(file).lines().flatten() {
        if let Some(stripped) = line.strip_prefix("MemFree:") {
            let val = stripped.trim().split_whitespace().next()?;
            return val.parse::<u64>().ok();
        }
    }
    None
}

/// Reads /proc/cpuinfo and returns (cpu_brand, cpu_cores).
fn parse_cpuinfo() -> (String, usize) {
    let file = match File::open("/proc/cpuinfo") {
        Ok(f) => f,
        Err(_) => return ("Unknown CPU".to_string(), 0),
    };
    let reader = BufReader::new(file);

    let mut brand = "Unknown CPU".to_string();
    let mut core_count = 0;

    for line in reader.lines().flatten() {
        if line.starts_with("processor") {
            core_count += 1;
        } else if let Some(model_str) = line.strip_prefix("model name") {
            let parts: Vec<_> = model_str.split(':').collect();
            if parts.len() > 1 && brand == "Unknown CPU" {
                brand = parts[1].trim().to_string();
            }
        }
    }
    (brand, core_count)
}

/// Reads the first line from a file (trimmed). Returns None if fail.
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

/// Returns (username, from_ip).
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

/// If fail, returns 0.
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

/// Converts (total_kb, free_kb) to used_gb / total_gb and usage ratio.
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

/// Converts kB to GB as f64.
fn kb_to_gb(kb: u64) -> f64 {
    kb as f64 / 1024.0 / 1024.0
}

/// Prints key-value pairs in aligned format.
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
/// Parses `/proc/mounts` and prints disk usage for root or NFS, automatically scaling to KB/MB/GB/TB/PB if needed.
fn parse_and_print_disk_usage() {
    let file = match File::open("/proc/mounts") {
        Ok(f) => f,
        Err(_) => {
            eprintln!("Failed to open /proc/mounts");
            return;
        }
    };

    for line in BufReader::new(file).lines().flatten() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        let mount_path = fields[1];
        let fstype = fields[2];

        if mount_path == "/" {
            print_disk_usage(mount_path, "Disk usage (root):");
        } else if matches!(fstype, "nfs" | "nfs4") {
            print_disk_usage(mount_path, "Disk usage (NFS):");
        }
    }
}

#[cfg(unix)]
/// Prints disk usage for a given path, using `get_mount_usage` + `human_readable_usage`.
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
/// Gets total_bytes and used_bytes for the mountpoint via `statvfs`.
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
/// Converts usage and total (in bytes) to a scale-based string (KB, MB, GB, TB, PB).
/// Returns (used_str, total_str, usage_ratio).
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
/// Decides best scale for the given size (in bytes) and returns (scale_value, suffix).
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