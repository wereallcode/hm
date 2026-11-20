use hm::{hosts::parse_lines, mutations, output::Renderer};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

fn strip_style(text: &str) -> String {
    let mut plain = String::new();
    let mut escape = false;
    for character in text.chars() {
        if character == '\x1b' {
            escape = true;
        } else if escape {
            if character == 'm' {
                escape = false;
            }
        } else {
            plain.push(character);
        }
    }
    plain
}

fn fits(text: &str, width: usize) {
    for line in strip_style(text).lines() {
        assert!(
            line.width() < width,
            "{width} columns: {line:?} ({})",
            line.width()
        );
        assert_eq!(line, line.trim_end(), "trailing spaces: {line:?}");
    }
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(String::from).collect()
}

#[test]
fn help_aligns_usage_and_stacks_on_narrow_terminals() {
    for width in [40, 60, 80, 120] {
        let renderer = Renderer::new(width, true, true);
        let help = renderer.help();
        fits(&help, width);
        let plain = strip_style(&help);
        assert!(plain.starts_with("\nhm — A CLI to manage entries"));
        assert!(plain.ends_with("\n\n"));
        if width >= 60 {
            let columns: Vec<_> = [
                "List active",
                "Filter by IP",
                "Add hostnames",
                "Change the IP",
                "Show help",
            ]
            .iter()
            .map(|description| {
                let row = plain
                    .lines()
                    .find(|line| line.contains(description))
                    .unwrap();
                row[..row.find(description).unwrap()].width()
            })
            .collect();
            assert!(columns.iter().all(|column| *column == columns[0]));
        } else {
            assert!(plain.contains("  hm --help\n    Show help"));
        }
        assert!(plain.contains("--force"));
        assert!(!plain.contains("Environment"));
        assert!(!plain.contains("--dry-run"));
        assert!(!plain.contains('→'));
    }
}

#[test]
fn list_groups_status_and_aligns_ipv4_and_full_ipv6_addresses() {
    let entries = parse_lines(&lines(
        "127.0.0.1 localhost app.local\nffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff ipv6.local\n# ::1 inactive.local",
    ));
    let renderer = Renderer::new(120, false, true);
    let plain = renderer.list(&entries, Path::new("/etc/hosts"), None);
    assert!(!plain.contains('\x1b'));
    assert!(
        plain.starts_with("hm — A CLI to manage entries in /etc/hosts.\n\n2 Active · 1 Inactive\n")
    );
    assert!(plain.find("Active:").unwrap() < plain.find("Inactive:").unwrap());
    assert_eq!(
        plain.lines().filter(|line| line.starts_with("→ ")).count(),
        3
    );
    assert!(plain.contains("Active:\n→ 127.0.0.1"));
    assert!(plain.contains("Inactive:\n→ ::1"));
    let columns: Vec<_> = ["localhost", "ipv6.local", "inactive.local"]
        .iter()
        .map(|hostname| {
            let line = plain.lines().find(|line| line.contains(hostname)).unwrap();
            line.find(hostname).unwrap()
        })
        .collect();
    assert!(columns.iter().all(|column| *column == columns[0]));

    let narrow = Renderer::new(40, true, false).list(&entries, Path::new("/etc/hosts"), None);
    fits(&narrow, 40);
    assert_eq!(
        narrow.lines().filter(|line| line.starts_with("→ ")).count(),
        3
    );
    assert!(narrow.contains("  ipv6.local"));
    assert!(narrow.contains("  inactive.local"));
    assert!(
        narrow
            .replace(char::is_whitespace, "")
            .contains("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff")
    );
}

#[test]
fn long_hostnames_and_unicode_paths_wrap_without_losing_content() {
    let hostname = format!("{}.{}.local", "a".repeat(63), "b".repeat(63));
    let entries = parse_lines(&lines(&format!("127.0.0.1 {hostname} last.local")));
    let path = Path::new("/tmp/配置/長いディレクトリ/hosts/another-directory/hosts");
    for width in [40, 80, 120] {
        let renderer = Renderer::new(width, true, true);
        let output = renderer.list(&entries, path, Some("local"));
        fits(&output, width);
        let flat = strip_style(&output).replace(char::is_whitespace, "");
        assert!(flat.contains(&hostname));
        assert!(flat.contains("last.local"));
        assert!(flat.contains(&path.display().to_string()));
    }
}

#[test]
fn empty_results_explain_filters_and_selected_file() {
    let renderer = Renderer::new(80, false, false);
    assert_eq!(
        renderer.list(&[], Path::new("/tmp/hosts"), None),
        "hm — A CLI to manage entries in /tmp/hosts.\n\n→ No host entries found.\n"
    );
    assert_eq!(
        renderer.list(&[], Path::new("/tmp/hosts"), Some("missing")),
        "hm — A CLI to manage entries in /tmp/hosts.\nSearch: missing\n\n→ No entries match this search.\n"
    );
}

#[test]
fn mutation_summaries_report_saved_changes() {
    let original = lines("127.0.0.1 app.local alias.local");
    let added = mutations::add(&original, "::1", &lines("app.local\nnew.local"), true).unwrap();
    let removed = mutations::remove(&original, &lines("app.local")).unwrap();
    let disabled = mutations::toggle(&original, &lines("app.local")).unwrap();
    let enabled = mutations::toggle(&disabled.lines, &lines("app.local")).unwrap();
    for (result, action) in [
        (&added, "Updated:"),
        (&removed, "Removed:"),
        (&disabled, "Disabled:"),
        (&enabled, "Enabled:"),
    ] {
        for width in [40, 80, 120] {
            let renderer = Renderer::new(width, true, true);
            let saved = strip_style(&renderer.mutation(result, Path::new("/tmp/hosts")));
            fits(&saved, width);
            assert!(
                saved
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .starts_with("hm — A CLI to manage entries in /tmp/hosts.")
            );
            assert!(saved.contains(action));
            assert!(saved.contains(&format!("{action}\n→ ")));
            assert!(saved.contains("Changes saved."));
            assert!(saved.contains("Backup: /tmp/hosts.bak"));
        }
    }
}

#[test]
fn restore_and_error_details_fit_narrow_terminals() {
    let path = Path::new("/tmp/a-long-directory/another-long-directory/hosts");
    for width in [40, 80, 120] {
        let renderer = Renderer::new(width, true, true);
        let error = renderer.error(
            "Could not read /tmp/a-long-directory/another-long-directory/hosts",
            Some("Run hm --help for usage examples."),
        );
        let restored = renderer.restored(path);
        for output in [&error, &restored] {
            fits(output, width);
        }
        assert!(strip_style(&restored).contains("Backup:\n→ "));
        assert!(strip_style(&error).contains("\n→ Run hm --help"));
    }
}

#[test]
fn untrusted_control_characters_cannot_change_terminal_styling() {
    let renderer = Renderer::new(80, false, true);
    let output = renderer.error("bad\x1b[31mname\nnext\tvalue", None);
    assert!(!output.contains('\x1b'));
    assert_eq!(output.lines().count(), 1);
    assert!(output.contains("\\u{1b}[31mname\\nnext\\tvalue"));
    let colored = Renderer::new(80, true, true).error("Invalid IP address: bad", None);
    let no_color = Renderer::new(80, true, false).error("Invalid IP address: bad", None);
    assert!(colored.contains('\x1b'));
    assert!(!no_color.contains('\x1b'));
    assert_eq!(strip_style(&colored), no_color);
}
