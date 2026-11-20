use crate::hosts::{parse_line, parse_lines, split_entry, valid_hostname, valid_ip};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: &'static str,
    pub hostname: String,
    pub ip: Option<String>,
}

pub struct Mutation {
    pub lines: Vec<String>,
    pub changes: Vec<Change>,
}

fn unique(names: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for name in names {
        if !result.contains(name) {
            result.push(name.clone());
        }
    }
    result
}

fn hostname_group(name: &str) -> Vec<String> {
    let base = name.strip_prefix("www.").unwrap_or(name);
    if valid_hostname(name) && valid_hostname(base) && base.contains('.') {
        vec![base.into(), format!("www.{base}")]
    } else {
        vec![name.into()]
    }
}

fn expand_names(names: &[String]) -> Vec<String> {
    unique(
        &names
            .iter()
            .flat_map(|name| hostname_group(name))
            .collect::<Vec<_>>(),
    )
}

fn render(line: &str, ip: Option<&str>, names: Option<&[String]>) -> String {
    let p = split_entry(line).expect("parsed entry");
    let hostnames = names
        .map(|n| n.join(" "))
        .unwrap_or_else(|| p.hostnames.join(" "));
    format!(
        "{}{}{}\t{}{}",
        p.indent,
        p.prefix,
        ip.unwrap_or(p.ip),
        hostnames,
        p.comment
    )
}

pub fn add(lines: &[String], ip: &str, names: &[String], force: bool) -> Result<Mutation, String> {
    if !valid_ip(ip) {
        return Err(format!("Invalid IP address: {ip}"));
    }
    let names = expand_names(names);
    let invalid: Vec<_> = names
        .iter()
        .filter(|name| !valid_hostname(name))
        .cloned()
        .collect();
    if !invalid.is_empty() {
        return Err(format!("Invalid hostname: {}", invalid.join(", ")));
    }
    let entries = parse_lines(lines);
    let duplicates: Vec<_> = names
        .iter()
        .filter(|name| {
            entries
                .iter()
                .any(|e| e.active && e.hostnames.contains(name))
        })
        .cloned()
        .collect();
    if !duplicates.is_empty() && !force {
        return Err(format!(
            "Already has an active entry: {}. Remove it first or use --force.",
            duplicates.join(", ")
        ));
    }
    let mut next = lines.to_vec();
    let mut changes = Vec::new();
    let mut new_names = Vec::new();
    for name in names {
        let indices: Vec<_> = entries
            .iter()
            .filter(|e| e.active && e.hostnames.contains(&name))
            .map(|e| e.line_index)
            .collect();
        if indices.is_empty() {
            new_names.push(name);
        } else {
            for index in indices {
                next[index] = render(&next[index], Some(ip), None);
            }
            changes.push(Change {
                kind: "Updated",
                hostname: name,
                ip: Some(ip.into()),
            });
        }
    }
    if !new_names.is_empty() {
        if next.last().is_some_and(|line| !line.is_empty()) {
            next.push(String::new());
        }
        for name in new_names {
            next.push(format!("{ip}\t{name}"));
            changes.push(Change {
                kind: "Added",
                hostname: name,
                ip: Some(ip.into()),
            });
        }
    }
    Ok(Mutation {
        lines: next,
        changes,
    })
}

pub fn remove(lines: &[String], names: &[String]) -> Result<Mutation, String> {
    let entries = parse_lines(lines);
    ensure_present(names, &entries)?;
    let names: Vec<_> = expand_names(names)
        .into_iter()
        .filter(|name| entries.iter().any(|entry| entry.hostnames.contains(name)))
        .collect();
    let mut next = lines.to_vec();
    for name in &names {
        for i in (0..next.len()).rev() {
            if let Some(entry) = parse_line(&next[i], i) {
                if !entry.hostnames.contains(name) {
                    continue;
                }
                let remaining: Vec<_> = entry.hostnames.into_iter().filter(|h| h != name).collect();
                if remaining.is_empty() {
                    next.remove(i);
                } else {
                    next[i] = render(&next[i], None, Some(&remaining));
                }
            }
        }
    }
    Ok(Mutation {
        lines: next,
        changes: names
            .into_iter()
            .map(|hostname| Change {
                kind: "Removed",
                hostname,
                ip: None,
            })
            .collect(),
    })
}

fn ensure_present(names: &[String], entries: &[crate::hosts::Entry]) -> Result<(), String> {
    let missing: Vec<_> = names
        .iter()
        .filter(|name| {
            let group = hostname_group(name);
            !entries
                .iter()
                .any(|entry| entry.hostnames.iter().any(|host| group.contains(host)))
        })
        .cloned()
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("No entry found for: {}", missing.join(", ")))
    }
}

pub fn toggle(lines: &[String], names: &[String]) -> Result<Mutation, String> {
    let entries = parse_lines(lines);
    ensure_present(names, &entries)?;
    let mut next = lines.to_vec();
    let mut changes = Vec::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for name in names {
        let group = hostname_group(name);
        let mut matching: Vec<_> = entries
            .iter()
            .filter(|entry| entry.hostnames.iter().any(|host| group.contains(host)))
            .map(|entry| entry.line_index)
            .collect();
        // Requests that share a hosts line must use the same target state.
        let mut index = 0;
        while index < groups.len() {
            if groups[index].iter().any(|line| matching.contains(line)) {
                for line in groups.remove(index) {
                    if !matching.contains(&line) {
                        matching.push(line);
                    }
                }
            } else {
                index += 1;
            }
        }
        groups.push(matching);
    }
    let names = expand_names(names);
    for group in groups {
        let matching: Vec<_> = entries
            .iter()
            .filter(|entry| group.contains(&entry.line_index))
            .collect();
        let enable = !matching.iter().any(|entry| entry.active);
        for entry in matching.iter().filter(|entry| entry.active != enable) {
            let i = entry.line_index;
            let body = next[i].trim_start();
            let indent = &next[i][..next[i].len() - body.len()];
            next[i] = if entry.active {
                format!("{indent}# {body}")
            } else {
                let body = &body[1..];
                let body = if body.starts_with(char::is_whitespace) {
                    &body[body.chars().next().unwrap().len_utf8()..]
                } else {
                    body
                };
                format!("{indent}{body}")
            };
        }
        for name in &names {
            if matching
                .iter()
                .any(|entry| entry.active != enable && entry.hostnames.contains(name))
            {
                changes.push(Change {
                    kind: if enable { "Enabled" } else { "Disabled" },
                    hostname: name.clone(),
                    ip: None,
                });
            }
        }
    }
    Ok(Mutation {
        lines: next,
        changes,
    })
}
