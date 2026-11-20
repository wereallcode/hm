use crate::{hosts::Entry, mutations::Mutation, storage};
use std::{
    env,
    io::{self, IsTerminal},
    os::fd::RawFd,
    path::Path,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const USAGE: &[(&str, &str)] = &[
    ("hm list", "List active and inactive entries"),
    (
        "hm list --search <query>",
        "Filter by IP or hostname substring",
    ),
    ("hm list --json", "Output entries as JSON"),
    ("", ""),
    (
        "hm add <ip> <hostnames>",
        "Add hostnames and their www. variants",
    ),
    (
        "hm add <ip> <hostnames> --force",
        "Change the IP of existing hostnames",
    ),
    (
        "hm remove <hostnames>",
        "Remove hostnames and their www. variants",
    ),
    (
        "hm toggle <hostnames>",
        "Enable or disable hostnames and their www. variants",
    ),
    ("hm restore", "Restore the last backup"),
    ("", ""),
    ("hm setup", "Authorize password-free hosts changes"),
    ("hm setup --remove", "Revoke password-free hosts changes"),
    ("", ""),
    ("hm --help", "Show help"),
    ("hm --version", "Show version"),
];
const ITEM_PREFIX: &str = "→ ";

#[derive(Clone, Copy)]
enum Tone {
    Plain,
    Bold,
    Dim,
    Success,
    Error,
}

/// Text layout and styling for one output stream.
pub struct Renderer {
    width: usize,
    terminal: bool,
    color: bool,
}

impl Renderer {
    pub fn new(width: usize, terminal: bool, color: bool) -> Self {
        Self {
            width: width.max(1),
            terminal,
            color: terminal && color,
        }
    }

    pub fn stdout() -> Self {
        Self::detect(libc::STDOUT_FILENO, io::stdout().is_terminal())
    }

    pub fn stderr() -> Self {
        Self::detect(libc::STDERR_FILENO, io::stderr().is_terminal())
    }

    fn detect(fd: RawFd, is_terminal: bool) -> Self {
        let terminal = is_terminal && env::var_os("TERM").is_none_or(|term| term != "dumb");
        let color = terminal && env::var_os("NO_COLOR").is_none();
        let mut size = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: TIOCGWINSZ writes a winsize to the valid pointer for this call.
        let has_size = terminal && unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut size) } == 0;
        let width = if has_size && size.ws_col > 0 {
            usize::from(size.ws_col)
        } else {
            80
        };
        Self::new(width, terminal, color)
    }

    fn budget(&self) -> usize {
        if self.terminal {
            self.width.saturating_sub(1).max(1)
        } else {
            usize::MAX
        }
    }

    fn styled(&self, text: &str, tone: Tone) -> String {
        if !self.color || matches!(tone, Tone::Plain) {
            return text.into();
        }
        let code = match tone {
            Tone::Plain => unreachable!(),
            Tone::Bold => "1",
            Tone::Dim => "2",
            Tone::Success => "32",
            Tone::Error => "1;31",
        };
        format!("\x1b[{code}m{text}\x1b[0m")
    }

    fn text(&self, lines: &mut Vec<String>, text: &str, indent: usize, tone: Tone) {
        let prefix = " ".repeat(indent.min(self.budget().saturating_sub(1)));
        self.marked(lines, text, &prefix, tone);
    }

    fn marked(&self, lines: &mut Vec<String>, text: &str, prefix: &str, tone: Tone) {
        let prefix = if prefix.width() < self.budget() {
            prefix
        } else {
            ""
        };
        let continuation = " ".repeat(prefix.width());
        for (index, part) in wrap(&clean(text), self.budget() - prefix.width())
            .iter()
            .enumerate()
        {
            let head = if index == 0 { prefix } else { &continuation };
            lines.push(format!("{head}{}", self.styled(part, tone)));
        }
    }

    fn item(&self, lines: &mut Vec<String>, text: &str, tone: Tone) {
        self.marked(lines, text, ITEM_PREFIX, tone);
    }

    fn heading(&self, lines: &mut Vec<String>, path: &Path) {
        self.text(
            lines,
            &format!("hm — A CLI to manage entries in {}.", path.display()),
            0,
            Tone::Bold,
        );
    }

    fn section(&self, lines: &mut Vec<String>, title: &str) {
        lines.push(String::new());
        self.text(lines, &format!("{title}:"), 0, Tone::Bold);
    }

    fn finish(&self, lines: Vec<String>) -> String {
        let text = lines.join("\n");
        if self.terminal {
            format!("\n{text}\n\n")
        } else {
            format!("{text}\n")
        }
    }

    /// Align definitions across sections, or stack them on narrow terminals.
    fn definitions(
        &self,
        lines: &mut Vec<String>,
        rows: &[(&str, &str)],
        column: usize,
        label_tone: Tone,
        prefix: &str,
    ) {
        let inline = column.saturating_add(20) <= self.budget();
        for &(label, value) in rows {
            if label.is_empty() {
                lines.push(String::new());
                continue;
            }
            let label = clean(label);
            let value = clean(value);
            if inline {
                let padding = " ".repeat(column - prefix.width() - label.width());
                for (index, part) in wrap(&value, self.budget() - column).iter().enumerate() {
                    let head = if index == 0 {
                        format!("{prefix}{}{padding}", self.styled(&label, label_tone))
                    } else {
                        " ".repeat(column)
                    };
                    lines.push(format!("{head}{part}"));
                }
            } else {
                self.marked(lines, &label, prefix, label_tone);
                let indent = if prefix.trim().is_empty() {
                    4
                } else {
                    prefix.width()
                };
                self.text(lines, &value, indent, Tone::Plain);
            }
        }
    }

    pub fn help(&self) -> String {
        let mut lines = Vec::new();
        self.heading(&mut lines, Path::new("/etc/hosts"));
        let column = USAGE
            .iter()
            .map(|(label, _)| label.width())
            .max()
            .unwrap_or(0)
            + 4;
        self.section(&mut lines, "Usage");
        self.definitions(&mut lines, USAGE, column, Tone::Bold, "  ");
        self.finish(lines)
    }

    pub fn list(&self, entries: &[Entry], path: &Path, search: Option<&str>) -> String {
        let mut lines = Vec::new();
        self.heading(&mut lines, path);
        if let Some(query) = search {
            self.text(&mut lines, &format!("Search: {query}"), 0, Tone::Dim);
        }
        if entries.is_empty() {
            lines.push(String::new());
            self.item(
                &mut lines,
                if search.is_some() {
                    "No entries match this search."
                } else {
                    "No host entries found."
                },
                Tone::Dim,
            );
        } else {
            let active = entries.iter().filter(|entry| entry.active).count();
            lines.push(String::new());
            self.text(
                &mut lines,
                &format!("{active} Active · {} Inactive", entries.len() - active),
                0,
                Tone::Dim,
            );
            let column = entries
                .iter()
                .map(|entry| entry.ip.width())
                .max()
                .unwrap_or(0)
                + 4;
            for (active, title) in [(true, "Active"), (false, "Inactive")] {
                let group: Vec<_> = entries
                    .iter()
                    .filter(|entry| entry.active == active)
                    .collect();
                if group.is_empty() {
                    continue;
                }
                self.section(&mut lines, title);
                for entry in group {
                    let mut row = Vec::new();
                    let hostnames = entry.hostnames.join(" ");
                    self.definitions(
                        &mut row,
                        &[(&entry.ip, &hostnames)],
                        column,
                        Tone::Plain,
                        ITEM_PREFIX,
                    );
                    lines.extend(row.into_iter().map(|line| {
                        if active {
                            line
                        } else {
                            self.styled(&line, Tone::Dim)
                        }
                    }));
                }
            }
        }
        self.finish(lines)
    }

    pub fn mutation(&self, result: &Mutation, path: &Path) -> String {
        let mut lines = Vec::new();
        self.heading(&mut lines, path);
        let column = result
            .changes
            .iter()
            .filter_map(|change| change.ip.as_deref())
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0)
            + 4;
        for kind in ["Added", "Updated", "Removed", "Enabled", "Disabled"] {
            let changes: Vec<_> = result
                .changes
                .iter()
                .filter(|change| change.kind == kind)
                .collect();
            if changes.is_empty() {
                continue;
            }
            self.section(&mut lines, kind);
            for change in changes {
                if let Some(ip) = &change.ip {
                    self.definitions(
                        &mut lines,
                        &[(ip, &change.hostname)],
                        column,
                        Tone::Plain,
                        ITEM_PREFIX,
                    );
                } else {
                    self.item(&mut lines, &change.hostname, Tone::Plain);
                }
            }
        }
        lines.push(String::new());
        if result
            .changes
            .iter()
            .any(|change| matches!(change.kind, "Updated" | "Enabled" | "Disabled"))
        {
            self.text(
                &mut lines,
                "Updates and toggles affect the whole matching line.",
                0,
                Tone::Dim,
            );
        }
        self.success(&mut lines, "Changes saved.");
        self.item(
            &mut lines,
            &format!("Backup: {}", storage::backup_path(path).display()),
            Tone::Dim,
        );
        self.finish(lines)
    }

    pub fn restored(&self, path: &Path) -> String {
        let mut lines = Vec::new();
        self.heading(&mut lines, path);
        self.section(&mut lines, "Backup");
        self.item(
            &mut lines,
            &storage::backup_path(path).display().to_string(),
            Tone::Plain,
        );
        lines.push(String::new());
        self.success(&mut lines, "Hosts file restored. Backup kept.");
        self.finish(lines)
    }

    fn success(&self, lines: &mut Vec<String>, text: &str) {
        let prefix = if self.terminal { "✓ " } else { "OK: " };
        self.marked(lines, text, prefix, Tone::Success);
    }

    pub fn error(&self, message: &str, hint: Option<&str>) -> String {
        let mut lines = Vec::new();
        self.text(&mut lines, &format!("Error: {message}"), 0, Tone::Error);
        if let Some(hint) = hint {
            lines.push(String::new());
            self.item(&mut lines, hint, Tone::Dim);
        }
        self.finish(lines)
    }
}

/// Escape file and argument control characters before adding terminal styling.
fn clean(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text;
    while rest.width() > width {
        let mut used = 0;
        let mut end = 0;
        let mut space = None;
        for (index, character) in rest.char_indices() {
            let next = character.width().unwrap_or(0);
            if used + next > width {
                break;
            }
            used += next;
            end = index + character.len_utf8();
            if character == ' ' && index > 0 {
                space = Some(index);
            }
        }
        // A single wide character may exceed a one-column terminal.
        if end == 0 {
            end = rest.chars().next().unwrap().len_utf8();
        }
        let split = space.unwrap_or(end);
        lines.push(rest[..split].trim_end().into());
        rest = rest[split..].trim_start();
    }
    if !rest.is_empty() || lines.is_empty() {
        lines.push(rest.into());
    }
    lines
}
