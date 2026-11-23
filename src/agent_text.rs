//! The text of the agent doctor's report (issue 110 and T-182). The rows,
//! the UDP, IPv6 and localhost probes, the resolver and the advice are
//! rendered here, so `agent` keeps the probes and this file keeps their
//! presentation.

use crate::agent::{Outcome, Report};

fn mark(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Ok => "ok".into(),
        Outcome::Intercepted => "tls-intercepted".into(),
        Outcome::Failed(_) => "fail".into(),
        Outcome::NotTried => "-".into(),
    }
}

/// The report as text.
pub fn text(report: &Report) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "proxy: {}   podnet warp up: {}\n\n",
        report.proxy.as_deref().unwrap_or("none"),
        if report.warp_up {
            "runs"
        } else {
            "does not run"
        }
    ));
    out.push_str(&format!(
        "{:<20} {:<28} {:<16} {:<16} {:<16}\n",
        "TASK", "TARGET", "DIRECT", "PROXY", "WARP"
    ));
    for row in &report.rows {
        out.push_str(&format!(
            "{:<20} {:<28} {:<16} {:<16} {:<16}\n",
            row.task,
            row.target,
            mark(&row.direct),
            mark(&row.proxy),
            mark(&row.warp)
        ));
    }
    for row in &report.rows {
        if let Some(files) = &row.files {
            out.push_str(&format!("  {}: files from {files}\n", row.task));
        }
    }
    out.push_str(&format!(
        "\nThis host, direct (no WARP): UDP DNS (1.1.1.1:53): {}   IPv6: {}   localhost: {}   resolver: {}\n",
        mark(&report.udp_dns),
        mark(&report.ipv6),
        mark(&report.localhost),
        mark(&report.resolver)
    ));
    if !report.udp_dns.works() {
        out.push_str(if report.warp_up {
            "  UDP fails for a program that runs on this host. Under WARP it has UDP: podnet warp exec -- CMD, podnet warp shell, or podnet warp run -- CMD.\n"
        } else {
            "  UDP fails for a program that runs on this host. Under WARP it has UDP: podnet warp run -- CMD, or start podnet warp up and use podnet warp exec -- CMD or podnet warp shell.\n"
        });
    }
    out.push('\n');
    for line in &report.advice {
        out.push_str(&format!("* {line}\n"));
    }
    out
}
