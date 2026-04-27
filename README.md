<div align=right>Table of Contents↗️</div>

<div align="center">
  <h1><code>motdyn</code></h1>
  <p><strong>Fast, dynamic MOTD output for Linux login sessions.</strong></p>

  <p>
    <a href="https://crates.io/crates/motdyn">
      <img src="https://img.shields.io/crates/v/motdyn.svg" alt="crates.io version">
    </a>
    <a href="https://github.com/lvillis/motdyn/actions">
      <img src="https://github.com/lvillis/motdyn/actions/workflows/ci.yaml/badge.svg" alt="build status">
    </a>
  </p>
</div>

`motdyn` prints a compact host snapshot when a user opens a shell. It is designed for SSH and login-shell paths where output must be useful, fast, and safe to fail.

## Features

- Shows host, network, user, time, uptime, load, OS, kernel, virtualization, CPU, memory, swap, and disk usage.
- Adds root-focused login security summaries for last login and failed login attempts.
- Supports role-aware output: root gets the full view, non-root users get a smaller baseline by default.
- Uses hard timeouts and panic isolation so a probe failure does not block shell startup.
- Provides a slim container build that only renders environment templates.

## Install

```bash
cargo install motdyn
cargo install motdyn --no-default-features --features slim
```

Docker images are published in full and slim variants:

```bash
docker run --rm lvillis/motdyn:latest --help
docker run --rm lvillis/motdyn:slim --help
```

## Usage

Print the current MOTD:

```bash
motdyn
motdyn --compact
motdyn --profile full
motdyn --plain
```

Install or remove login hooks:

```bash
sudo motdyn install
motdyn install --user
motdyn status --user
motdyn uninstall --user
```

Render a template without system probing:

```bash
motdyn --text 'Hello ${USER}, service=${SERVICE_NAME:-unknown}'
motdyn --file ./motd.template --env-prefix APP_
```

Template syntax:

- `${VAR}` reads environment variable `VAR`.
- `${VAR:-default}` uses `default` when the variable is missing or empty.
- `$$` prints a literal `$`.
- `--env-prefix APP_` makes `${NAME}` read `APP_NAME`.

## Build Variants

Default builds use the `full` feature set:

- `template`: local template rendering.
- `color`: ANSI color output.
- `config-toml`: `/etc/motdyn/config.toml` and `~/.config/motdyn/config.toml`.
- `system-probes`: local Linux host facts.
- `login-security`: `lastlog` and `lastb` summaries.
- `remote-welcome`: `file://`, `http`, and `https` welcome sources with cache revalidation.
- `install-hooks`: login shell hook management.

Slim builds compile only the template renderer:

```bash
cargo build --release --no-default-features --features slim
docker build --build-arg 'CARGO_FEATURE_FLAGS=--no-default-features --features slim' -t motdyn:slim .
```

Release artifacts and Docker tags are split as `motdyn-*` and `motdyn-slim-*`, with image tags such as `latest` and `slim`.

## Configuration

System config:

```text
/etc/motdyn/config.toml
```

User config:

```text
~/.config/motdyn/config.toml
```

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

Supported modules:

```text
host network user time uptime load os kernel virtualization cpu memory swap disk last_login failed_login services updates
```

Notes:

- `welcome_sources` are tried in order until one returns usable text.
- `--profile auto` is default: root uses the full built-in view, non-root uses `host`, `network`, `user`, `time`, `uptime`, and `load`.
- Explicit `modules` always override role-based defaults.
- `services` and `updates` are opt-in modules.
- Remote welcome uses a 250 ms default timeout, cache revalidation, and stale-cache fallback.

## Example

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
Resource use:      mem  25% ###-------  disk  16% ##--------
Last login:        4d ago from 10.10.1.15 via ssh
Failed login:      clear

Have a nice day!
```
