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
```

Check status:

```bash
motdyn status
motdyn status --user
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
```

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
welcome = "https://example.com/motd.txt"
farewell = "Have a nice day!"
modules = ["host", "network", "user", "time", "uptime", "os", "kernel", "virtualization", "cpu", "memory", "swap", "disk"]

[remote_welcome]
enabled = true
timeout_ms = 1000
cache_ttl_secs = 300
cache_path = "~/.cache/motdyn/welcome.txt"
follow_redirects = true
allow_http = false
```

Supported module names:

- `host`
- `network`
- `user`
- `time`
- `uptime`
- `os`
- `kernel`
- `virtualization`
- `cpu`
- `memory`
- `swap`
- `disk`

Notes:

- `welcome` may be a literal string or a URL.
- if `modules` is omitted, the default built-in order is used.
- if `modules` is an empty list, only the welcome and farewell text are shown.
- remote welcome uses a local cache and falls back to stale cache or default text on failure.
- `--verbose` shows config loading, module resolution, welcome source, cache state, and external command availability.

## Example Output

```shell
$ motdyn

Welcome!

Host name:         prod-hpc-01
Main NIC:          bond0 (10.10.8.24)
User info:         admin (from 10.10.1.15), 4 user(s) logged in
Current time (TZ): 2026-01-15 09:30:00 +00:00
System uptime:     24 days, 18:42:11
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

Some fields may degrade to `unknown` on minimal systems or containers.
