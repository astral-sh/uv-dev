//! Normalize common Windows-only dependencies in command snapshots.

use regex::{Captures, Regex};

use crate::WindowsFilters;

/// Remove each displayed Windows-only dependency once from operation counts.
///
/// This is a heuristic: a snapshot does not identify which displayed packages were prepared or
/// checked. Universal resolutions include Windows dependencies on every platform, so their
/// resolution counts remain intact. Counts that would reach zero remain intact because the removed
/// dependency may belong to a different phase.
pub(super) fn normalize(mut snapshot: String, mode: Option<WindowsFilters>) -> String {
    let Some(mode) = mode else {
        return snapshot;
    };
    // Group overlapping formats by dependency so appearances in both streams count only once.
    let dependencies = [
        [
            r"(?m)^( ?[-+~] ?)?colorama==\d+(\.\d+)+( [\\]\n\s+--hash=.*)?\n(\s+# via .*\n)?",
            r"(?m)^( ?[-+~] ?)?colorama==\d+(\.\d+)+(\s+[-+~]?\s+# via .*)?\n",
        ],
        [
            r"(?m)^( ?[-+~] ?)?tzdata==\d+(\.\d+)+( [\\]\n\s+--hash=.*)?\n(\s+# via .*\n)?",
            r"(?m)^( ?[-+~] ?)?tzdata==\d+(\.\d+)+(\s+[-+~]?\s+# via .*)?\n",
        ],
    ];
    let mut removed_packages = 0;
    for formats in dependencies {
        let mut removed = false;
        for pattern in formats {
            let regex = Regex::new(pattern).expect("valid Windows dependency pattern");
            if regex.is_match(&snapshot) {
                snapshot = regex.replace_all(&snapshot, "").into_owned();
                removed = true;
            }
        }
        removed_packages += u64::from(removed);
    }
    if removed_packages == 0 {
        return snapshot;
    }

    let normalize_resolution = match mode {
        WindowsFilters::Platform => true,
        WindowsFilters::Universal => false,
    };
    let summary = Regex::new(
        r"(?m)^(Resolved|Prepared|Installed|Checked|Uninstalled) ([0-9]+) packages?((?: without build isolation)?(?: in [^\n]+)?)$",
    )
    .expect("valid operation summary pattern");
    summary
        .replace_all(&snapshot, |captures: &Captures<'_>| {
            if &captures[1] == "Resolved" && !normalize_resolution {
                return captures[0].to_string();
            }
            let Ok(count) = captures[2].parse::<u64>() else {
                return captures[0].to_string();
            };
            let Some(count) = count.checked_sub(removed_packages) else {
                return captures[0].to_string();
            };
            if count == 0 {
                return captures[0].to_string();
            }
            format!(
                "{} {count} package{}{}",
                &captures[1],
                if count == 1 { "" } else { "s" },
                &captures[3],
            )
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::normalize;
    use crate::WindowsFilters;

    #[test]
    fn count_boundaries() {
        let output = indoc! {"
            exit_code: 0 (success)
            ----- stderr -----
            Resolved 200 packages in [TIME]
            Prepared 21 packages in [TIME]
            Installed 20 packages in [TIME]
            Checked 1 package in [TIME]
            Uninstalled 0 packages in [TIME]
             + colorama==0.4.6
            Other 200 packages and version 21 remain unchanged
        "};
        insta::assert_snapshot!(normalize(output.to_string(), Some(WindowsFilters::Platform)), @"
        exit_code: 0 (success)
        ----- stderr -----
        Resolved 199 packages in [TIME]
        Prepared 20 packages in [TIME]
        Installed 19 packages in [TIME]
        Checked 1 package in [TIME]
        Uninstalled 0 packages in [TIME]
        Other 200 packages and version 21 remain unchanged
        ");
    }

    #[test]
    fn repeated_dependencies_across_streams() {
        let output = indoc! {"
            exit_code: 0 (success)
            ----- stdout -----
            colorama==0.4.6
                # via click
            tzdata==2024.1
                # via pandas
            click==8.1.7

            ----- stderr -----
            Resolved 3 packages in [TIME]
            Prepared 3 packages in [TIME]
            Installed 3 packages in [TIME]
             + colorama==0.4.6
             + tzdata==2024.1
             + click==8.1.7
        "};
        insta::assert_snapshot!(normalize(output.to_string(), Some(WindowsFilters::Platform)), @"
        exit_code: 0 (success)
        ----- stdout -----
        click==8.1.7

        ----- stderr -----
        Resolved 1 package in [TIME]
        Prepared 1 package in [TIME]
        Installed 1 package in [TIME]
         + click==8.1.7
        ");
    }

    #[test]
    fn universal_resolution_count() {
        let output = indoc! {"
            Resolved 21 packages in [TIME]
            Prepared 2 packages in [TIME]
            Installed 2 packages in [TIME]
             + colorama==0.4.6
             + click==8.1.7
        "};
        insta::assert_snapshot!(normalize(output.to_string(), Some(WindowsFilters::Universal)), @"
        Resolved 21 packages in [TIME]
        Prepared 1 package in [TIME]
        Installed 1 package in [TIME]
         + click==8.1.7
        ");
    }

    #[test]
    fn preparation_without_build_isolation() {
        let output = indoc! {"
            Resolved 2 packages in [TIME]
            Prepared 2 packages without build isolation in [TIME]
            Installed 2 packages in [TIME]
             + colorama==0.4.6
             + click==8.1.7
            note: Prepared 2 packages without build isolation in [TIME]
        "};
        insta::assert_snapshot!(normalize(output.to_string(), Some(WindowsFilters::Platform)), @"
        Resolved 1 package in [TIME]
        Prepared 1 package without build isolation in [TIME]
        Installed 1 package in [TIME]
         + click==8.1.7
        note: Prepared 2 packages without build isolation in [TIME]
        ");
    }

    #[test]
    fn cached_and_replaced_packages_have_separate_phases() {
        let output = indoc! {"
            Resolved 3 packages in [TIME]
            Prepared 1 package in [TIME]
            Uninstalled 1 package in [TIME]
            Installed 3 packages in [TIME]
             + colorama==0.4.6
             + click==8.1.7
             ~ project==1.0.0
        "};
        insta::assert_snapshot!(normalize(output.to_string(), Some(WindowsFilters::Platform)), @"
        Resolved 2 packages in [TIME]
        Prepared 1 package in [TIME]
        Uninstalled 1 package in [TIME]
        Installed 2 packages in [TIME]
         + click==8.1.7
         ~ project==1.0.0
        ");
    }

    #[test]
    fn disabled() {
        let output = "Resolved 21 packages in [TIME]\n + colorama==0.4.6\n";
        assert_eq!(normalize(output.to_string(), None), output);
    }

    #[test]
    fn dependency_names_inside_other_lines() {
        let output = indoc! {"
            Resolved 21 packages in [TIME]
            notcolorama==0.4.6
            error: colorama==0.4.6
        "};
        assert_eq!(
            normalize(output.to_string(), Some(WindowsFilters::Platform)),
            output
        );
    }
}
