//! Parse raw metric output into structured JSON values per shell type.

use std::collections::HashMap;

use crate::config::schema::ShellType;
use serde_json::Value;

/// Parse command output for a given metric and shell type into JSON.
pub fn parse(shell: ShellType, metric: &str, stdout: &str) -> Value {
    match (shell, metric) {
        (ShellType::Sh, "system_info") => parse_sh_system_info(stdout),
        (ShellType::Sh, "cpu_arch") => Value::String(stdout.trim().to_string()),
        (ShellType::Sh, "memory") => parse_sh_memory(stdout),
        (ShellType::Sh, "swap") => parse_sh_swap(stdout),
        (ShellType::Sh, "disk") => parse_sh_disk(stdout),
        (ShellType::Sh, "cpu_load") => parse_sh_cpu_load(stdout),
        (ShellType::Sh, "network") => parse_sh_network(stdout),
        (ShellType::Sh, "battery") => parse_sh_battery(stdout),
        (ShellType::PowerShell, "system_info") => parse_ps_system_info(stdout),
        (ShellType::PowerShell, "cpu_arch") => Value::String(stdout.trim().to_string()),
        (ShellType::PowerShell, "memory") => parse_ps_memory(stdout),
        (ShellType::PowerShell, "swap") => parse_ps_swap(stdout),
        _ => Value::String(stdout.trim().to_string()),
    }
}

/// Parse path size output across shells and formats.
pub fn parse_path_size(shell: ShellType, stdout: &str) -> u64 {
    let trimmed = stdout.trim();

    // 1. Explicit machine-readable marker: `---SIZE:<bytes>`
    if let Some(pos) = trimmed.find("---SIZE:") {
        let rest = &trimmed[pos + "---SIZE:".len()..];
        if let Some(num_str) = rest.split_whitespace().next() {
            if let Ok(val) = num_str.parse::<u64>() {
                return val;
            }
        }
    }

    // 2. Cmd `dir /s /-c` (or `dir /s`) format
    // Summary line looks like:
    // "               2 File(s)           3072 bytes"
    // "               0 File(s)              0 bytes"
    // "               2 File(s)          3,072 bytes"
    if shell == ShellType::Cmd || trimmed.contains("File(s)") {
        for line in trimmed.lines().rev() {
            if line.contains("File(s)") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(bytes_idx) = parts.iter().position(|p| p.eq_ignore_ascii_case("bytes"))
                {
                    if bytes_idx > 0 {
                        let num_str = parts[bytes_idx - 1].replace(',', "");
                        if let Ok(val) = num_str.parse::<u64>() {
                            return val;
                        }
                    }
                }
                for part in parts.iter().rev() {
                    let cleaned = part.replace(',', "");
                    if let Ok(val) = cleaned.parse::<u64>() {
                        return val;
                    }
                }
            }
        }
    }

    // 3. Number alone (e.g. raw PowerShell output)
    if let Ok(val) = trimmed.parse::<u64>() {
        return val;
    }

    // 4. `du -sb` / `du -sk` outputs "SIZE\tPATH"
    trimmed
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Parse batched metric output (split by `---METRIC:name` markers).
/// Returns a map of metric_name → parsed Value.
pub fn parse_batch(shell: ShellType, metrics: &[String], stdout: &str) -> HashMap<String, Value> {
    let mut results = HashMap::new();
    let blocks = split_by_marker(stdout, "---METRIC:");
    for (name, content) in &blocks {
        if metrics.contains(&name.to_string()) {
            results.insert(name.to_string(), parse(shell, name, content));
        }
    }
    results
}

/// Parse batched path size output (split by `---PATH:label` markers).
/// Returns a map of label → Option<size_bytes> (None if MISSING).
pub fn parse_batch_paths(
    shell: ShellType,
    paths: &[(String, String)],
    stdout: &str,
) -> HashMap<String, Option<u64>> {
    let mut results = HashMap::new();
    let blocks = split_by_marker(stdout, "---PATH:");
    let label_map: HashMap<&str, &str> = paths
        .iter()
        .map(|(p, l)| (l.as_str(), p.as_str()))
        .collect();
    for (label, content) in &blocks {
        if label_map.contains_key(label.as_str()) {
            let trimmed = content.trim();
            let is_missing = trimmed == "MISSING"
                || trimmed.is_empty()
                || trimmed.contains("File Not Found")
                || trimmed.contains("The system cannot find the");
            if is_missing {
                results.insert(label.to_string(), None);
            } else {
                results.insert(label.to_string(), Some(parse_path_size(shell, content)));
            }
        }
    }
    results
}

/// Split output by `PREFIX<name>` markers into (name, content) pairs.
fn split_by_marker(output: &str, prefix: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_lines: Vec<&str> = Vec::new();

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix(prefix) {
            // Save previous block
            if let Some(name) = current_name.take() {
                results.push((name, current_lines.join("\n")));
            }
            current_name = Some(rest.trim().to_string());
            current_lines.clear();
        } else if current_name.is_some() {
            current_lines.push(line);
        }
    }
    // Save last block
    if let Some(name) = current_name {
        results.push((name, current_lines.join("\n")));
    }
    results
}

// --- sh parsers ---

fn parse_sh_system_info(stdout: &str) -> Value {
    let lines: Vec<&str> = stdout.lines().collect();
    let mut map = serde_json::Map::new();
    if let Some(uname) = lines.first() {
        map.insert("uname".to_string(), Value::String(uname.trim().to_string()));
        // Try to extract OS from uname -a
        let parts: Vec<&str> = uname.split_whitespace().collect();
        if parts.len() >= 2 {
            map.insert("hostname".to_string(), Value::String(parts[1].to_string()));
        }
        if parts.len() >= 3 {
            map.insert("kernel".to_string(), Value::String(parts[2].to_string()));
        }
    }
    Value::Object(map)
}

fn parse_sh_memory(stdout: &str) -> Value {
    let mut map = serde_json::Map::new();
    // 1. Linux `free -b` output
    for line in stdout.lines() {
        if line.starts_with("Mem:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                if let Ok(total) = parts[1].parse::<u64>() {
                    map.insert("total_bytes".to_string(), Value::Number(total.into()));
                }
                if let Ok(used) = parts[2].parse::<u64>() {
                    map.insert("used_bytes".to_string(), Value::Number(used.into()));
                }
            }
            return Value::Object(map);
        }
    }

    // 2. macOS `sysctl hw.memsize` + `vm_stat`
    let mut total_bytes: Option<u64> = None;
    let mut page_size: u64 = 4096;
    let mut pages_free: u64 = 0;
    let mut pages_speculative: u64 = 0;
    let mut pages_active: u64 = 0;
    let mut pages_inactive: u64 = 0;
    let mut pages_wired: u64 = 0;
    let mut pages_compressed: u64 = 0;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Ok(tb) = trimmed.parse::<u64>() {
            total_bytes = Some(tb);
            continue;
        }
        if trimmed.starts_with("Mach Virtual Memory Statistics:") {
            if let Some(pos) = trimmed.find("page size of ") {
                let rest = &trimmed[pos + "page size of ".len()..];
                if let Some(num_str) = rest.split_whitespace().next() {
                    if let Ok(ps) = num_str.parse::<u64>() {
                        page_size = ps;
                    }
                }
            }
            continue;
        }
        let parse_pages = |prefix: &str| -> Option<u64> {
            if trimmed.starts_with(prefix) {
                let num_part = trimmed.strip_prefix(prefix)?.trim().trim_end_matches('.');
                num_part.parse::<u64>().ok()
            } else {
                None
            }
        };
        if let Some(p) = parse_pages("Pages free:") {
            pages_free = p;
        } else if let Some(p) = parse_pages("Pages speculative:") {
            pages_speculative = p;
        } else if let Some(p) = parse_pages("Pages active:") {
            pages_active = p;
        } else if let Some(p) = parse_pages("Pages inactive:") {
            pages_inactive = p;
        } else if let Some(p) = parse_pages("Pages wired down:") {
            pages_wired = p;
        } else if let Some(p) = parse_pages("Pages occupied by compressor:") {
            pages_compressed = p;
        }
    }

    let total = total_bytes.unwrap_or_else(|| {
        (pages_free
            + pages_speculative
            + pages_active
            + pages_inactive
            + pages_wired
            + pages_compressed)
            .saturating_mul(page_size)
    });
    if total > 0 {
        let free_bytes = (pages_free + pages_speculative).saturating_mul(page_size);
        let used_bytes = total.saturating_sub(free_bytes);
        map.insert("total_bytes".to_string(), Value::Number(total.into()));
        map.insert("used_bytes".to_string(), Value::Number(used_bytes.into()));
    }

    Value::Object(map)
}

fn parse_sh_swap(stdout: &str) -> Value {
    if !stdout.lines().any(|l| l.starts_with("Swap:")) {
        if let Some(map) = parse_bsd_swapusage(stdout) {
            return Value::Object(map);
        }
    }
    let mut map = serde_json::Map::new();
    for line in stdout.lines() {
        if line.starts_with("Swap:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                if let Ok(total) = parts[1].parse::<u64>() {
                    map.insert("total_bytes".to_string(), Value::Number(total.into()));
                }
                if let Ok(used) = parts[2].parse::<u64>() {
                    map.insert("used_bytes".to_string(), Value::Number(used.into()));
                }
            }
        }
    }
    Value::Object(map)
}

/// macOS/BSD `sysctl -n vm.swapusage`: `total = 2048.00M  used = 1034.25M
/// free = 1013.75M  (encrypted)` (B73).
fn parse_bsd_swapusage(stdout: &str) -> Option<serde_json::Map<String, Value>> {
    let field = |name: &str| -> Option<u64> {
        let rest = stdout.split(&format!("{name} = ")).nth(1)?;
        let token = rest.split_whitespace().next()?;
        let (num, unit) = token.split_at(token.find(|c: char| c.is_ascii_alphabetic())?);
        let scale: f64 = match unit {
            "K" => 1024.0,
            "M" => 1024.0 * 1024.0,
            "G" => 1024.0 * 1024.0 * 1024.0,
            _ => return None,
        };
        Some((num.parse::<f64>().ok()? * scale).round() as u64)
    };
    let mut map = serde_json::Map::new();
    map.insert("total_bytes".to_string(), field("total")?.into());
    map.insert("used_bytes".to_string(), field("used")?.into());
    Some(map)
}

fn parse_sh_disk(stdout: &str) -> Value {
    let mut disks = Vec::new();
    let lines: Vec<&str> = stdout.lines().collect();
    if lines.is_empty() {
        return Value::Array(disks);
    }
    let header = lines[0];
    let multiplier: u64 = if header.contains("1024-blocks") || header.contains("1K-blocks") {
        1024
    } else if header.contains("512-blocks") {
        512
    } else {
        1
    };

    for line in lines.iter().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 6 {
            let mut entry = serde_json::Map::new();
            if let Ok(total) = parts[1].parse::<u64>() {
                entry.insert(
                    "total_bytes".to_string(),
                    Value::Number((total.saturating_mul(multiplier)).into()),
                );
            }
            if let Ok(used) = parts[2].parse::<u64>() {
                entry.insert(
                    "used_bytes".to_string(),
                    Value::Number((used.saturating_mul(multiplier)).into()),
                );
            }
            entry.insert(
                "mount".to_string(),
                Value::String(parts.last().unwrap_or(&"").to_string()),
            );
            disks.push(Value::Object(entry));
        }
    }
    Value::Array(disks)
}

fn parse_sh_cpu_load(stdout: &str) -> Value {
    let mut map = serde_json::Map::new();
    // Linux /proc/loadavg: "0.52 0.38 0.21 1/234 5678"
    // macOS sysctl vm.loadavg: "{ 6.94 7.49 6.58 }"
    let cleaned = stdout
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim();
    let parts: Vec<&str> = cleaned.split_whitespace().collect();
    if parts.len() >= 3 {
        if let Ok(v) = parts[0].parse::<f64>() {
            if let Some(n) = serde_json::Number::from_f64(v) {
                map.insert("load1".to_string(), Value::Number(n));
            }
        }
        if let Ok(v) = parts[1].parse::<f64>() {
            if let Some(n) = serde_json::Number::from_f64(v) {
                map.insert("load5".to_string(), Value::Number(n));
            }
        }
        if let Ok(v) = parts[2].parse::<f64>() {
            if let Some(n) = serde_json::Number::from_f64(v) {
                map.insert("load15".to_string(), Value::Number(n));
            }
        }
    }
    Value::Object(map)
}

fn parse_sh_network(stdout: &str) -> Value {
    Value::String(stdout.trim().to_string())
}

fn parse_sh_battery(stdout: &str) -> Value {
    let mut map = serde_json::Map::new();
    let trimmed = stdout.trim();
    if trimmed.is_empty()
        || trimmed.contains("No such file")
        || trimmed.contains("not found")
        || (trimmed.starts_with("Now drawing from") && !trimmed.contains("InternalBattery"))
    {
        map.insert("present".to_string(), Value::Bool(false));
        return Value::Object(map);
    }

    // Try single number (Linux /sys/class/power_supply/BAT0/capacity)
    if let Ok(v) = trimmed.parse::<u64>() {
        map.insert("present".to_string(), Value::Bool(true));
        map.insert("percent".to_string(), Value::Number(v.into()));
        return Value::Object(map);
    }

    // Try percentage anywhere in output (e.g. "85%;" or "85%")
    for part in trimmed.split(|c: char| c.is_whitespace() || c == ';' || c == ',') {
        if let Some(num_str) = part.strip_suffix('%') {
            if let Ok(v) = num_str.parse::<u64>() {
                map.insert("present".to_string(), Value::Bool(true));
                map.insert("percent".to_string(), Value::Number(v.into()));
                return Value::Object(map);
            }
        }
    }

    if trimmed.contains("InternalBattery") || trimmed.contains("BAT") {
        map.insert("present".to_string(), Value::Bool(true));
    } else {
        map.insert("present".to_string(), Value::Bool(false));
    }
    Value::Object(map)
}

// --- PowerShell parsers ---

fn parse_ps_system_info(stdout: &str) -> Value {
    Value::String(stdout.trim().to_string())
}

fn parse_ps_memory(stdout: &str) -> Value {
    Value::String(stdout.trim().to_string())
}

pub(crate) fn ps_swap_totals(val: &Value) -> Option<(u64, u64)> {
    if let Some(arr) = val.as_array() {
        let mut total = 0u64;
        let mut used = 0u64;
        for item in arr {
            total += item
                .get("AllocatedBaseSize")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
            used += item
                .get("CurrentUsage")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
        }
        if total > 0 {
            Some((total, used))
        } else {
            None
        }
    } else if let Some(obj) = val.as_object() {
        let total = obj.get("AllocatedBaseSize").and_then(|n| n.as_u64())?;
        let used = obj.get("CurrentUsage").and_then(|n| n.as_u64())?;
        if total > 0 {
            Some((total, used))
        } else {
            None
        }
    } else {
        None
    }
}

fn parse_ps_swap(stdout: &str) -> Value {
    let trimmed = stdout.trim();
    if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
        if let Some((total_mb, used_mb)) = ps_swap_totals(&val) {
            let mut map = serde_json::Map::new();
            map.insert(
                "total_bytes".to_string(),
                Value::Number((total_mb * 1024 * 1024).into()),
            );
            map.insert(
                "used_bytes".to_string(),
                Value::Number((used_mb * 1024 * 1024).into()),
            );
            return Value::Object(map);
        }
    }
    Value::String(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_batch_splits_metrics() {
        let output = "---METRIC:online\nok\n---METRIC:cpu_arch\nx86_64\n";
        let metrics = vec!["online".into(), "cpu_arch".into()];
        let result = parse_batch(ShellType::Sh, &metrics, output);
        assert_eq!(result.len(), 2);
        assert!(result.contains_key("online"));
        assert!(result.contains_key("cpu_arch"));
        assert_eq!(result["cpu_arch"], Value::String("x86_64".into()));
    }

    #[test]
    fn test_parse_batch_empty_output() {
        let result = parse_batch(ShellType::Sh, &["online".into()], "");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_batch_ignores_unknown_metrics() {
        let output = "---METRIC:online\nok\n---METRIC:unknown_thing\nfoo\n";
        let metrics = vec!["online".into()];
        let result = parse_batch(ShellType::Sh, &metrics, output);
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("online"));
    }

    #[test]
    fn test_parse_batch_paths_ok() {
        let output = "---PATH:home\n12345\t/home\n---PATH:logs\nMISSING\n";
        let paths = vec![("~/".into(), "home".into()), ("/var".into(), "logs".into())];
        let result = parse_batch_paths(ShellType::Sh, &paths, output);
        assert_eq!(result.len(), 2);
        assert_eq!(*result.get("home").unwrap(), Some(12345u64));
        assert_eq!(*result.get("logs").unwrap(), None);
    }

    #[test]
    fn test_parse_batch_paths_empty() {
        let result = parse_batch_paths(ShellType::Sh, &[], "");
        assert!(result.is_empty());
    }

    #[test]
    fn test_split_by_marker() {
        let output = "---METRIC:a\nline1\nline2\n---METRIC:b\nline3\n";
        let result = split_by_marker(output, "---METRIC:");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "a");
        assert!(result[0].1.contains("line1"));
        assert_eq!(result[1].0, "b");
        assert!(result[1].1.contains("line3"));
    }

    #[test]
    fn test_parse_ps_swap_single() {
        let output = r#"{"AllocatedBaseSize": 4096, "CurrentUsage": 1024}"#;
        let result = parse(ShellType::PowerShell, "swap", output);
        assert_eq!(
            result.get("total_bytes").and_then(|v| v.as_u64()),
            Some(4096 * 1024 * 1024)
        );
        assert_eq!(
            result.get("used_bytes").and_then(|v| v.as_u64()),
            Some(1024 * 1024 * 1024)
        );
    }

    #[test]
    fn test_parse_ps_swap_array() {
        let output = r#"[{"AllocatedBaseSize": 2048, "CurrentUsage": 512}, {"AllocatedBaseSize": 2048, "CurrentUsage": 512}]"#;
        let result = parse(ShellType::PowerShell, "swap", output);
        assert_eq!(
            result.get("total_bytes").and_then(|v| v.as_u64()),
            Some(4096 * 1024 * 1024)
        );
        assert_eq!(
            result.get("used_bytes").and_then(|v| v.as_u64()),
            Some(1024 * 1024 * 1024)
        );
    }

    #[test]
    fn test_parse_ps_swap_empty() {
        let result = parse(ShellType::PowerShell, "swap", "");
        assert_eq!(result, Value::String("".to_string()));
    }

    /// B73: macOS/BSD swap comes from `sysctl vm.swapusage` (no `free`).
    #[test]
    fn test_parse_macos_swap_fixtures() {
        let raw = include_str!("../../tests/fixtures/probes/macos_swap.txt");
        let val = parse(ShellType::Sh, "swap", raw);
        assert_eq!(val["total_bytes"], serde_json::json!(2048u64 * 1024 * 1024));
        assert_eq!(val["used_bytes"], serde_json::json!(1_084_489_728u64));

        let none = include_str!("../../tests/fixtures/probes/macos_swap_none.txt");
        let val = parse(ShellType::Sh, "swap", none);
        assert_eq!(val["total_bytes"], serde_json::json!(0));
        assert_eq!(val["used_bytes"], serde_json::json!(0));

        // Linux `free -b` still wins when present.
        let linux =
            "              total        used        free\nMem:  8 4 4\nSwap:  2048 1024 1024\n";
        let val = parse(ShellType::Sh, "swap", linux);
        assert_eq!(val["total_bytes"], serde_json::json!(2048));
        assert_eq!(val["used_bytes"], serde_json::json!(1024));
    }

    #[test]
    fn test_parse_macos_and_linux_cpu_load_fixtures() {
        let macos_raw = include_str!("../../tests/fixtures/probes/macos_cpu_load.txt");
        let val = parse(ShellType::Sh, "cpu_load", macos_raw);
        assert_eq!(val["load1"], serde_json::json!(6.94));
        assert_eq!(val["load5"], serde_json::json!(7.49));
        assert_eq!(val["load15"], serde_json::json!(6.58));

        let linux_raw = include_str!("../../tests/fixtures/probes/linux_cpu_load.txt");
        let val = parse(ShellType::Sh, "cpu_load", linux_raw);
        assert_eq!(val["load1"], serde_json::json!(0.52));
        assert_eq!(val["load5"], serde_json::json!(0.38));
        assert_eq!(val["load15"], serde_json::json!(0.21));
    }

    #[test]
    fn test_parse_macos_and_linux_disk_fixtures() {
        let macos_raw = include_str!("../../tests/fixtures/probes/macos_disk.txt");
        let val = parse(ShellType::Sh, "disk", macos_raw);
        let arr = val.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        // 239362496 * 1024 = 245107195904 bytes
        assert_eq!(arr[0]["total_bytes"], serde_json::json!(245107195904u64));
        assert_eq!(arr[0]["used_bytes"], serde_json::json!(59285569536u64));
        assert_eq!(arr[0]["mount"], "/");

        let linux_raw = include_str!("../../tests/fixtures/probes/linux_disk.txt");
        let val = parse(ShellType::Sh, "disk", linux_raw);
        let arr = val.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["total_bytes"], serde_json::json!(10485760000u64));
        assert_eq!(arr[0]["used_bytes"], serde_json::json!(4194304000u64));
        assert_eq!(arr[0]["mount"], "/");
    }

    #[test]
    fn test_parse_macos_and_linux_memory_fixtures() {
        let macos_raw = include_str!("../../tests/fixtures/probes/macos_memory.txt");
        let val = parse(ShellType::Sh, "memory", macos_raw);
        let total = val["total_bytes"].as_u64().expect("total_bytes");
        let used = val["used_bytes"].as_u64().expect("used_bytes");
        assert_eq!(total, 17179869184); // 16 GB from hw.memsize
                                        // free = (118193 + 80140) * 16384 = 3249491968; used = 17179869184 - 3249491968 = 13930381312
        assert_eq!(used, 13930381312);

        let linux_raw = include_str!("../../tests/fixtures/probes/linux_memory.txt");
        let val = parse(ShellType::Sh, "memory", linux_raw);
        assert_eq!(val["total_bytes"], serde_json::json!(16777216000u64));
        assert_eq!(val["used_bytes"], serde_json::json!(8388608000u64));
    }

    #[test]
    fn test_parse_macos_and_linux_battery_fixtures() {
        let mac_laptop = include_str!("../../tests/fixtures/probes/macos_battery_laptop.txt");
        let val = parse(ShellType::Sh, "battery", mac_laptop);
        assert_eq!(val["present"], serde_json::json!(true));
        assert_eq!(val["percent"], serde_json::json!(85));

        let mac_desktop = include_str!("../../tests/fixtures/probes/macos_battery_desktop.txt");
        let val = parse(ShellType::Sh, "battery", mac_desktop);
        assert_eq!(val["present"], serde_json::json!(false));

        let linux_bat = include_str!("../../tests/fixtures/probes/linux_battery.txt");
        let val = parse(ShellType::Sh, "battery", linux_bat);
        assert_eq!(val["present"], serde_json::json!(true));
        assert_eq!(val["percent"], serde_json::json!(85));

        let linux_missing = include_str!("../../tests/fixtures/probes/linux_battery_missing.txt");
        let val = parse(ShellType::Sh, "battery", linux_missing);
        assert_eq!(val["present"], serde_json::json!(false));
    }

    #[test]
    fn test_parse_macos_and_linux_path_size_fixtures() {
        let macos_raw = include_str!("../../tests/fixtures/probes/macos_path_size.txt");
        let size = parse_path_size(ShellType::Sh, macos_raw);
        assert_eq!(size, 4096);

        let linux_raw = include_str!("../../tests/fixtures/probes/linux_path_size.txt");
        let size = parse_path_size(ShellType::Sh, linux_raw);
        assert_eq!(size, 4096);
    }

    #[test]
    fn test_parse_cmd_path_size_fixtures() {
        let cmd_files = include_str!("../../tests/fixtures/probes/cmd_dir_files.txt");
        assert_eq!(parse_path_size(ShellType::Cmd, cmd_files), 3072);

        let cmd_empty = include_str!("../../tests/fixtures/probes/cmd_dir_empty.txt");
        assert_eq!(parse_path_size(ShellType::Cmd, cmd_empty), 0);

        let cmd_commas = include_str!("../../tests/fixtures/probes/cmd_dir_commas.txt");
        assert_eq!(parse_path_size(ShellType::Cmd, cmd_commas), 3072);

        let paths = vec![
            ("C:\\test".into(), "test".into()),
            ("C:\\empty".into(), "empty".into()),
            ("C:\\missing".into(), "missing".into()),
        ];
        let stdout = format!(
            "---PATH:test\n{}\n---PATH:empty\n{}\n---PATH:missing\n{}\n",
            cmd_files,
            cmd_empty,
            include_str!("../../tests/fixtures/probes/cmd_dir_missing.txt")
        );
        let results = parse_batch_paths(ShellType::Cmd, &paths, &stdout);
        assert_eq!(results.get("test"), Some(&Some(3072)));
        assert_eq!(results.get("empty"), Some(&Some(0))); // Empty directory is 0, NOT None
        assert_eq!(results.get("missing"), Some(&None));
    }

    #[test]
    fn test_parse_powershell_path_size_fixtures() {
        let ps_files = include_str!("../../tests/fixtures/probes/powershell_path_files.txt");
        assert_eq!(parse_path_size(ShellType::PowerShell, ps_files), 3072);

        let ps_empty = include_str!("../../tests/fixtures/probes/powershell_path_empty.txt");
        assert_eq!(parse_path_size(ShellType::PowerShell, ps_empty), 0);

        let ps_raw = include_str!("../../tests/fixtures/probes/powershell_path_raw_num.txt");
        assert_eq!(parse_path_size(ShellType::PowerShell, ps_raw), 3072);

        let paths = vec![
            ("C:\\test".into(), "test".into()),
            ("C:\\empty".into(), "empty".into()),
            ("C:\\missing".into(), "missing".into()),
        ];
        let stdout = format!(
            "---PATH:test\n{}\n---PATH:empty\n{}\n---PATH:missing\n{}\n",
            ps_files,
            ps_empty,
            include_str!("../../tests/fixtures/probes/powershell_path_missing.txt")
        );
        let results = parse_batch_paths(ShellType::PowerShell, &paths, &stdout);
        assert_eq!(results.get("test"), Some(&Some(3072)));
        assert_eq!(results.get("empty"), Some(&Some(0))); // Empty directory is 0, NOT None
        assert_eq!(results.get("missing"), Some(&None));
    }
}
