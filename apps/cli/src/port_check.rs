use anyhow::{bail, Result};
use std::collections::BTreeSet;
use std::net::TcpListener;

use crate::ui;

pub struct PortCheck {
    pub port: u16,
    pub service: String,
    pub status: PortStatus,
}

pub enum PortStatus {
    Free,
    InUse,
    Error(String),
}

impl PortStatus {
    fn label(&self) -> &'static str {
        match self {
            PortStatus::Free => "free",
            PortStatus::InUse => "in use",
            PortStatus::Error(_) => "error",
        }
    }

    fn style(&self) -> console::Style {
        let s = ui::style();
        match self {
            PortStatus::Free => s.ok.clone(),
            PortStatus::InUse => s.fail.clone(),
            PortStatus::Error(_) => s.warn.clone(),
        }
    }
}

fn probe(port: u16) -> PortStatus {
    match TcpListener::bind(("0.0.0.0", port)) {
        Ok(listener) => {
            drop(listener);
            PortStatus::Free
        }
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => PortStatus::InUse,
        Err(e) => PortStatus::Error(e.to_string()),
    }
}

/// Parse the host-side port out of a docker `-p` arg string.
/// Accepts: "8080", "8080:80", "127.0.0.1:8080:80".
/// Returns None for forms we don't understand (ranges, udp suffix, etc.).
pub fn parse_docker_host_port(spec: &str) -> Option<u16> {
    let spec = spec.split('/').next().unwrap_or(spec);
    let parts: Vec<&str> = spec.split(':').collect();
    let host = match parts.as_slice() {
        [p] => p,
        [h, _] => h,
        [_ip, h, _c] => h,
        _ => return None,
    };
    host.trim().parse().ok()
}

/// Probe every port `spky dev` is about to bind. Finishes `step` as
/// `✓ Ports free  8666 8667 …` on success; on a conflict, fails the step,
/// prints the table + a hint and bails.
pub fn ensure_ports_free(checks: Vec<(u16, String)>, step: ui::Step) -> Result<()> {
    if checks.is_empty() {
        step.done_quiet();
        return Ok(());
    }

    let mut seen: BTreeSet<u16> = BTreeSet::new();
    let mut results: Vec<PortCheck> = Vec::new();
    for (port, service) in checks {
        if !seen.insert(port) {
            continue;
        }
        let status = probe(port);
        results.push(PortCheck {
            port,
            service,
            status,
        });
    }

    let any_blocked = results
        .iter()
        .any(|r| !matches!(r.status, PortStatus::Free));

    if !any_blocked {
        let ports: Vec<String> = results.iter().map(|r| r.port.to_string()).collect();
        step.done(ports.join(" "));
        return Ok(());
    }

    let blocked: Vec<String> = results
        .iter()
        .filter(|r| !matches!(r.status, PortStatus::Free))
        .map(|r| r.port.to_string())
        .collect();
    step.fail(format!("in use: {}", blocked.join(" ")));
    ui::println("");
    print_table(&results);

    let first_busy = results
        .iter()
        .find(|r| matches!(r.status, PortStatus::InUse))
        .map(|r| r.port);
    if let Some(p) = first_busy {
        ui::println("");
        ui::hint(format!(
            "Find the listener with `lsof -nP -iTCP:{} -sTCP:LISTEN` and stop it",
            p
        ));
        ui::hint("(often a previous `spky dev` left a container running: `docker ps`, then `docker rm -f <name>`).");
    }

    bail!("port pre-check failed: one or more required ports are unavailable");
}

fn print_table(rows: &[PortCheck]) {
    let h_port = "PORT";
    let h_service = "SERVICE";
    let h_status = "STATUS";

    let w_port = rows
        .iter()
        .map(|r| r.port.to_string().len())
        .max()
        .unwrap_or(0)
        .max(h_port.len());
    let w_service = rows
        .iter()
        .map(|r| r.service.len())
        .max()
        .unwrap_or(0)
        .max(h_service.len());
    let w_status = rows
        .iter()
        .map(|r| r.status.label().len())
        .max()
        .unwrap_or(0)
        .max(h_status.len());

    let bar = |left: &str, mid: &str, right: &str| {
        format!(
            "  {}{}{}{}{}{}{}",
            left,
            "─".repeat(w_port + 2),
            mid,
            "─".repeat(w_service + 2),
            mid,
            "─".repeat(w_status + 2),
            right,
        )
    };

    ui::println(bar("┌", "┬", "┐"));
    ui::println(format!(
        "  │ {:<wp$} │ {:<ws$} │ {:<wst$} │",
        h_port,
        h_service,
        h_status,
        wp = w_port,
        ws = w_service,
        wst = w_status,
    ));
    ui::println(bar("├", "┼", "┤"));

    for r in rows {
        let label = r.status.label();
        let colored_status = r.status.style().apply_to(label).to_string();
        let pad = w_status.saturating_sub(label.len());
        ui::println(format!(
            "  │ {:<wp$} │ {:<ws$} │ {}{} │",
            r.port,
            r.service,
            colored_status,
            " ".repeat(pad),
            wp = w_port,
            ws = w_service,
        ));
    }

    ui::println(bar("└", "┴", "┘"));

    let errored: Vec<&PortCheck> = rows
        .iter()
        .filter(|r| matches!(r.status, PortStatus::Error(_)))
        .collect();
    if !errored.is_empty() {
        ui::println("");
        for r in errored {
            if let PortStatus::Error(msg) = &r.status {
                ui::warn(format!("port {} probe error: {}", r.port, msg));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_port() {
        assert_eq!(parse_docker_host_port("8080"), Some(8080));
    }

    #[test]
    fn parses_host_container_pair() {
        assert_eq!(parse_docker_host_port("8080:80"), Some(8080));
    }

    #[test]
    fn parses_ip_host_container_triple() {
        assert_eq!(parse_docker_host_port("127.0.0.1:8080:80"), Some(8080));
    }

    #[test]
    fn parses_with_proto_suffix() {
        assert_eq!(parse_docker_host_port("8080:80/tcp"), Some(8080));
    }

    #[test]
    fn rejects_range_form() {
        assert_eq!(parse_docker_host_port("3006-3008:3000-3002"), None);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_docker_host_port("not-a-port"), None);
    }
}
