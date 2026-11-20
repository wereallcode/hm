use serde::Serialize;
use std::net::{IpAddr, Ipv6Addr};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub ip: String,
    pub hostnames: Vec<String>,
    pub active: bool,
    #[serde(skip)]
    pub line_index: usize,
}

pub struct Parts<'a> {
    pub indent: &'a str,
    pub prefix: &'a str,
    pub ip: &'a str,
    pub hostnames: Vec<&'a str>,
    pub comment: String,
}

pub fn valid_ip(value: &str) -> bool {
    if value.parse::<IpAddr>().is_ok() {
        return true;
    }
    // Accept IPv6 zone identifiers without resolving the interface.
    value.split_once('%').is_some_and(|(address, zone)| {
        address.parse::<Ipv6Addr>().is_ok()
            && !zone.is_empty()
            && zone
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".:-".contains(&b))
    })
}

pub fn valid_hostname(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && !valid_ip(value)
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

pub fn split_entry(line: &str) -> Option<Parts<'_>> {
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    let (prefix, body) = if let Some(rest) = body.strip_prefix('#') {
        let body = rest.trim_start();
        (&line[indent.len()..line.len() - body.len()], body)
    } else {
        ("", body)
    };
    let end = body.find(char::is_whitespace)?;
    let ip = &body[..end];
    let rest = body[end..].trim_start();
    if rest.contains(['\r', '\n', '\u{2028}', '\u{2029}']) {
        return None;
    }
    let comment_at = rest.find('#').unwrap_or(rest.len());
    let hostnames = rest[..comment_at].split_whitespace().collect::<Vec<_>>();
    if hostnames.is_empty() {
        return None;
    }
    let comment = if comment_at < rest.len() {
        format!(" {}", rest[comment_at..].trim_end())
    } else {
        String::new()
    };
    Some(Parts {
        indent,
        prefix,
        ip,
        hostnames,
        comment,
    })
}

pub fn parse_line(line: &str, line_index: usize) -> Option<Entry> {
    let parts = split_entry(line)?;
    let active = !line.trim_start().starts_with('#');
    if !valid_ip(parts.ip) {
        return None;
    }
    Some(Entry {
        ip: parts.ip.into(),
        hostnames: parts.hostnames.into_iter().map(String::from).collect(),
        active,
        line_index,
    })
}

pub fn parse_lines(lines: &[String]) -> Vec<Entry> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| parse_line(line, i))
        .collect()
}
