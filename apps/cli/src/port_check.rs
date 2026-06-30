use anyhow::{bail, Result};
use std::collections::BTreeSet;
use std::net::TcpListener;

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_DIM: &str = "\x1b[2m";

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

    fn color(&self) -> &'static str {
        match self {
            PortStatus::Free => ANSI_GREEN,
            PortStatus::InUse => ANSI_RED,
            PortStatus::Error(_) => ANSI_YELLOW,
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

pub fn ensure_ports_free(checks: Vec<(u16, String)>, prefix: &str) -> Result<()> {
    if checks.is_empty() {
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
        println!(
            "{} Port check: all {} required port(s) free.",
            prefix,
            results.len()
        );
        return Ok(());
    }

    eprintln!("{} Some required ports are not available:\n", prefix);
    print_table(&results);

    let first_busy = results
        .iter()
        .find(|r| matches!(r.status, PortStatus::InUse))
        .map(|r| r.port);
    if let Some(p) = first_busy {
        eprintln!(
            "\n  {}Hint:{} find the listener with `lsof -nP -iTCP:{} -sTCP:LISTEN`,",
            ANSI_DIM, ANSI_RESET, p
        );
        eprintln!("        and stop it (often a previous `spky dev` left a container running:");
        eprintln!("        `docker ps` then `docker rm -f <name>`).");
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

    println!("{}", bar("┌", "┬", "┐"));
    println!(
        "  │ {:<wp$} │ {:<ws$} │ {:<wst$} │",
        h_port,
        h_service,
        h_status,
        wp = w_port,
        ws = w_service,
        wst = w_status,
    );
    println!("{}", bar("├", "┼", "┤"));

    for r in rows {
        let label = r.status.label();
        let colored_status = format!("{}{}{}", r.status.color(), label, ANSI_RESET);
        let pad = w_status.saturating_sub(label.len());
        println!(
            "  │ {:<wp$} │ {:<ws$} │ {}{} │",
            r.port,
            r.service,
            colored_status,
            " ".repeat(pad),
            wp = w_port,
            ws = w_service,
        );
    }

    println!("{}", bar("└", "┴", "┘"));

    let errored: Vec<&PortCheck> = rows
        .iter()
        .filter(|r| matches!(r.status, PortStatus::Error(_)))
        .collect();
    if !errored.is_empty() {
        eprintln!();
        for r in errored {
            if let PortStatus::Error(msg) = &r.status {
                eprintln!("  port {} probe error: {}", r.port, msg);
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
