<div align=right>Table of Contents↗️</div>

<h1 align=center><code>motdyn</code></h1>

<p align=center>🛠️ motdyn (Dynamic MOTD): Rust-powered and highly configurable for real-time login insights.</p>

<div align=center>
  <a href="https://crates.io/crates/motdyn">
    <img src="https://img.shields.io/crates/v/motdyn.svg" alt="crates.io version">
  </a>
  <a href="https://crates.io/crates/motdyn">
    <img src="https://img.shields.io/github/repo-size/lvillis/motdyn?style=flat-square&color=328657" alt="crates.io version">
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
  <a href="mailto:lvillis@outlook.com?subject=Thanks%20for%20motdyn!">
    <img src="https://img.shields.io/badge/Say%20Thanks-!-1EAEDB.svg" alt="say thanks">
  </a>

</div>

---

## Example

```shell
$ motdyn

Welcome!

Current time (TZ): 2024-12-27 22:36:25 +00:00
System uptime:     80 days, 05:13:42
Operating system:  Rocky Linux 9.5 (Blue Onyx)
Kernel version:    5.14.0-503.15.1.el9_5.x86_64
Host name:         localhost
CPU:               AMD EPYC 7313 16-Core Processor (32 cores)
Memory used/total: 100.67/127.77 GB (78.79%)
Swap used/total:   0.00/0.00 GB (0.00%)
Current user:      root (from 192.168.0.1)
Login user count:  11
Disk usage (root): / => 43.61 GB/98.40 GB (44.32%)
Disk usage (nfs):  /mnt => 1.51 TB/1.97 TB (76.51%)

Have a nice day!
```

## Usage

```bash
motdyn [subcommand] [options]
```

Where:

- `install` places a script (/etc/profile.d/motdyn.sh) to run motdyn on login (root privileges).
- `uninstall` removes that script, stopping MOTD from running automatically.
- `status` checks if the script is currently installed.
- (no subcommand) prints the dynamic MOTD immediately.
- -v, --verbose shows additional details.