use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

#[cfg(target_os = "linux")]
use libc::USER_PROCESS;
use tempfile::tempdir;

use crate::config::{MotdConfig, OutputConfig, RemoteWelcomeConfig};

use super::probe::{
    count_logged_in_users_from_linux_utmp_file, detect_virtualization_from_cgroup, format_uptime,
    parse_apt_upgradable_output, parse_cpuinfo_content, parse_default_interface_output,
    parse_dnf_check_update_output, parse_interface_ipv4_output, parse_lastb_output,
    parse_lastlog_output, parse_loadavg_content, parse_meminfo_content, parse_os_release_content,
    parse_redhat_release_content, parse_ssh_connection_ip, parse_uptime_content,
    run_command_with_timeout, to_gb_and_ratio,
};
use super::render::{
    build_verbose_items, default_modules, render_module_lines, resolve_modules,
    resolve_output_settings,
};
use super::types::{
    HiddenField, ModuleKind, ModuleSelection, ModuleSource, NetworkProbeError, OutputSettings,
    ProbeIssue, RenderContext, RenderedItem, SnapshotDiagnostics, SystemSnapshot, UsageSummary,
    WelcomeCacheEntry, WelcomeResolution, WelcomeSource, DEFAULT_WELCOME,
};
#[cfg(target_os = "linux")]
use super::types::{LinuxUtmpExitStatus, LinuxUtmpRecord, LinuxUtmpTimeVal32};
use super::welcome::{
    current_unix_secs, read_welcome_cache, resolve_remote_welcome_settings, resolve_welcome_text,
    write_welcome_cache,
};

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
fn parse_uptime_content_rejects_invalid_input() {
    assert_eq!(parse_uptime_content("not-a-number 0"), None);
    assert_eq!(parse_uptime_content(""), None);
}

#[cfg(unix)]
#[test]
fn run_command_with_timeout_kills_slow_processes() {
    let err = run_command_with_timeout("sh", &["-c", "sleep 1"], &[], 50).unwrap_err();
    assert!(err.contains("timed out"));
}

#[test]
fn parse_loadavg_content_reads_first_three_values() {
    assert_eq!(
        parse_loadavg_content("0.42 0.38 0.35 2/345 6789\n"),
        Some("0.42 0.38 0.35".to_string())
    );
    assert_eq!(parse_loadavg_content("0.42 0.38"), None);
}

#[test]
fn resolve_output_settings_normalizes_hidden_fields_and_ignores_unknowns() {
    let cfg = MotdConfig {
        output: OutputConfig {
            compact: Some(true),
            plain: Some(true),
            section_headers: Some(true),
            hidden_fields: Some(vec!["source_ip".into(), "nfs".into(), "bogus".into()]),
        },
        ..MotdConfig::default()
    };

    let settings = resolve_output_settings(&cfg);
    assert!(settings.compact);
    assert!(settings.plain);
    assert!(settings.section_headers);
    assert!(settings.hidden(HiddenField::SourceIp));
    assert!(settings.hidden(HiddenField::NfsDisks));
    assert_eq!(settings.ignored_hidden_fields, vec!["bogus".to_string()]);
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
fn fetch_welcome_text_reads_local_file_sources() {
    let dir = tempdir().unwrap();
    let welcome_path = dir.path().join("welcome.txt");
    fs::write(&welcome_path, "Local welcome\n").unwrap();

    let cfg = MotdConfig {
        welcome_sources: Some(vec![welcome_path.display().to_string()]),
        ..MotdConfig::default()
    };
    let resolution = resolve_welcome_text(&cfg);
    let expected_url = welcome_path.display().to_string();

    assert_eq!(resolution.source, WelcomeSource::LocalFile);
    assert_eq!(resolution.text, "Local welcome\n");
    assert_eq!(resolution.url.as_deref(), Some(expected_url.as_str()));
}

#[test]
fn fetch_welcome_text_uses_next_source_after_failure() {
    let dir = tempdir().unwrap();
    let missing_path = dir.path().join("missing.txt");

    let cfg = MotdConfig {
        welcome_sources: Some(vec![
            missing_path.display().to_string(),
            "Fallback literal".into(),
        ]),
        ..MotdConfig::default()
    };
    let resolution = resolve_welcome_text(&cfg);

    assert_eq!(resolution.source, WelcomeSource::Literal);
    assert_eq!(resolution.text, "Fallback literal");
    assert!(resolution.warnings.iter().any(|warning| matches!(
        warning,
        super::types::WelcomeIssue::LocalFileRead { path, .. } if path == &missing_path
    )));
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
fn parse_os_release_content_requires_name_and_version() {
    assert_eq!(
        parse_os_release_content("NAME=\"Rocky Linux\"\nVERSION_ID=\"9.7\"\n"),
        Some(("Rocky Linux".to_string(), "9.7".to_string()))
    );
    assert_eq!(parse_os_release_content("NAME=\"Rocky Linux\"\n"), None);
    assert_eq!(parse_os_release_content("VERSION_ID=\"9.7\"\n"), None);
}

#[test]
fn parse_redhat_release_content_rejects_unexpected_format() {
    assert_eq!(
        parse_redhat_release_content("Rocky Linux release 9.7 (Blue Onyx)\n"),
        Some(("Rocky Linux".to_string(), "9.7 (Blue Onyx)".to_string()))
    );
    assert_eq!(parse_redhat_release_content("Rocky Linux 9.7"), None);
}

#[test]
fn parse_lastlog_output_extracts_latest_entry() {
    let output = "\
Username         Port     From                                       Latest\n\
admin            pts/0    10.10.1.15                                 Thu Mar 30 09:30:00 +0000 2026\n";

    assert_eq!(
        parse_lastlog_output(output),
        Some(Some(
            "Thu Mar 30 09:30:00 +0000 2026, from 10.10.1.15, via pts/0".to_string()
        ))
    );
}

#[test]
fn parse_lastlog_output_handles_never_logged_in() {
    let output = "\
Username         Port     From                                       Latest\n\
admin                                                         **Never logged in**\n";

    assert_eq!(parse_lastlog_output(output), Some(None));
}

#[test]
fn parse_lastb_output_summarizes_recent_failures() {
    let output = "\
admin    ssh:notty   10.10.1.20   Thu Mar 30 09:25 - 09:25  (00:00)\n\
admin    ssh:notty   10.10.1.21   Thu Mar 30 09:20 - 09:20  (00:00)\n\
btmp begins Thu Mar 30 00:00:00 2026\n";

    assert_eq!(
        parse_lastb_output(output),
        Some(Some(
            "2 recent failures, last from 10.10.1.20, via ssh:notty, at Thu Mar 30 09:25 - 09:25 (00:00)"
                .to_string()
        ))
    );
}

#[test]
fn parse_package_update_outputs_count_packages() {
    let apt_output = "\
Listing...\n\
bash/stable 5.2 amd64 [upgradable from: 5.1]\n\
curl/stable 8.7 amd64 [upgradable from: 8.6]\n";
    let dnf_output = "\
Last metadata expiration check: 0:42:11 ago on Thu 30 Mar 2026 09:00:00 AM UTC.\n\
bash.x86_64          5.2-1.el9          baseos\n\
curl.x86_64          8.7-1.el9          appstream\n";

    assert_eq!(parse_apt_upgradable_output(apt_output), 2);
    assert_eq!(parse_dnf_check_update_output(dnf_output), 2);
}

#[test]
fn parse_meminfo_content_falls_back_to_memfree_when_memavailable_missing() {
    let content = "\
MemTotal:       16384 kB\n\
MemFree:         4096 kB\n\
SwapTotal:       2048 kB\n\
SwapFree:        1024 kB\n";

    assert_eq!(parse_meminfo_content(content), (16384, 4096, 2048, 1024));
}

#[test]
fn parse_meminfo_content_tolerates_invalid_values() {
    let content = "\
MemTotal:       nope kB\n\
MemAvailable:   2048 kB\n\
SwapTotal:      broken kB\n\
SwapFree:       1024 kB\n";

    assert_eq!(parse_meminfo_content(content), (0, 2048, 0, 1024));
}

#[test]
fn parse_cpuinfo_content_detects_arm_fallback_brand() {
    let content = "\
processor\t: 0\n\
processor\t: 1\n\
CPU implementer\t: 0x41\n\
CPU part\t: 0xd08\n";

    assert_eq!(
        parse_cpuinfo_content(content),
        ("ARM Cortex-A72".to_string(), 2)
    );
}

#[test]
fn parse_ssh_connection_ip_defaults_to_unknown_when_missing() {
    assert_eq!(parse_ssh_connection_ip(""), "unknown");
    assert_eq!(parse_ssh_connection_ip("   "), "unknown");
    assert_eq!(
        parse_ssh_connection_ip("192.168.2.2 54012 192.168.88.121 22"),
        "192.168.2.2"
    );
}

#[test]
fn parse_default_interface_output_skips_malformed_lines() {
    let output = "\
default via 192.168.88.1 dev ens192 proto static\n\
default dev broken\n";
    assert_eq!(
        parse_default_interface_output(output),
        Some("ens192".to_string())
    );
    assert_eq!(parse_default_interface_output("default dev broken\n"), None);
}

#[test]
fn parse_interface_ipv4_output_returns_none_without_inet_record() {
    let valid = "2: ens192    inet 192.168.88.121/24 brd 192.168.88.255 scope global ens192\n";
    let invalid = "2: ens192    mtu 1500 qdisc mq state UP mode DEFAULT group default qlen 1000\n";

    assert_eq!(
        parse_interface_ipv4_output(valid),
        Some("192.168.88.121".to_string())
    );
    assert_eq!(parse_interface_ipv4_output(invalid), None);
}

#[test]
fn detect_virtualization_from_cgroup_detects_known_runtimes() {
    assert_eq!(
        detect_virtualization_from_cgroup("12:cpu:/docker/abcdef"),
        Some("Docker".to_string())
    );
    assert_eq!(
        detect_virtualization_from_cgroup("1:name=systemd:/lxc.payload/demo"),
        Some("LXC".to_string())
    );
    assert_eq!(detect_virtualization_from_cgroup("0::/"), None);
}

#[test]
fn render_module_lines_applies_hidden_fields_and_plain_output() {
    let mut hidden = HashSet::new();
    hidden.insert(HiddenField::SourceIp);
    hidden.insert(HiddenField::Timezone);
    hidden.insert(HiddenField::NfsDisks);
    let settings = OutputSettings {
        compact: true,
        plain: true,
        section_headers: false,
        hidden_fields: hidden,
        ignored_hidden_fields: Vec::new(),
    };

    let lines = render_module_lines(
        &[
            ModuleKind::User,
            ModuleKind::Time,
            ModuleKind::Disk,
            ModuleKind::Virtualization,
        ],
        &sample_snapshot(),
        &settings,
    );
    let rendered = lines.join("\n");

    assert!(rendered.contains("admin, 4 user(s) logged in"));
    assert!(!rendered.contains("10.10.1.15"));
    assert!(rendered.contains("2026-01-15 09:30:00"));
    assert!(!rendered.contains("+00:00"));
    assert!(rendered.contains("Disk usage (root):"));
    assert!(!rendered.contains("Disk usage (nfs):"));
    assert!(!rendered.contains('\u{1b}'));
}

#[test]
fn render_module_lines_inserts_section_headers_when_enabled() {
    let settings = OutputSettings {
        compact: false,
        plain: true,
        section_headers: true,
        hidden_fields: HashSet::new(),
        ignored_hidden_fields: Vec::new(),
    };

    let lines = render_module_lines(
        &[ModuleKind::Host, ModuleKind::Time, ModuleKind::Disk],
        &sample_snapshot(),
        &settings,
    );

    assert!(lines.iter().any(|line| line == "Identity"));
    assert!(lines.iter().any(|line| line == "Runtime"));
    assert!(lines.iter().any(|line| line == "Storage"));
    assert!(lines.iter().any(|line| line.is_empty()));
}

#[test]
fn build_verbose_items_reports_degraded_modules_and_ignored_fields() {
    let mut snapshot = sample_snapshot();
    snapshot.diagnostics.degrade(
        ModuleKind::Network,
        ProbeIssue::Network(NetworkProbeError::DefaultRouteCommand(
            "ip command unavailable".to_string(),
        )),
    );
    snapshot.diagnostics.note(ProbeIssue::SshConnectionMissing);

    let output = OutputSettings {
        compact: false,
        plain: true,
        section_headers: false,
        hidden_fields: HashSet::new(),
        ignored_hidden_fields: vec!["bogus".to_string()],
    };
    let items = build_verbose_items(
        &MotdConfig::default(),
        &RenderContext {
            system_config_path: "/etc/motdyn/config.toml".into(),
            system_config_status: "missing".into(),
            user_config_path: "/root/.config/motdyn/config.toml".into(),
            user_config_status: "missing".into(),
            config_notes: Vec::new(),
        },
        &ModuleSelection {
            modules: default_modules(),
            ignored: Vec::new(),
            source: ModuleSource::Default,
        },
        &WelcomeResolution {
            text: DEFAULT_WELCOME.to_string(),
            source: WelcomeSource::Default,
            source_detail: "default welcome".to_string(),
            url: None,
            settings: resolve_remote_welcome_settings(&MotdConfig::default()),
            warnings: Vec::new(),
        },
        &snapshot,
        &output,
    );

    assert!(items
        .iter()
        .any(|item| item.label == "Degraded modules:" && item.value.contains("network")));
    assert!(items
        .iter()
        .any(|item| item.label == "Ignored fields:" && item.value.contains("bogus")));
    assert!(items
        .iter()
        .any(|item| item.label == "Probe notes:" && item.value.contains("SSH_CONNECTION")));
}

#[test]
fn write_and_read_welcome_cache_round_trip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("welcome.cache");
    let entry = WelcomeCacheEntry {
        url: "https://example.com/motd.txt".into(),
        fetched_at_secs: 123,
        etag: Some("\"abc123\"".into()),
        last_modified: Some("Mon, 01 Jan 2024 00:00:00 GMT".into()),
        body: "hello\nworld".into(),
    };

    write_welcome_cache(&path, &entry).unwrap();
    let restored = read_welcome_cache(&path).unwrap().unwrap();

    assert_eq!(restored.url, entry.url);
    assert_eq!(restored.fetched_at_secs, entry.fetched_at_secs);
    assert_eq!(restored.etag, entry.etag);
    assert_eq!(restored.last_modified, entry.last_modified);
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
            etag: None,
            last_modified: None,
            body: "cached welcome".into(),
        },
    )
    .unwrap();

    let cfg = MotdConfig {
        welcome: Some("https://example.com/motd.txt".into()),
        remote_welcome: RemoteWelcomeConfig {
            cache_path: Some(cache_path.display().to_string()),
            ..RemoteWelcomeConfig::default()
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
            etag: None,
            last_modified: None,
            body: "stale welcome".into(),
        },
    )
    .unwrap();

    let cfg = MotdConfig {
        welcome: Some("https://example.com/motd.txt".into()),
        remote_welcome: RemoteWelcomeConfig {
            enabled: Some(false),
            cache_path: Some(cache_path.display().to_string()),
            cache_ttl_secs: Some(0),
            ..RemoteWelcomeConfig::default()
        },
        ..MotdConfig::default()
    };

    let resolution = resolve_welcome_text(&cfg);
    assert_eq!(resolution.source, WelcomeSource::CacheStale);
    assert_eq!(resolution.text, "stale welcome");
    assert!(resolution
        .warnings
        .iter()
        .any(|warning| warning.to_string().contains("disabled")));
}

#[test]
fn resolve_welcome_reports_malformed_cache() {
    let dir = tempdir().unwrap();
    let cache_path = dir.path().join("welcome.cache");
    fs::write(&cache_path, "broken cache").unwrap();

    let cfg = MotdConfig {
        welcome: Some("https://example.com".into()),
        remote_welcome: RemoteWelcomeConfig {
            enabled: Some(false),
            cache_path: Some(cache_path.display().to_string()),
            ..RemoteWelcomeConfig::default()
        },
        ..MotdConfig::default()
    };

    let resolution = resolve_welcome_text(&cfg);
    assert_eq!(resolution.source, WelcomeSource::Default);
    assert!(resolution
        .warnings
        .iter()
        .any(|warning| warning.to_string().contains("malformed")));
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
            etag: None,
            last_modified: None,
            body: "cached normalized welcome".into(),
        },
    )
    .unwrap();

    let cfg = MotdConfig {
        welcome: Some("https://example.com:443/#section".into()),
        remote_welcome: RemoteWelcomeConfig {
            cache_path: Some(cache_path.display().to_string()),
            ..RemoteWelcomeConfig::default()
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
        remote_welcome: RemoteWelcomeConfig {
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
            ..RemoteWelcomeConfig::default()
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
        .any(|warning| warning.to_string().contains("HTTP 404")));
}

#[test]
fn remote_welcome_revalidates_stale_cache_on_http_304() {
    let dir = tempdir().unwrap();
    let cache_path = dir.path().join("welcome.cache");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let welcome_url = format!("http://{}/motd.txt", addr);
    let request_etag = "\"abc123\"".to_string();
    let request_last_modified = "Mon, 01 Jan 2024 00:00:00 GMT".to_string();
    write_welcome_cache(
        &cache_path,
        &WelcomeCacheEntry {
            url: welcome_url.clone(),
            fetched_at_secs: 1,
            etag: Some(request_etag.clone()),
            last_modified: Some(request_last_modified.clone()),
            body: "cached welcome".into(),
        },
    )
    .unwrap();
    let cache_path_for_cfg = cache_path.display().to_string();
    let server = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0_u8; 1024];
            let bytes_read = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..bytes_read]).to_ascii_lowercase();
            assert!(request.contains("if-none-match: \"abc123\""));
            assert!(request.contains("if-modified-since: mon, 01 jan 2024 00:00:00 gmt"));

            let response = concat!(
                "HTTP/1.1 304 Not Modified\r\n",
                "ETag: \"abc123\"\r\n",
                "Last-Modified: Mon, 01 Jan 2024 00:00:00 GMT\r\n",
                "Content-Length: 0\r\n",
                "Connection: close\r\n",
                "\r\n"
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });

    let cfg = MotdConfig {
        welcome: Some(welcome_url),
        remote_welcome: RemoteWelcomeConfig {
            allow_http: Some(true),
            timeout_ms: Some(500),
            cache_ttl_secs: Some(0),
            cache_path: Some(cache_path_for_cfg),
            ..RemoteWelcomeConfig::default()
        },
        ..MotdConfig::default()
    };

    let resolution = resolve_welcome_text(&cfg);
    server.join().unwrap();

    assert_eq!(resolution.source, WelcomeSource::CacheRevalidated);
    assert_eq!(resolution.text, "cached welcome");

    let restored = read_welcome_cache(&cache_path).unwrap().unwrap();
    assert_eq!(restored.etag, Some(request_etag));
    assert_eq!(restored.last_modified, Some(request_last_modified));
    assert!(restored.fetched_at_secs > 1);
}

#[cfg(target_os = "linux")]
#[test]
fn count_logged_in_users_from_linux_utmp_file_counts_user_process_records() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("utmp");
    let mut file = File::create(&path).unwrap();

    write_linux_utmp_record(&mut file, USER_PROCESS, "alice");
    write_linux_utmp_record(&mut file, USER_PROCESS, "");
    write_linux_utmp_record(&mut file, 8, "");
    write_linux_utmp_record(&mut file, USER_PROCESS, "root");

    assert_eq!(count_logged_in_users_from_linux_utmp_file(&path), Some(2));
}

#[test]
fn to_gb_and_ratio_handles_zero_total() {
    let (used, total, ratio) = to_gb_and_ratio(0, 0);
    assert_eq!(used, 0.0);
    assert_eq!(total, 0.0);
    assert_eq!(ratio, 0.0);
}

#[cfg(target_os = "linux")]
fn write_linux_utmp_record(file: &mut File, ut_type: i16, user: &str) {
    let mut record = LinuxUtmpRecord {
        ut_type,
        ut_pid: 0,
        ut_line: [0; 32],
        ut_id: [0; 4],
        ut_user: [0; 32],
        ut_host: [0; 256],
        ut_exit: LinuxUtmpExitStatus {
            e_termination: 0,
            e_exit: 0,
        },
        ut_session: 0,
        ut_tv: LinuxUtmpTimeVal32 {
            tv_sec: 0,
            tv_usec: 0,
        },
        ut_addr_v6: [0; 4],
        __unused: [0; 20],
    };

    for (slot, byte) in record.ut_user.iter_mut().zip(user.bytes()) {
        *slot = byte;
    }

    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&record as *const LinuxUtmpRecord).cast::<u8>(),
            std::mem::size_of::<LinuxUtmpRecord>(),
        )
    };
    file.write_all(bytes).unwrap();
}

fn sample_snapshot() -> SystemSnapshot {
    SystemSnapshot {
        host_name: "prod-hpc-01".to_string(),
        main_iface: "bond0".to_string(),
        main_ip: "10.10.8.24".to_string(),
        current_user: "admin".to_string(),
        from_ip: "10.10.1.15".to_string(),
        login_user_count: 4,
        now_str_with_tz: "2026-01-15 09:30:00 +00:00".to_string(),
        uptime_str: "24 days, 18:42:11".to_string(),
        load_average: "0.42 0.38 0.35".to_string(),
        os_name: "Rocky Linux".to_string(),
        os_version: "9.5".to_string(),
        kernel_version: "5.14.0-503.15.1.el9_5.x86_64".to_string(),
        virt_info: Some("kvm".to_string()),
        cpu_brand: "2x AMD EPYC 9654".to_string(),
        cpu_count: 192,
        memory: UsageSummary {
            used_gb: 384.0,
            total_gb: 1536.0,
            ratio: 25.0,
        },
        swap: UsageSummary {
            used_gb: 0.0,
            total_gb: 64.0,
            ratio: 0.0,
        },
        disk_items: vec![
            RenderedItem {
                label: "Disk usage (root):".to_string(),
                value: "/ => 1.20 TB/7.68 TB (15.62%)".to_string(),
            },
            RenderedItem {
                label: "Disk usage (nfs):".to_string(),
                value: "/NFS => 1.72 TB/1.97 TB (87.47%)".to_string(),
            },
        ],
        last_login: "Thu Mar 30 09:30:00 +0000 2026, from 10.10.1.15, via pts/0".to_string(),
        failed_login: "none".to_string(),
        service_items: vec![
            RenderedItem {
                label: "Service sshd:".to_string(),
                value: "active".to_string(),
            },
            RenderedItem {
                label: "Service chronyd:".to_string(),
                value: "active".to_string(),
            },
        ],
        update_summary: "2 package(s) via dnf".to_string(),
        diagnostics: SnapshotDiagnostics {
            degraded_modules: Vec::new(),
            issues: Vec::new(),
            os_source: "/etc/redhat-release".to_string(),
            network_source: "ip route/ip addr".to_string(),
            login_user_count_source: "linux utmp".to_string(),
            virtualization_source: "systemd-detect-virt".to_string(),
            load_source: "/proc/loadavg".to_string(),
            last_login_source: "lastlog".to_string(),
            failed_login_source: "lastb".to_string(),
            service_status_source: "systemctl is-active".to_string(),
            updates_source: "dnf check-update --cacheonly".to_string(),
        },
    }
}
