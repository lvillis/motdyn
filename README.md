<div align=right>Table of Contents↗️</div>

<div align="center">
  <h1><code>motdyn</code></h1>
  <p><strong>Dynamic MOTD CLI for Linux login sessions.</strong></p>

  <p>
    <a href="https://crates.io/crates/motdyn">
      <img src="https://img.shields.io/crates/v/motdyn.svg" alt="crates.io version">
    </a>
    <a href="https://github.com/lvillis/motdyn">
      <img src="https://img.shields.io/github/repo-size/lvillis/motdyn?style=flat-square&color=328657" alt="repository size">
    </a>
    <a href="https://github.com/lvillis/motdyn/actions">
      <img src="https://github.com/lvillis/motdyn/actions/workflows/ci.yaml/badge.svg" alt="build status">
    </a>
    <a href="https://hub.docker.com/r/lvillis/motdyn">
      <img src="https://img.shields.io/docker/pulls/lvillis/motdyn?style=flat-square" alt="docker pulls">
    </a>
    <a href="https://hub.docker.com/r/lvillis/motdyn">
      <img src="https://img.shields.io/docker/image-size/lvillis/motdyn/latest?style=flat-square" alt="image size">
    </a>
  </p>
</div>

`motdyn` prints a concise snapshot of the current host state when a user opens a shell. It is built for Linux SSH and login-shell workflows where fast context matters more than dashboards or long-term monitoring.

## Quick Start

Install from crates.io:

```bash
cargo install motdyn
```

Run once:

```bash
motdyn
```

Install for login shells:

```bash
sudo motdyn install
motdyn install --user
motdyn install --user --target bash_profile
```

Check status:

```bash
motdyn status
motdyn status --user
motdyn status --user --target zprofile
```

## What It Shows

Depending on the environment and configured module order, `motdyn` can print:

- current time and timezone
- system uptime
- operating system and kernel
- host name
- CPU, memory, and swap usage
- current user, source IP, and logged-in user count
- main network interface and IPv4 address
- root and NFS filesystem usage
- virtualization or container information

## Installation

### Cargo

```bash
cargo install motdyn
```

### Docker

```bash
docker build -t motdyn .
docker run --rm motdyn --help
```

### Login Hooks

System-wide install writes:

```text
/etc/profile.d/motdyn.sh
```

User install always manages:

```text
~/.profile
```

If they already exist, user install also updates:

- `~/.bash_profile`
- `~/.bash_login`
- `~/.zprofile`

Remove hooks with:

```bash
sudo motdyn uninstall
motdyn uninstall --user
motdyn uninstall --user --target bash_profile
```

User install blocks are guarded so `motdyn` only runs once even if multiple login profile files source each other.

## Usage

```bash
motdyn [OPTIONS] [COMMAND]
```

Commands:

- `install`
- `uninstall`
- `status`

Global options:

- `-v`, `--verbose`
- `--plain`
- `--compact`
- `--section-headers`

Without a subcommand, `motdyn` prints the current MOTD immediately.

## Configuration

System config:

```text
/etc/motdyn/config.toml
```

User config:

```text
~/.config/motdyn/config.toml
```

User config overrides system config.

Example:

```toml
welcome_sources = [
  "./motd.txt",
  "file:///etc/motdyn/welcome.txt",
  "https://example.com/motd.txt",
]
farewell = "Have a nice day!"
modules = ["host", "network", "user", "time", "uptime", "load", "os", "kernel", "virtualization", "cpu", "memory", "swap", "disk", "last_login", "failed_login", "services", "updates"]

[remote_welcome]
enabled = true
timeout_ms = 250
cache_ttl_secs = 300
cache_path = "~/.cache/motdyn/welcome.txt"
follow_redirects = true
allow_http = false

[service_status]
services = ["sshd", "chronyd", "docker"]

[output]
compact = false
plain = false
section_headers = false
hidden_fields = ["source_ip", "nfs_disks"]
```

Supported module names:

- `host`
- `network`
- `user`
- `time`
- `uptime`
- `load`
- `os`
- `kernel`
- `virtualization`
- `cpu`
- `memory`
- `swap`
- `disk`
- `last_login`
- `failed_login`
- `services`
- `updates`

Notes:

- `welcome` is the single-source shortcut. `welcome_sources` is preferred when you want ordered fallback.
- each welcome source may be a literal string, a local path, a `file://` URL, or an `http`/`https` URL.
- sources are tried in order until one yields usable content.
- if `modules` is omitted, the default built-in order is used: `host`, `network`, `user`, `time`, `uptime`, `load`, `os`, `kernel`, `virtualization`, `cpu`, `memory`, `swap`, `disk`.
- if `modules` is an empty list, only the welcome and farewell text are shown.
- `services` is opt-in and reads the ordered list from `[service_status].services`.
- `updates` is opt-in and uses local package manager metadata when `apt` or `dnf` is available.
- `last_login` and `failed_login` use `lastlog` and `lastb` when present.
- external probes use short hard timeouts so slow commands do not stall login.
- login hook scripts ignore `motdyn` failures so a non-zero exit does not abort shell startup.
- remote welcome defaults to a 250 ms timeout, uses a local cache, sends `If-None-Match` / `If-Modified-Since` when cache metadata is available, and falls back to stale cache or default text on failure.
- `--verbose` shows config loading, module resolution, output mode, probe sources, degraded modules, and fallback notes.
- `hidden_fields` currently supports `main_interface`, `main_ipv4`, `source_ip`, `login_user_count`, `timezone`, `kernel_version`, `virtualization`, `swap`, and `nfs_disks`.

## Example Output

```shell
$ motdyn

Welcome!

Host name:         prod-hpc-01
Main NIC:          bond0 (10.10.8.24)
User info:         admin (from 10.10.1.15), 4 user(s) logged in
Current time (TZ): 2026-01-15 09:30:00 +00:00
System uptime:     24 days, 18:42:11
Load average:      0.42 0.38 0.35
Operating system:  Rocky Linux 9.5
Kernel version:    5.14.0-503.15.1.el9_5.x86_64
Virtualization:    kvm
CPU:               2x AMD EPYC 9654 (192 cores)
Memory used/total: 384.00/1536.00 GB (25.00%)
Swap used/total:   0.00/64.00 GB (0.00%)
Disk usage (root): / => 1.20 TB/7.68 TB (15.62%)

Have a nice day!
```

## Compatibility

`motdyn` is Linux-oriented and works best in environments with:

- `/proc`
- `ip`
- `/etc/profile.d`
- `systemd-detect-virt` (optional)
- Linux `utmp` session records for login user counting
- `lastlog` and `lastb` for the login-security modules
- `systemctl` for the service status module
- `apt` or `dnf` for the updates module

Some fields may degrade to `unknown` on minimal systems or containers.
