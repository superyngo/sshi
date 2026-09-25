//! CMD probe commands for system metrics (minimal support).

/// Return the CMD probe command for a given metric.
/// CMD support is minimal / nice-to-have.
pub fn command_for(metric: &str) -> String {
    match metric {
        "online" => "echo ok".to_string(),
        "system_info" => "systeminfo".to_string(),
        "cpu_arch" => "wmic cpu get AddressWidth /value".to_string(),
        "memory" => "wmic OS get FreePhysicalMemory,TotalVisibleMemorySize /value".to_string(),
        "swap" => "wmic pagefile get AllocatedBaseSize,CurrentUsage /value".to_string(),
        "disk" => "wmic logicaldisk get size,freespace,caption /value".to_string(),
        "cpu_load" => "wmic cpu get LoadPercentage /value".to_string(),
        "network" => "ipconfig".to_string(),
        "battery" => "wmic path Win32_Battery get EstimatedChargeRemaining /value".to_string(),
        "ip_address" => {
            "for /f \"tokens=2 delims=:\" %a in ('ipconfig ^| findstr /i \"IPv4\"') do @echo %a"
                .to_string()
        }
        _ => String::new(),
    }
}

/// Build a single CMD command that collects all metrics with `---METRIC:` markers.
pub fn batch_command(metrics: &[String]) -> String {
    if metrics.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    for metric in metrics {
        let cmd = command_for(metric);
        if cmd.is_empty() {
            continue;
        }
        parts.push(format!("echo ---METRIC:{} & {}", metric, cmd));
    }
    parts.join(" & ")
}

/// Build a single CMD command that measures all path sizes with `---PATH:` markers.
pub fn batch_path_command(paths: &[(String, String)]) -> anyhow::Result<String> {
    use crate::host::quote::{cmd_echo_arg, quote_path};
    let mut parts = Vec::new();
    for (path, label) in paths {
        parts.push(format!(
            "echo ---PATH:{} & dir /s /-c /a {} 2>nul || echo MISSING",
            cmd_echo_arg(label)?,
            quote_path(crate::config::schema::ShellType::Cmd, path)?
        ));
    }
    Ok(parts.join(" & "))
}
