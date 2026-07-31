//! System metrics collection.
//!
//! Mirrors `pi_dashboard/metrics.py`.
//! - Fast path (`/proc`, `/sys`) is called synchronously each frame.
//! - Slow path (`hostname`, `docker`, `tailscale`) runs in a background tokio
//!   task and publishes a `watch::Sender<MetricsSnapshot>`.

use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{interval, timeout};
use tracing::{debug, warn};

use crate::config::{
    CPU_SMOOTH_WINDOW, IP_FILTER_ENABLED, SLOW_DATA_INTERVAL,
};

#[derive(Clone, Default, Debug)]
pub struct ContainerInfo {
    pub name: String,
    pub id: String,
    pub status: String,
    pub state: String,
    pub cpu: Option<f32>,
}

/// CPU sampler for Docker containers via cgroup v2 `cpu.stat`.
/// Holds the last (timestamp, usage_usec) per container name.
#[derive(Default)]
struct ContainerCpuSampler {
    last: HashMap<String, (std::time::Instant, u64)>,
}

impl ContainerCpuSampler {
    fn new() -> Self {
        Self {
            last: HashMap::new(),
        }
    }

    fn compute(&mut self, name: &str, id: &str, now: std::time::Instant) -> Option<f32> {
        let usage = Self::read_usage_usec(id)?;
        let pct = if let Some((prev_time, prev_usage)) = self.last.get(name) {
            let dt = now.duration_since(*prev_time).as_micros() as f64;
            let du = usage.saturating_sub(*prev_usage) as f64;
            if dt > 0.0 {
                Some((100.0 * du / dt) as f32)
            } else {
                None
            }
        } else {
            None
        };
        self.last.insert(name.to_string(), (now, usage));
        pct
    }

    fn read_usage_usec(id: &str) -> Option<u64> {
        let path = format!("/sys/fs/cgroup/system.slice/docker-{}.scope/cpu.stat", id);
        let text = std::fs::read_to_string(&path).ok()?;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("usage_usec ") {
                return v.parse::<u64>().ok();
            }
        }
        None
    }
}

/// Abbreviate a docker `Status` string for the dashboard container list.
/// Parenthetical health suffixes `(healthy)` / `(unhealthy)` are stripped from
/// the returned text; callers should detect `(unhealthy)` separately for the
/// STATE column colour.
pub fn abbreviate_status(status: &str) -> String {
    let s = status
        .replace(" (healthy)", "")
        .replace("(healthy)", "")
        .replace(" (unhealthy)", "")
        .replace("(unhealthy)", "");
    let s = s.trim();

    if s == "Created" {
        return "New".to_string();
    }
    if s == "Paused" {
        return "Paus".to_string();
    }

    if s == "Up About an hour" || s == "Up about an hour" {
        return "1h".to_string();
    }
    if s == "Up Less than a second" || s == "Up less than a second" {
        return "0s".to_string();
    }

    if let Some(rest) = s.strip_prefix("Up ") {
        if let Some((num, unit)) = parse_duration(rest) {
            return format!("{}{}", num, unit);
        }
    }

    if let Some(rest) = s.strip_prefix("Exited ") {
        // Expected: "(0) 3 hours ago"
        if let Some(after_paren) = rest.split_once(')').map(|(_, r)| r.trim()) {
            let code = rest
                .trim_start_matches('(')
                .split_once(')')
                .map(|(c, _)| c)
                .unwrap_or("?");
            if let Some((num, unit)) = parse_duration(after_paren) {
                return format!("Ex{} {}{}", code, num, unit);
            }
        }
    }

    if s.starts_with("Restarting ") {
        // Expected: "Restarting (1) 5 seconds ago"
        if let Some((_, after_paren)) = s.split_once(')') {
            if let Some((num, unit)) = parse_duration(after_paren.trim()) {
                return format!("Rst {}{}", num, unit);
            }
        }
    }

    s.to_string()
}

/// Parse a docker duration fragment such as "45 seconds" or "3 hours ago".
/// Returns the numeric value and a one/two-letter unit suffix.
fn parse_duration(text: &str) -> Option<(u32, &'static str)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 2 {
        return None;
    }
    let num = words[0].parse::<u32>().ok()?;
    let unit = words[1].to_ascii_lowercase();
    let suffix = if unit.starts_with("second") {
        "s"
    } else if unit.starts_with("minute") {
        "m"
    } else if unit.starts_with("hour") {
        "h"
    } else if unit.starts_with("day") {
        "d"
    } else if unit.starts_with("week") {
        "w"
    } else if unit.starts_with("month") {
        "mo"
    } else if unit.starts_with("year") {
        "y"
    } else {
        return None;
    };
    Some((num, suffix))
}

#[derive(Clone, Default, Debug)]
pub struct MetricsSnapshot {
    pub ips: Vec<String>,
    pub tailscale: String,
    pub containers: Vec<ContainerInfo>,
    pub disk: String,
    pub temp: String,
    pub mem: String,
    pub cpu: Vec<(String, f32)>,
}

/// CPU usage sampler with sliding-window smoothing.
pub struct CpuSampler {
    prev: Option<HashMap<String, (u64, u64)>>,
    history: HashMap<String, VecDeque<f32>>,
}

impl CpuSampler {
    pub fn new() -> Self {
        Self {
            prev: None,
            history: HashMap::new(),
        }
    }

    /// Sample `/proc/stat` once and return `(total, idle)` per core.
    fn sample() -> Option<HashMap<String, (u64, u64)>> {
        let text = std::fs::read_to_string("/proc/stat").ok()?;
        let mut stats = HashMap::new();
        for line in text.lines() {
            if !line.starts_with("cpu") {
                break;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 {
                continue;
            }
            let core = parts[0];
            // Skip the aggregate "cpu" line; keep cpuN lines only.
            if core == "cpu" {
                continue;
            }
            if !core[3..].chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let values: Vec<u64> = parts[1..]
                .iter()
                .filter_map(|s| s.parse::<u64>().ok())
                .collect();
            if values.len() < 5 {
                continue;
            }
            let total: u64 = values.iter().sum();
            // idle = values[3], iowait = values[4]
            let idle = values[3] + values[4];
            stats.insert(core.to_string(), (total, idle));
        }
        Some(stats)
    }

    /// Read current smoothed CPU usage per core, sorted by core index.
    pub fn read(&mut self) -> Vec<(String, f32)> {
        let curr = match Self::sample() {
            Some(c) => c,
            None => return Vec::new(),
        };

        let mut raw: HashMap<String, f32> = HashMap::new();
        if let Some(prev) = self.prev.take() {
            for (core, (c_total, c_idle)) in &curr {
                if let Some((p_total, p_idle)) = prev.get(core) {
                    let total_diff = c_total.saturating_sub(*p_total);
                    let idle_diff = c_idle.saturating_sub(*p_idle);
                    let usage = if total_diff > 0 {
                        100.0 * (1.0 - idle_diff as f32 / total_diff as f32)
                    } else {
                        0.0
                    };
                    raw.insert(core.clone(), usage.clamp(0.0, 100.0));
                } else {
                    raw.insert(core.clone(), 0.0);
                }
            }
        } else {
            for core in curr.keys() {
                raw.insert(core.clone(), 0.0);
            }
        }

        self.prev = Some(curr);

        let mut results: Vec<(String, f32)> = Vec::new();
        for (core, usage) in raw {
            let hist = self.history.entry(core.clone()).or_insert_with(|| {
                VecDeque::with_capacity(CPU_SMOOTH_WINDOW)
            });
            hist.push_back(usage);
            while hist.len() > CPU_SMOOTH_WINDOW {
                hist.pop_front();
            }
            let avg = hist.iter().sum::<f32>() / hist.len().max(1) as f32;
            results.push((core, avg));
        }

        results.sort_by(|a, b| {
            let an: u32 = a.0[3..].parse().unwrap_or(0);
            let bn: u32 = b.0[3..].parse().unwrap_or(0);
            an.cmp(&bn)
        });
        results
    }
}

pub fn read_cpu_temp() -> String {
    match std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp") {
        Ok(text) => match text.trim().parse::<i64>() {
            Ok(v) => format!("{:.0}C", v as f32 / 1000.0),
            Err(_) => "N/A".to_string(),
        },
        Err(_) => "N/A".to_string(),
    }
}

pub fn read_mem_info() -> String {
    let text = match std::fs::read_to_string("/proc/meminfo") {
        Ok(t) => t,
        Err(_) => return "N/A".to_string(),
    };
    let mut mem: HashMap<String, u64> = HashMap::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let val = v
            .trim()
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        mem.insert(k.trim().to_string(), val);
    }
    let total = mem.get("MemTotal").copied().unwrap_or(0);
    if total == 0 {
        return "N/A".to_string();
    }
    let available = mem
        .get("MemAvailable")
        .copied()
        .or_else(|| mem.get("MemFree").copied())
        .unwrap_or(0);
    let used = total.saturating_sub(available);
    let pct = 100.0 * used as f32 / total as f32;
    format!("{}/{:.0}MB ({:.0}%)", used / 1024, total as f32 / 1024.0, pct)
}

async fn read_disk_usage() -> String {
    let out = match run_command("df", &["--output=pcent", "/"], 3.0).await {
        Some(o) => o,
        None => return "N/A".to_string(),
    };
    out.lines()
        .nth(1)
        .and_then(|line| {
            line.trim()
                .trim_end_matches('%')
                .parse::<f32>()
                .ok()
                .map(|pct| format!("{:.0}%", pct))
        })
        .unwrap_or_else(|| "N/A".to_string())
}

async fn run_command(cmd: &str, args: &[&str], secs: f32) -> Option<String> {
    match timeout(Duration::from_secs_f32(secs), Command::new(cmd).args(args).output()).await {
        Ok(Ok(out)) if out.status.success() => {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        Ok(Ok(out)) => {
            debug!("Command {} exited with {:?}", cmd, out.status);
            None
        }
        Ok(Err(e)) => {
            warn!("Command {} failed: {}", cmd, e);
            None
        }
        Err(_) => {
            warn!("Command {} timed out after {}s", cmd, secs);
            None
        }
    }
}

pub async fn get_ip_list() -> Vec<String> {
    let out = match run_command("hostname", &["-I"], 5.0).await {
        Some(o) if !o.is_empty() => o,
        _ => return vec!["No IP".to_string()],
    };
    let mut ips: Vec<String> = out
        .split_whitespace()
        .filter(|ip| !ip.starts_with("127."))
        .map(|s| s.to_string())
        .collect();
    if IP_FILTER_ENABLED {
        ips.retain(|ip| ip.starts_with("192.") || ip.starts_with("10.") || ip.starts_with("100."));
    }
    if ips.is_empty() {
        ips.push("No IP".to_string());
    }
    ips
}

pub async fn read_docker_containers() -> Vec<ContainerInfo> {
    let out = match run_command(
        "docker",
        &["ps", "-a", "--no-trunc", "--format", "{{.Names}}|{{.ID}}|{{.Status}}|{{.State}}"],
        3.0,
    )
    .await
    {
        Some(o) => o,
        None => return Vec::new(),
    };
    let mut containers = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() != 4 {
            continue;
        }
        containers.push(ContainerInfo {
            name: parts[0].to_string(),
            id: parts[1].to_string(),
            status: parts[2].to_string(),
            state: parts[3].to_string(),
            cpu: None,
        });
    }
    containers
}

/// Global container CPU sampler. Since metrics runs on a single tokio
/// current_thread runtime, a Mutex is sufficient and contention-free.
static CONTAINER_CPU_SAMPLER: LazyLock<std::sync::Mutex<ContainerCpuSampler>> =
    LazyLock::new(|| std::sync::Mutex::new(ContainerCpuSampler::new()));

/// Join freshly fetched container list with CPU data read from cgroup v2.
/// Falls back to the previous sample's CPU if cgroup is not yet readable.
pub async fn read_docker_containers_with_cpu(prev: &[ContainerInfo]) -> Vec<ContainerInfo> {
    let mut containers = read_docker_containers().await;
    let now = std::time::Instant::now();
    let mut sampler = match CONTAINER_CPU_SAMPLER.lock() {
        Ok(g) => g,
        Err(_) => {
            // Lock poisoned: fall back to stale values.
            let prev_map: HashMap<String, f32> = prev
                .iter()
                .filter_map(|c| c.cpu.map(|cpu| (c.name.clone(), cpu)))
                .collect();
            for c in &mut containers {
                c.cpu = prev_map.get(&c.name).copied();
            }
            return containers;
        }
    };
    for c in &mut containers {
        c.cpu = sampler.compute(&c.name, &c.id, now);
    }
    containers
}

pub async fn read_tailscale_status() -> String {
    match run_command("tailscale", &["status", "--json"], 5.0).await {
        Some(out) if out.contains("\"BackendState\": \"Running\"") => "ON".to_string(),
        Some(_) => "OFF".to_string(),
        None => "OFF".to_string(),
    }
}

pub struct Metrics {
    rx: watch::Receiver<MetricsSnapshot>,
    sampler: std::sync::Mutex<CpuSampler>,
}

impl Metrics {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(MetricsSnapshot::default());

        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs_f32(SLOW_DATA_INTERVAL));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            // First refresh immediately.
            let mut snap = Self::refresh_slow(&MetricsSnapshot::default()).await;
            let _ = tx.send(snap.clone());

            loop {
                ticker.tick().await;
                snap = Self::refresh_slow(&snap).await;
                let _ = tx.send(snap.clone());
            }
        });

        Self {
            rx,
            sampler: std::sync::Mutex::new(CpuSampler::new()),
        }
    }

    async fn refresh_slow(prev: &MetricsSnapshot) -> MetricsSnapshot {
        let (ips, tailscale, containers, disk) = tokio::join!(
            get_ip_list(),
            read_tailscale_status(),
            read_docker_containers_with_cpu(&prev.containers),
            read_disk_usage(),
        );
        MetricsSnapshot {
            ips,
            tailscale,
            containers,
            disk,
            temp: String::new(),
            mem: String::new(),
            cpu: Vec::new(),
        }
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        self.rx.borrow().clone()
    }

    pub fn cpu(&self) -> Vec<(String, f32)> {
        match self.sampler.lock() {
            Ok(mut s) => s.read(),
            Err(_) => Vec::new(),
        }
    }

    pub fn temp(&self) -> String {
        read_cpu_temp()
    }

    pub fn mem(&self) -> String {
        read_mem_info()
    }

    pub fn disk(&self) -> String {
        self.snapshot().disk
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_temp_format() {
        let t = read_cpu_temp();
        assert!(t.ends_with('C') || t == "N/A", "unexpected temp: {}", t);
    }

    #[test]
    fn mem_info_format() {
        let m = read_mem_info();
        assert!(m.contains("MB") || m == "N/A", "unexpected mem: {}", m);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disk_usage_format() {
        let d = read_disk_usage().await;
        assert!(d.ends_with('%') || d == "N/A", "unexpected disk: {}", d);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ip_list_non_empty() {
        let ips = get_ip_list().await;
        assert!(!ips.is_empty(), "ip list empty");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tailscale_on_or_off() {
        let ts = read_tailscale_status().await;
        assert!(ts == "ON" || ts == "OFF", "unexpected tailscale: {}", ts);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cpu_sampler_returns_cores() {
        let mut sampler = CpuSampler::new();
        let _ = sampler.read();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let cores = sampler.read();
        assert!(!cores.is_empty(), "no cpu cores sampled");
        for (name, pct) in &cores {
            assert!(name.starts_with("cpu"));
            assert!(*pct >= 0.0 && *pct <= 100.0);
        }
    }

    #[test]
    fn abbreviate_status_up_units() {
        assert_eq!(abbreviate_status("Up 45 seconds"), "45s");
        assert_eq!(abbreviate_status("Up 1 second"), "1s");
        assert_eq!(abbreviate_status("Up 5 minutes"), "5m");
        assert_eq!(abbreviate_status("Up 2 hours"), "2h");
        assert_eq!(abbreviate_status("Up 3 days"), "3d");
        assert_eq!(abbreviate_status("Up 1 weeks"), "1w");
        assert_eq!(abbreviate_status("Up 2 months"), "2mo");
        assert_eq!(abbreviate_status("Up 1 years"), "1y");
    }

    #[test]
    fn abbreviate_status_special_and_health() {
        assert_eq!(abbreviate_status("Up About an hour"), "1h");
        assert_eq!(abbreviate_status("Up Less than a second"), "0s");
        assert_eq!(abbreviate_status("Up 2 hours (healthy)"), "2h");
        assert_eq!(abbreviate_status("Up 2 hours (unhealthy)"), "2h");
    }

    #[test]
    fn abbreviate_status_exited_and_restarting() {
        assert_eq!(abbreviate_status("Exited (0) 3 hours ago"), "Ex0 3h");
        assert_eq!(abbreviate_status("Exited (137) 5 minutes ago"), "Ex137 5m");
        assert_eq!(abbreviate_status("Restarting (1) 5 seconds ago"), "Rst 5s");
    }

    #[test]
    fn abbreviate_status_states_and_fallback() {
        assert_eq!(abbreviate_status("Created"), "New");
        assert_eq!(abbreviate_status("Paused"), "Paus");
        assert_eq!(abbreviate_status("Unknown weird state"), "Unknown weird state");
    }
}
