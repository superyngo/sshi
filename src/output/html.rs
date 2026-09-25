//! HTML rendering for `--out <file>.html` operation reports.

use super::report::OperationReport;

/// Render an [`OperationReport`] as a self-contained HTML page.
pub(crate) fn render_html_report(report: &OperationReport) -> String {
    let filter_str = match &report.filter.values {
        Some(vals) => format!("{}: {}", report.filter.mode, vals.join(", ")),
        None => report.filter.mode.clone(),
    };

    let rows = report
        .results
        .iter()
        .map(|r| {
            let duration = r
                .duration_ms
                .map(|ms| format!("{}ms", ms))
                .unwrap_or_else(|| "—".to_string());
            let status_class = match r.status.as_str() {
                "success" => "status-ok",
                "error" => "status-err",
                _ => "status-skip",
            };
            let output_html = render_output_html(&r.output);
            let output_raw = serde_json::to_string_pretty(&r.output)
                .unwrap_or_else(|e| format!("(serialization error: {})", e));
            format!(
                r#"<tr>
  <td>{host}</td>
  <td><span class="{cls}">{status}</span></td>
  <td>{duration}</td>
  <td class="output-cell">
    {output}
    <details>
      <summary>Raw output JSON</summary>
      <pre>{output_raw}</pre>
    </details>
  </td>
</tr>"#,
                host = html_escape(&r.host),
                cls = status_class,
                status = html_escape(&r.status),
                duration = duration,
                output = output_html,
                output_raw = html_escape(&output_raw),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let task_json = serde_json::to_string_pretty(&report.task)
        .unwrap_or_else(|e| format!("(serialization error: {})", e));
    let targets_json = serde_json::to_string_pretty(&report.targets)
        .unwrap_or_else(|e| format!("(serialization error: {})", e));

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>sshi {command} report</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; margin: 2rem; background: #f8f9fa; color: #212529; }}
  h1 {{ font-size: 1.4rem; margin-bottom: 0.25rem; }}
  .meta {{ color: #6c757d; font-size: 0.9rem; margin-bottom: 1.5rem; }}
  .summary {{ display: flex; gap: 1rem; margin-bottom: 1.5rem; }}
  .badge {{ padding: 0.35rem 0.75rem; border-radius: 4px; font-weight: 600; font-size: 0.85rem; }}
  .badge-total {{ background: #e9ecef; }}
  .badge-ok {{ background: #d4edda; color: #155724; }}
  .badge-err {{ background: #f8d7da; color: #721c24; }}
  .badge-skip {{ background: #fff3cd; color: #856404; }}
  table {{ width: 100%; border-collapse: collapse; background: white; border-radius: 8px; overflow: hidden; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }}
  th {{ background: #343a40; color: white; text-align: left; padding: 0.6rem 1rem; font-size: 0.85rem; }}
  td {{ padding: 0.6rem 1rem; border-bottom: 1px solid #dee2e6; vertical-align: top; font-size: 0.85rem; }}
  tr:last-child td {{ border-bottom: none; }}
  .status-ok {{ color: #28a745; font-weight: 600; }}
  .status-err {{ color: #dc3545; font-weight: 600; }}
  .status-skip {{ color: #ffc107; font-weight: 600; }}
  .output-cell {{ font-family: monospace; white-space: pre-wrap; word-break: break-all; max-width: 600px; }}
  details summary {{ cursor: pointer; color: #0066cc; }}
</style>
</head>
<body>
<h1>sshi {command} report</h1>
<div class="meta">
  <strong>Executed:</strong> {executed_at} &nbsp;|&nbsp;
  <strong>Filter:</strong> {filter}
</div>
<details>
  <summary>Task (JSON)</summary>
  <pre>{task_json}</pre>
</details>
<details>
  <summary>Targets (JSON)</summary>
  <pre>{targets_json}</pre>
</details>
<div class="summary">
  <span class="badge badge-total">Total: {total}</span>
  <span class="badge badge-ok">Success: {success}</span>
  <span class="badge badge-err">Failed: {failed}</span>
  <span class="badge badge-skip">Skipped: {skipped}</span>
</div>
<table>
<thead><tr><th>Host</th><th>Status</th><th>Duration</th><th>Output</th></tr></thead>
<tbody>
{rows}
</tbody>
</table>
</body>
</html>"#,
        command = html_escape(&report.command),
        executed_at = html_escape(&report.executed_at),
        filter = html_escape(&filter_str),
        task_json = html_escape(&task_json),
        targets_json = html_escape(&targets_json),
        total = report.summary.total,
        success = report.summary.success,
        failed = report.summary.failed,
        skipped = report.summary.skipped,
        rows = rows,
    )
}

fn render_output_html(output: &serde_json::Value) -> String {
    // log command: has "timestamp", "command", "action"
    if let Some(cmd) = output.get("command") {
        if output.get("timestamp").is_some() && output.get("action").is_some() {
            let timestamp = output
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let action = output.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let note = output.get("note").and_then(|v| v.as_str()).unwrap_or("");
            let cmd_str = cmd.as_str().unwrap_or("");
            let mut html = format!(
                "<strong>Time:</strong> {}<br><strong>Cmd:</strong> {}<br><strong>Action:</strong> {}",
                html_escape(timestamp), html_escape(cmd_str), html_escape(action)
            );
            if !note.is_empty() {
                html.push_str(&format!("<br><strong>Note:</strong> {}", html_escape(note)));
            }
            return html;
        }
    }

    // list command: has "ssh_host", "shell", "groups"
    if let Some(ssh_host) = output.get("ssh_host") {
        let shell = output.get("shell").and_then(|v| v.as_str()).unwrap_or("");
        let groups = output
            .get("groups")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|v| v.as_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        return format!(
            "<strong>SSH Host:</strong> {}<br><strong>Shell:</strong> {}<br><strong>Groups:</strong> {}",
            html_escape(ssh_host.as_str().unwrap_or("")),
            html_escape(shell),
            html_escape(&groups)
        );
    }

    // check command: has "metrics" and "probe_outputs"
    if let Some(metrics) = output.get("metrics") {
        let mut html = String::from("<strong>Metrics:</strong><br>");
        if let Some(obj) = metrics.as_object() {
            for (k, v) in obj {
                html.push_str(&format!(
                    "{}: {}<br>",
                    html_escape(k),
                    html_escape(&v.to_string())
                ));
            }
        }
        if let Some(probes) = output.get("probe_outputs") {
            html.push_str("<details><summary>Raw probe output</summary><pre>");
            html.push_str(&html_escape(
                &serde_json::to_string_pretty(probes)
                    .unwrap_or_else(|e| format!("(serialization error: {})", e)),
            ));
            html.push_str("</pre></details>");
        }
        return html;
    }

    // sync command: has "files_synced" and "files_skipped"
    if let Some(synced) = output.get("files_synced") {
        let mut html = String::new();
        if let Some(arr) = synced.as_array() {
            if !arr.is_empty() {
                html.push_str("<strong>Synced:</strong><br>");
                for f in arr {
                    let fallback = f.to_string();
                    let path_str = f.as_str().unwrap_or(&fallback);
                    html.push_str(&format!("  {}<br>", html_escape(path_str)));
                }
            }
        }
        if let Some(skipped) = output.get("files_skipped") {
            if let Some(arr) = skipped.as_array() {
                if !arr.is_empty() {
                    html.push_str("<strong>Skipped (in-sync):</strong><br>");
                    for f in arr {
                        let fallback = f.to_string();
                        let path_str = f.as_str().unwrap_or(&fallback);
                        html.push_str(&format!("  {}<br>", html_escape(path_str)));
                    }
                }
            }
        }
        if let Some(stderr) = output.get("stderr") {
            let s = stderr.as_str().unwrap_or("");
            if !s.is_empty() {
                html.push_str(&format!(
                    "<strong>stderr:</strong><pre>{}</pre>",
                    html_escape(s)
                ));
            }
        }
        return html;
    }

    // checkout: has "snapshot"
    if let Some(snap) = output.get("snapshot") {
        let online = output
            .get("online")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let collected_at = output
            .get("collected_at")
            .and_then(|v| v.as_str())
            .unwrap_or("—");
        return format!(
            "Online: {} | Collected: {}<details><summary>Snapshot</summary><pre>{}</pre></details>",
            if online { "✓" } else { "✗" },
            html_escape(collected_at),
            html_escape(
                &serde_json::to_string_pretty(snap)
                    .unwrap_or_else(|e| format!("(serialization error: {})", e))
            ),
        );
    }

    // run/exec: stdout/stderr
    let stdout = output.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    let stderr = output.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
    let mut html = String::new();
    if !stdout.is_empty() {
        html.push_str(&format!(
            "<strong>stdout:</strong><pre>{}</pre>",
            html_escape(stdout)
        ));
    }
    if !stderr.is_empty() {
        html.push_str(&format!(
            "<strong>stderr:</strong><pre>{}</pre>",
            html_escape(stderr)
        ));
    }
    html
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
