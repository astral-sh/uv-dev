use std::path::Path;
use std::process::Command;

use anyhow::Result;
use assert_fs::fixture::{ChildPath, FileWriteStr, PathChild, PathCreateDir};
use indoc::indoc;

use uv_static::EnvVars;
use uv_test::{TestContext, uv_snapshot};

fn show(context: &TestContext, target: &Path) -> Command {
    let mut command = context.pip_show();
    command
        .arg("--target")
        .arg(target)
        .arg("--offline")
        .arg("--no-config")
        .env(EnvVars::UV_NO_BUILD, "1")
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_ENTERPRISE_TOKEN")
        .env_remove("GITHUB_ENTERPRISE_TOKEN");
    command
}

fn write_dist_info(
    target: &ChildPath,
    name: &str,
    metadata: impl AsRef<[u8]>,
) -> Result<ChildPath> {
    let distribution = target.child(format!("{name}-1.0.0.dist-info"));
    distribution.create_dir_all()?;
    fs_err::write(distribution.child("METADATA"), metadata)?;
    Ok(distribution)
}

#[test]
fn show_descriptive_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    let alpha = write_dist_info(
        &target,
        "alpha_thing",
        indoc! {r#"
            Metadata-Version: 2.4
            Name: Alpha.Thing
            Version: 9.9.9
            Summary: An authored metadata fixture
            Home-page: https://example.invalid/alpha
            Author: First Author,
              Second Author
            Author-email: author@example.invalid
            License: Ignored legacy license
            License-Expression: MIT OR Apache-2.0
            Requires-Dist: bravo
            Requires-Dist: Bravo; python_version >= "3"
            Requires-Dist: hidden; python_version < "2"
        "#},
    )?;
    alpha
        .child("RECORD")
        .write_str("alpha.py,,\nalpha_thing-1.0.0.dist-info/METADATA,,\n")?;
    write_dist_info(
        &target,
        "bravo",
        indoc! {r"
            Metadata-Version: 2.1
            Name: bravo
            Version: 1.0.0
            License: BSD-3-Clause
            Requires-Dist: alpha-thing
        "},
    )?;

    uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("Bravo")
        .arg("Alpha_Thing")
        .arg("alpha-thing")
        .arg("missing-package"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: alpha-thing
    Version: 1.0.0
    Summary: An authored metadata fixture
    Home-page: https://example.invalid/alpha
    Author: First Author, Second Author
    Author-email: author@example.invalid
    License-Expression: MIT OR Apache-2.0
    Location: [TEMP_DIR]/target
    Requires: bravo
    Required-by: bravo
    ---
    Name: bravo
    Version: 1.0.0
    License: BSD-3-Clause
    Location: [TEMP_DIR]/target
    Requires: alpha-thing
    Required-by: alpha-thing

    ----- stderr -----
    warning: Package(s) not found for: missing-package
    ");

    uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("alpha-thing")
        .arg("--files"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: alpha-thing
    Version: 1.0.0
    Summary: An authored metadata fixture
    Home-page: https://example.invalid/alpha
    Author: First Author, Second Author
    Author-email: author@example.invalid
    License-Expression: MIT OR Apache-2.0
    Location: [TEMP_DIR]/target
    Requires: bravo
    Required-by: bravo
    Files:
      alpha.py
      alpha_thing-1.0.0.dist-info/METADATA
    ");

    uv_snapshot!(show(&context, target.path()).arg("missing-package"), @"
    exit_code: 1 (failure)
    ----- stderr -----
    warning: Package(s) not found for: missing-package
    ");
    uv_snapshot!(show(&context, target.path()).arg("alpha-thing").arg("-q"), @"exit_code: 0 (success)");
    uv_snapshot!(show(&context, target.path()).arg("alpha-thing").arg("-qq"), @"exit_code: 0 (success)");
    Ok(())
}

#[test]
fn show_missing_and_empty_metadata_fields() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    write_dist_info(
        &target,
        "empty",
        indoc! {r"
            Metadata-Version: 2.4
            Name: empty
            Version: 1.0.0
            Summary:
            Home-page: UNKNOWN
            Author:
            Author-email: UNKNOWN
            License-Expression:
            License: MIT
        "},
    )?;
    write_dist_info(
        &target,
        "missing",
        "Metadata-Version: 2.1\nName: missing\nVersion: 1.0.0\nProject-URL: Homepage, https://example.invalid/not-a-legacy-home-page\n",
    )?;
    write_dist_info(
        &target,
        "unknown",
        indoc! {r"
            Metadata-Version: 2.4
            Name: unknown
            Version: 1.0.0
            Summary: UNKNOWN
            Home-page:
            Author: UNKNOWN
            Author-email:
            License-Expression: UNKNOWN
            License: UNKNOWN
        "},
    )?;

    uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("unknown")
        .arg("missing")
        .arg("empty"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: empty
    Version: 1.0.0
    License: MIT
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    ---
    Name: missing
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    ---
    Name: unknown
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    ");
    Ok(())
}

#[test]
fn show_full_metadata_errors_are_best_effort() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    write_dist_info(
        &target,
        "missing_version",
        "Name: missing-version\nVersion: 1.0.0\nSummary: Unavailable\nRequires-Dist: dependency\n",
    )?;
    write_dist_info(
        &target,
        "invalid_description",
        b"Metadata-Version: 2.1\nName: invalid-description\nVersion: 1.0.0\nSummary: Unavailable\nRequires-Dist: dependency\n\n\xff",
    )?;
    write_dist_info(
        &target,
        "import_overlap",
        indoc! {r"
            Metadata-Version: 2.5
            Name: import-overlap
            Version: 1.0.0
            Summary: Unavailable
            Import-Name: fixture
            Import-Namespace: fixture
            Requires-Dist: dependency
        "},
    )?;
    write_dist_info(
        &target,
        "invalid_requirement",
        "Metadata-Version: 2.1\nName: invalid-requirement\nVersion: 1.0.0\nSummary: Still available\nRequires-Dist: !\n",
    )?;
    target
        .child("missing_file-1.0.0.dist-info")
        .create_dir_all()?;

    uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("missing-version")
        .arg("invalid-description")
        .arg("import-overlap")
        .arg("invalid-requirement")
        .arg("missing-file"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: import-overlap
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires: dependency
    Required-by:
    ---
    Name: invalid-description
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires: dependency
    Required-by:
    ---
    Name: invalid-requirement
    Version: 1.0.0
    Summary: Still available
    Location: [TEMP_DIR]/target
    ---
    Name: missing-file
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    ---
    Name: missing-version
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires: dependency
    Required-by:
    ");
    Ok(())
}

#[test]
fn show_legacy_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    target.create_dir_all()?;
    target.child("egg_file-1.0.0.egg-info").write_str(
        "Metadata-Version: 1.0\nName: egg-file\nVersion: 1.0.0\nSummary: An egg-info file\n",
    )?;
    let egg_directory = target.child("egg_directory-1.0.0.egg-info");
    egg_directory.create_dir_all()?;
    egg_directory.child("PKG-INFO").write_str(
        "Metadata-Version: 1.0\nName: egg-directory\nVersion: 1.0.0\nSummary: An egg-info directory\n",
    )?;
    let editable = target.child("source/legacy_editable.egg-info");
    editable.create_dir_all()?;
    editable.child("PKG-INFO").write_str(
        "Metadata-Version: 1.0\nName: legacy-editable\nVersion: 1.0.0\nSummary: A legacy editable\n",
    )?;
    target
        .child("legacy_editable.egg-link")
        .write_str("source\n")?;
    let direct_url = write_dist_info(
        &target,
        "direct_url",
        "Metadata-Version: 2.1\nName: direct-url\nVersion: 1.0.0\nSummary: A direct URL\n",
    )?;
    direct_url
        .child("direct_url.json")
        .write_str(r#"{"url":"https://example.invalid/direct_url-1.0.0.whl","archive_info":{}}"#)?;

    uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("legacy-editable")
        .arg("egg-file")
        .arg("egg-directory")
        .arg("direct-url"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: direct-url
    Version: 1.0.0
    Summary: A direct URL
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    ---
    Name: egg-directory
    Version: 1.0.0
    Summary: An egg-info directory
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    ---
    Name: egg-file
    Version: 1.0.0
    Summary: An egg-info file
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    ---
    Name: legacy-editable
    Version: 1.0.0
    Summary: A legacy editable
    Location: [TEMP_DIR]/target/source
    Editable project location: [TEMP_DIR]/target/source
    Requires:
    Required-by:
    ");
    Ok(())
}

#[test]
fn show_metadata_strips_terminal_sequences() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    write_dist_info(
        &target,
        "terminal",
        concat!(
            "Metadata-Version: 2.4\nName: terminal\nVersion: 1.0.0\n",
            "Summary: \x1b[31mA summary\x1b[0m\x07\n",
            "Home-page: \x1b]8;;https://example.invalid\x1b\\https://example.invalid\x1b]8;;\x1b\\\n",
            "Author: First\n Second\n",
            "Author-email: \x1b[2Jauthor@example.invalid\n",
            "License-Expression: \x1b[31m\x1b[0m\n",
            "License: =?utf-8?Q?Line_one=0ALine_two?=\n",
        ),
    )?;

    let output = uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("terminal")
        .arg("--color=always"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: terminal
    Version: 1.0.0
    Summary: A summary
    Home-page: https://example.invalid
    Author: First Second
    Author-email: author@example.invalid
    License: Line one
    Line two
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    ");
    assert!(!output.stdout.contains(&b'\x1b'));
    assert!(!output.stdout.contains(&b'\x07'));
    Ok(())
}
