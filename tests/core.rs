use hm::{hosts::*, mutations::*};
fn lines(raw: &str) -> Vec<String> {
    raw.split('\n').map(String::from).collect()
}
fn names(raw: &[&str]) -> Vec<String> {
    raw.iter().map(|s| s.to_string()).collect()
}

#[test]
fn ip_and_hostname_validation_boundaries() {
    for ip in [
        "127.0.0.1",
        "0.0.0.0",
        "::1",
        "2001:db8::1",
        "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
        "fe80::1%en0",
    ] {
        assert!(valid_ip(ip), "{ip}");
    }
    for ip in [
        "",
        "999.0.0.1",
        "127.0.0",
        "01.2.3.4",
        "2001:db8:::1",
        "::1%",
        "::1%bad zone",
    ] {
        assert!(!valid_ip(ip), "{ip}");
    }
    for host in [
        "localhost",
        "a.b.c",
        "sub-domain.example.co.uk",
        &format!("{}.com", "a".repeat(63)),
        &format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61)
        ),
    ] {
        assert!(valid_hostname(host), "{host}");
    }
    for host in [
        "",
        "127.0.0.1",
        "::1",
        "a_b",
        "a.",
        "-a",
        "a-",
        "a..b",
        "é.local",
        &"a".repeat(254),
        &format!("{}.com", "a".repeat(64)),
    ] {
        assert!(!valid_hostname(host), "{host}");
    }
}

#[test]
fn parser_keeps_aliases_indices_and_legacy_entries() {
    let entry = parse_line("  #127.0.0.1 localhost legacy_name app.local. # comment", 7).unwrap();
    assert_eq!(entry.ip, "127.0.0.1");
    assert_eq!(
        entry.hostnames,
        names(&["localhost", "legacy_name", "app.local."])
    );
    assert!(!entry.active);
    assert_eq!(entry.line_index, 7);
    assert!(parse_line("::1 legacy!name", 0).is_some());
    for raw in [
        "",
        " ",
        "# a comment",
        "# not.an.ip comment, punctuation!",
        "bad.ip app",
        "127.0.0.1 # comment",
    ] {
        assert!(parse_line(raw, 0).is_none(), "{raw}");
    }
}

#[test]
fn inline_comments_do_not_require_a_space_after_hostnames() {
    for raw in [
        "127.0.0.1 app.local alias.local# keep # comment",
        "# 127.0.0.1 app.local alias.local# keep # comment",
    ] {
        let entry = parse_line(raw, 0).unwrap();
        assert_eq!(entry.hostnames, names(&["app.local", "alias.local"]));
        assert_eq!(entry.active, !raw.starts_with('#'));
    }
}

#[test]
fn mutations_keep_attached_comments_and_recognize_existing_hostnames() {
    let original = lines("127.0.0.1 app.local alias.local# keep  this # comment\n");
    assert!(add(&original, "::1", &names(&["alias.local"]), false).is_err());
    let updated = add(&original, "::1", &names(&["alias.local"]), true).unwrap();
    assert_eq!(
        updated.lines,
        lines("::1\tapp.local alias.local # keep  this # comment\n\n::1\twww.alias.local")
    );
    let removed = remove(&original, &names(&["alias.local"])).unwrap();
    assert_eq!(
        removed.lines,
        lines("127.0.0.1\tapp.local # keep  this # comment\n")
    );
    let disabled = toggle(&original, &names(&["alias.local"])).unwrap();
    assert!(!parse_line(&disabled.lines[0], 0).unwrap().active);
    assert_eq!(
        toggle(&disabled.lines, &names(&["alias.local"]))
            .unwrap()
            .lines,
        original
    );
}

#[test]
fn force_updates_all_active_lines_and_their_aliases() {
    let original =
        lines("# 1.1.1.1 app.local\n  127.0.0.1 app.local api.local # keep me\n::1 app.local");
    assert!(
        add(
            &original,
            "10.0.0.1",
            &names(&["app.local", "new.local"]),
            false
        )
        .is_err()
    );
    let result = add(
        &original,
        "10.0.0.1",
        &names(&["app.local", "new.local", "new.local"]),
        true,
    )
    .unwrap();
    assert_eq!(
        result.lines,
        lines(
            "# 1.1.1.1 app.local\n  10.0.0.1\tapp.local api.local # keep me\n10.0.0.1\tapp.local\n\n10.0.0.1\twww.app.local\n10.0.0.1\tnew.local\n10.0.0.1\twww.new.local"
        )
    );
    assert_eq!(result.changes.len(), 4);
    assert!(add(&original, "::1", &names(&["legacy_name"]), false).is_err());
}

#[test]
fn inactive_names_can_be_added_and_matching_is_case_sensitive() {
    let original = lines("# 127.0.0.1 app.local\n127.0.0.1 App.local\n");
    assert!(add(&original, "::1", &names(&["app.local"]), false).is_ok());
    assert!(remove(&original, &names(&["APP.local"])).is_err());
}

#[test]
fn remove_validates_every_name_and_removes_only_selected_aliases() {
    let original = lines("127.0.0.1 app.local keep.local # comment\n# ::1 app.local\n");
    assert!(remove(&original, &names(&["app.local", "missing.local"])).is_err());
    let result = remove(&original, &names(&["app.local", "app.local"])).unwrap();
    assert_eq!(result.lines, lines("127.0.0.1\tkeep.local # comment\n"));
    assert_eq!(result.changes.len(), 1);
}

#[test]
fn toggle_prefers_active_and_toggles_shared_line_once() {
    let original = lines("# ::1 app.local\n  127.0.0.1\tapp.local legacy_name app.local.");
    let disabled = toggle(
        &original,
        &names(&["app.local", "legacy_name", "legacy_name"]),
    )
    .unwrap();
    assert_eq!(
        disabled.lines,
        lines("# ::1 app.local\n  # 127.0.0.1\tapp.local legacy_name app.local.")
    );
    assert_eq!(disabled.changes.len(), 2);
    assert_eq!(
        toggle(&disabled.lines, &names(&["legacy_name"]))
            .unwrap()
            .lines,
        original
    );
    assert!(toggle(&original, &names(&["app.local", "missing"])).is_err());
}

#[test]
fn toggling_existing_legacy_hostnames_can_be_reversed() {
    for hostname in ["legacy!name", "legacy+name", "例.local"] {
        let original = lines(&format!("127.0.0.1 {hostname}\n"));
        let disabled = toggle(&original, &names(&[hostname])).unwrap();
        assert!(!parse_line(&disabled.lines[0], 0).unwrap().active);
        assert_eq!(
            toggle(&disabled.lines, &names(&[hostname])).unwrap().lines,
            original
        );
    }
}

#[test]
fn standalone_carriage_return_is_not_a_line_terminator() {
    assert!(parse_line("127.0.0.1 app.local\r", 0).is_none());
    let original = lines("127.0.0.1 app.local\r");
    assert_eq!(
        add(&original, "::1", &names(&["app.local"]), false)
            .unwrap()
            .lines,
        lines("127.0.0.1 app.local\r\n\n::1\tapp.local\n::1\twww.app.local")
    );
}

#[test]
fn add_pairs_domains_in_both_directions_without_duplicates() {
    for input in [
        names(&["example.com"]),
        names(&["www.example.com"]),
        names(&["example.com", "www.example.com", "example.com"]),
    ] {
        let added = add(&[], "127.0.0.1", &input, false).unwrap();
        assert_eq!(
            added.lines,
            lines("127.0.0.1\texample.com\n127.0.0.1\twww.example.com")
        );
        assert_eq!(added.changes.len(), 2);
    }
}

#[test]
fn add_checks_and_force_updates_both_domain_variants() {
    let original = lines("127.0.0.1 www.example.com # keep\n# ::1 example.com\n");
    assert!(add(&original, "10.0.0.1", &names(&["example.com"]), false).is_err());
    let updated = add(&original, "10.0.0.1", &names(&["example.com"]), true).unwrap();
    assert_eq!(
        updated.lines,
        lines("10.0.0.1\twww.example.com # keep\n# ::1 example.com\n\n10.0.0.1\texample.com")
    );
    assert_eq!(updated.changes.len(), 2);
    let updated = add(&updated.lines, "::1", &names(&["www.example.com"]), true).unwrap();
    let active: Vec<_> = parse_lines(&updated.lines)
        .into_iter()
        .filter(|entry| entry.active)
        .collect();
    assert_eq!(active.len(), 2);
    assert!(active.iter().all(|entry| entry.ip == "::1"));
}

#[test]
fn toggle_pairs_separate_lines_and_all_active_ip_mappings() {
    let original = lines(
        "127.0.0.1 example.com\n::1 example.com\n  127.0.0.1 www.example.com # keep\n127.0.0.1 api.example.com",
    );
    for input in [
        names(&["example.com"]),
        names(&["www.example.com"]),
        names(&["example.com", "www.example.com", "example.com"]),
    ] {
        let disabled = toggle(&original, &input).unwrap();
        assert_eq!(
            disabled.lines,
            lines(
                "# 127.0.0.1 example.com\n# ::1 example.com\n  # 127.0.0.1 www.example.com # keep\n127.0.0.1 api.example.com"
            )
        );
        assert_eq!(disabled.changes.len(), 2);
        assert_eq!(toggle(&disabled.lines, &input).unwrap().lines, original);
    }
}

#[test]
fn toggle_synchronizes_mixed_states_instead_of_swapping_them() {
    for original in [
        lines("127.0.0.1 example.com\n# ::1 www.example.com"),
        lines("# 127.0.0.1 example.com\n::1 www.example.com"),
    ] {
        for name in ["example.com", "www.example.com"] {
            let disabled = toggle(&original, &names(&[name])).unwrap();
            assert!(
                parse_lines(&disabled.lines)
                    .iter()
                    .all(|entry| !entry.active)
            );
            assert_eq!(disabled.changes.len(), 1);
            let enabled = toggle(&disabled.lines, &names(&[name])).unwrap();
            assert!(parse_lines(&enabled.lines).iter().all(|entry| entry.active));
            assert_eq!(enabled.changes.len(), 2);
        }
    }
}

#[test]
fn toggle_keeps_overlapping_pairs_consistent_in_either_argument_order() {
    let original = lines(
        "127.0.0.1 example.com\n# 127.0.0.1 www.example.com second.com # keep\n# ::1 www.second.com",
    );
    for input in [
        names(&["example.com", "second.com"]),
        names(&["second.com", "example.com"]),
    ] {
        let disabled = toggle(&original, &input).unwrap();
        assert_eq!(
            disabled.lines,
            lines(
                "# 127.0.0.1 example.com\n# 127.0.0.1 www.example.com second.com # keep\n# ::1 www.second.com"
            )
        );
        assert_eq!(disabled.changes.len(), 1);
        let enabled = toggle(&disabled.lines, &input).unwrap();
        assert!(parse_lines(&enabled.lines).iter().all(|entry| entry.active));
        assert_eq!(enabled.changes.len(), 4);
    }
}

#[test]
fn paired_commands_accept_either_existing_variant_and_validate_all_requests() {
    for existing in ["example.com", "www.example.com"] {
        let original = lines(&format!("127.0.0.1 {existing}"));
        for requested in ["example.com", "www.example.com"] {
            let input = names(&[requested]);
            assert!(remove(&original, &input).unwrap().lines.is_empty());
            assert!(!parse_lines(&toggle(&original, &input).unwrap().lines)[0].active);
            let input = names(&[requested, "missing.com"]);
            assert!(remove(&original, &input).is_err());
            assert!(toggle(&original, &input).is_err());
        }
    }
}

#[test]
fn paired_remove_preserves_other_aliases_comments_and_subdomains() {
    let original = lines(
        "127.0.0.1 example.com www.example.com keep.local # keep\n# ::1 www.example.com\n::1 example.com\n127.0.0.1 api.example.com",
    );
    for input in [
        names(&["example.com"]),
        names(&["www.example.com", "example.com"]),
    ] {
        let removed = remove(&original, &input).unwrap();
        assert_eq!(
            removed.lines,
            lines("127.0.0.1\tkeep.local # keep\n127.0.0.1 api.example.com")
        );
        assert_eq!(removed.changes.len(), 2);
    }
    let disabled = toggle(
        &lines("127.0.0.1 example.com www.example.com # keep"),
        &names(&["example.com", "www.example.com"]),
    )
    .unwrap();
    assert_eq!(
        disabled.lines,
        lines("# 127.0.0.1 example.com www.example.com # keep")
    );
}

#[test]
fn pairing_preserves_single_labels_and_legacy_names_and_rejects_invalid_generated_names() {
    for name in ["localhost", "printer", "www.localhost", "www.127.0.0.1"] {
        let added = add(&[], "127.0.0.1", &names(&[name]), false).unwrap();
        assert_eq!(added.lines, lines(&format!("127.0.0.1\t{name}")));
    }
    for name in [
        "legacy_name.local",
        "www.legacy_name.local",
        "例.local",
        "example.com.",
    ] {
        let original = lines(&format!("127.0.0.1 {name}"));
        assert!(remove(&original, &names(&[name])).unwrap().lines.is_empty());
        assert_eq!(toggle(&original, &names(&[name])).unwrap().changes.len(), 1);
    }
    let longest = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    assert!(valid_hostname(&longest));
    assert!(add(&[], "127.0.0.1", std::slice::from_ref(&longest), false).is_err());
    let original = lines(&format!("127.0.0.1 {longest}"));
    assert!(remove(&original, &[longest]).unwrap().lines.is_empty());
}
