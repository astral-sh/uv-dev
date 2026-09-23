use std::path::Path;
use std::process::Command;

use anyhow::Result;
use assert_fs::fixture::{ChildPath, FileWriteBin, FileWriteStr, PathChild, PathCreateDir};
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

fn write_dist_info(target: &ChildPath, name: &str, metadata: impl AsRef<[u8]>) -> Result<()> {
    let distribution = target.child(format!("{name}-1.0.0.dist-info"));
    distribution.create_dir_all()?;
    fs_err::write(distribution.child("METADATA"), metadata)?;
    Ok(())
}

#[test]
fn show_duplicate_distributions_keep_requirements() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    for (directory, metadata) in [
        (
            "alpha_pkg-1.0.dist-info",
            "Metadata-Version: 2.1\nName: Alpha.Pkg\nVersion: 1.0\nRequires-Dist: bravo\nRequires-Dist: alpha-pkg\n",
        ),
        (
            "alpha_pkg-2.0.dist-info",
            "Metadata-Version: 2.1\nName: alpha-pkg\nVersion: 2.0\nRequires-Dist: charlie\nRequires-Dist: bravo; python_version < '2'\n",
        ),
        (
            "bravo-1.0.dist-info",
            "Metadata-Version: 2.1\nName: bravo\nVersion: 1.0\n",
        ),
        (
            "charlie-1.0.dist-info",
            "Metadata-Version: 2.1\nName: charlie\nVersion: 1.0\n",
        ),
        (
            "consumer-1.0.dist-info",
            "Metadata-Version: 2.1\nName: consumer\nVersion: 1.0\nRequires-Dist: alpha-pkg\n",
        ),
        (
            "consumer-2.0.dist-info",
            "Metadata-Version: 2.1\nName: consumer\nVersion: 2.0\nRequires-Dist: Alpha.Pkg\n",
        ),
    ] {
        let distribution = target.child(directory);
        distribution.create_dir_all()?;
        distribution.child("METADATA").write_str(metadata)?;
    }

    uv_snapshot!(context.filters(), show(&context, target.path()).arg("Alpha.Pkg"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: alpha-pkg
    Version: 1.0
    Location: [TEMP_DIR]/target
    Requires: alpha-pkg, bravo
    Required-by: consumer
    ---
    Name: alpha-pkg
    Version: 2.0
    Location: [TEMP_DIR]/target
    Requires: charlie
    Required-by: consumer
    ");

    uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("alpha-pkg")
        .arg("bravo")
        .arg("charlie"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: alpha-pkg
    Version: 1.0
    Location: [TEMP_DIR]/target
    Requires: alpha-pkg, bravo
    Required-by: consumer
    ---
    Name: alpha-pkg
    Version: 2.0
    Location: [TEMP_DIR]/target
    Requires: charlie
    Required-by: consumer
    ---
    Name: bravo
    Version: 1.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by: alpha-pkg
    ---
    Name: charlie
    Version: 1.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by: alpha-pkg
    ");
    Ok(())
}

#[test]
fn show_required_by_includes_duplicate_distributions() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    for (directory, metadata) in [
        (
            "consumer-1.0.dist-info",
            "Metadata-Version: 2.1\nName: consumer\nVersion: 1.0\nRequires-Dist: bravo\n",
        ),
        (
            "consumer-2.0.dist-info",
            "Metadata-Version: 2.1\nName: consumer\nVersion: 2.0\nRequires-Dist: charlie\n",
        ),
        (
            "zeta-1.0.dist-info",
            "Metadata-Version: 2.1\nName: zeta\nVersion: 1.0\nRequires-Dist: bravo\n",
        ),
        (
            "zeta-2.0.dist-info",
            "Metadata-Version: 2.1\nName: zeta\nVersion: 2.0\nRequires-Dist: bravo\n",
        ),
        (
            "bravo-1.0.dist-info",
            "Metadata-Version: 2.1\nName: bravo\nVersion: 1.0\n",
        ),
        (
            "charlie-1.0.dist-info",
            "Metadata-Version: 2.1\nName: charlie\nVersion: 1.0\n",
        ),
    ] {
        let distribution = target.child(directory);
        distribution.create_dir_all()?;
        distribution.child("METADATA").write_str(metadata)?;
    }

    uv_snapshot!(context.filters(), show(&context, target.path()).arg("bravo"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: bravo
    Version: 1.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by: consumer, zeta
    ");

    uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("bravo")
        .arg("charlie"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: bravo
    Version: 1.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by: consumer, zeta
    ---
    Name: charlie
    Version: 1.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by: consumer
    ");
    Ok(())
}

#[test]
fn show_duplicate_metadata_is_best_effort_per_distribution() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    for (directory, metadata) in [
        (
            "alpha-1.0.dist-info",
            "Metadata-Version: 2.1\nName: alpha\nVersion: 1.0\nRequires-Dist: !\n",
        ),
        (
            "alpha-3.0.dist-info",
            "Metadata-Version: 2.1\nName: alpha\nVersion: 3.0\nRequires-Dist: bravo\n",
        ),
    ] {
        let distribution = target.child(directory);
        distribution.create_dir_all()?;
        distribution.child("METADATA").write_str(metadata)?;
    }
    target.child("alpha-2.0.dist-info").create_dir_all()?;

    uv_snapshot!(context.filters(), show(&context, target.path()).arg("alpha"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: alpha
    Version: 1.0
    Location: [TEMP_DIR]/target
    ---
    Name: alpha
    Version: 2.0
    Location: [TEMP_DIR]/target
    ---
    Name: alpha
    Version: 3.0
    Location: [TEMP_DIR]/target
    Requires: bravo
    Required-by:
    ");
    Ok(())
}

#[test]
fn show_duplicate_reverse_dependency_preserves_read_errors() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    write_dist_info(
        &target,
        "bravo",
        "Metadata-Version: 2.1\nName: bravo\nVersion: 1.0.0\n",
    )?;
    write_dist_info(
        &target,
        "consumer",
        "Metadata-Version: 2.1\nName: consumer\nVersion: 1.0.0\nRequires-Dist: bravo\n",
    )?;
    let metadata = target.child("consumer-2.0.0.dist-info/METADATA");
    metadata.create_dir_all()?;
    assert_metadata_read_error(&context, target.path(), "bravo", metadata.path())?;
    Ok(())
}

#[test]
fn show_descriptive_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    write_dist_info(
        &target,
        "alpha_thing",
        indoc! {r"
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
        "},
    )?;

    uv_snapshot!(context.filters(), show(&context, target.path()).arg("alpha-thing"), @"
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
    Required-by:
    ");
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
        "Metadata-Version: 2.1\nName: missing\nVersion: 1.0.0\nLicense: BSD-3-Clause\nProject-URL: Homepage, https://example.invalid/not-a-legacy-home-page\n",
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
    License: BSD-3-Clause
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

fn assert_metadata_read_error(
    context: &TestContext,
    target: &Path,
    package: &str,
    metadata: &Path,
) -> Result<()> {
    let expected = fs_err::read(metadata).expect_err("the metadata path is a directory");
    assert_ne!(expected.kind(), std::io::ErrorKind::NotFound);

    let output = show(context, target).arg(package).output()?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        uv_test::apply_filters(String::from_utf8(output.stderr)?, context.filters()),
        uv_test::apply_filters(format!("error: {expected}\n"), context.filters()),
    );
    assert!(metadata.is_dir());
    Ok(())
}

#[test]
fn show_metadata_read_errors_are_not_optional() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    for (layout, metadata_path) in [
        ("wheel", "fixture-1.0.0.dist-info/METADATA"),
        ("legacy", "fixture-1.0.0.egg-info/PKG-INFO"),
    ] {
        let target = context.temp_dir.child(layout);
        let metadata = target.child(metadata_path);
        // Reading a directory fails even for a privileged user. The versioned metadata
        // directory is still discoverable without reading its metadata contents.
        metadata.create_dir_all()?;
        assert_metadata_read_error(&context, target.path(), "fixture", metadata.path())?;
    }
    Ok(())
}

#[test]
fn show_required_by_metadata_read_errors_are_not_optional() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    write_dist_info(
        &target,
        "fixture",
        "Metadata-Version: 2.1\nName: fixture\nVersion: 1.0.0\n",
    )?;
    write_dist_info(
        &target,
        "consumer",
        "Metadata-Version: 2.1\nName: consumer\nVersion: 1.0.0\nRequires-Dist: fixture\n",
    )?;
    let metadata = target.child("consumer-1.0.0.dist-info/METADATA");
    let output = show(&context, target.path()).arg("fixture").output()?;
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)?.contains("Required-by: consumer\n"));

    metadata.write_str("Metadata-Version: 2.1\nVersion: 1.0.0\n")?;
    let output = show(&context, target.path()).arg("fixture").output()?;
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)?.contains("Required-by:\n"));

    fs_err::remove_file(metadata.path())?;
    let output = show(&context, target.path()).arg("fixture").output()?;
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)?.contains("Required-by:\n"));

    metadata.create_dir_all()?;
    assert_metadata_read_error(&context, target.path(), "fixture", metadata.path())?;
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

    uv_snapshot!(context.filters(), show(&context, target.path()).arg("egg-file"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: egg-file
    Version: 1.0.0
    Summary: An egg-info file
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    ");
    Ok(())
}

#[test]
fn show_legacy_metadata_with_files() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    target.create_dir_all()?;
    target.child("egg_file-1.0.0.egg-info").write_str(
        "Metadata-Version: 1.0\nName: egg-file\nVersion: 1.0.0\nSummary: An egg-info file\n",
    )?;

    let directory = target.child("egg_directory-1.0.0.egg-info");
    directory.create_dir_all()?;
    directory.child("PKG-INFO").write_str(
        "Metadata-Version: 1.0\nName: egg-directory\nVersion: 1.0.0\nSummary: An egg-info directory\n",
    )?;

    let source = context.temp_dir.child("legacy-source");
    let editable = source.child("legacy_editable.egg-info");
    editable.create_dir_all()?;
    editable.child("PKG-INFO").write_str(
        "Metadata-Version: 1.0\nName: legacy-editable\nVersion: 1.0.0\nSummary: An editable egg\n",
    )?;
    target
        .child("legacy-editable.egg-link")
        .write_str(&format!("{}\n", source.path().display()))?;

    uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("legacy-editable")
        .arg("egg-file")
        .arg("egg-directory")
        .arg("--files"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: egg-directory
    Version: 1.0.0
    Summary: An egg-info directory
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:
    Cannot locate RECORD or installed-files.txt
    ---
    Name: egg-file
    Version: 1.0.0
    Summary: An egg-info file
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:
    Cannot locate RECORD or installed-files.txt
    ---
    Name: legacy-editable
    Version: 1.0.0
    Summary: An editable egg
    Location: [TEMP_DIR]/legacy-source
    Editable project location: [TEMP_DIR]/legacy-source
    Requires:
    Required-by:
    Files:
    Cannot locate RECORD or installed-files.txt
    ");
    Ok(())
}

#[test]
fn show_legacy_installed_file_paths() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    let metadata = target.child("legacy_paths-1.0.0.egg-info");
    metadata.create_dir_all()?;
    metadata
        .child("PKG-INFO")
        .write_str("Metadata-Version: 1.0\nName: legacy-paths\nVersion: 1.0.0\n")?;

    let package = target.child("package");
    package.create_dir_all()?;
    package.child("__init__.py").write_str("unchanged\n")?;
    let elsewhere = context.temp_dir.child("elsewhere");
    elsewhere.create_dir_all()?;
    elsewhere.child("marker").write_str("untouched\n")?;
    #[cfg(unix)]
    fs_err::os::unix::fs::symlink(elsewhere.path(), package.child("link").path())?;
    #[cfg(windows)]
    package.child("link").create_dir_all()?;

    let absolute = context.temp_dir.child("absolute-missing");
    let contents = format!(
        "\nPKG-INFO\r\n../package/__init__.py\r\n../../bin/legacy-script\n../package/link/../missing.py\n../package/file with space.py \n./../package/dot.py\n.\n./\n..\n../..\n{}\n\n",
        absolute.path().display()
    );
    metadata.child("installed-files.txt").write_str(&contents)?;
    let mut filters = context.filters();
    // Record the significant space before the standard snapshot filters trim line endings.
    filters.insert(0, (r"(?m)(file with space\.py) $", "$1[TRAILING-SPACE]"));
    uv_snapshot!(filters, show(&context, target.path()).arg("legacy-paths").arg("--files"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: legacy-paths
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:
      legacy_paths-1.0.0.egg-info/PKG-INFO
      package/__init__.py
      ../bin/legacy-script
      package/link/../missing.py
      package/file with space.py[TRAILING-SPACE]
      package/dot.py
      legacy_paths-1.0.0.egg-info
      legacy_paths-1.0.0.egg-info
      .
      ..
      [TEMP_DIR]/absolute-missing
    ");
    assert_eq!(
        fs_err::read_to_string(metadata.child("installed-files.txt"))?,
        contents
    );
    assert_eq!(
        fs_err::read_to_string(package.child("__init__.py"))?,
        "unchanged\n"
    );
    assert_eq!(
        fs_err::read_to_string(elsewhere.child("marker"))?,
        "untouched\n"
    );
    assert!(!absolute.path().exists());
    assert!(!package.child("missing.py").path().exists());
    Ok(())
}

#[test]
fn show_legacy_installed_files_for_directory_layouts() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    write_dist_info(
        &target,
        "recordless",
        "Metadata-Version: 2.1\nName: recordless\nVersion: 1.0.0\n",
    )?;
    target
        .child("recordless-1.0.0.dist-info/installed-files.txt")
        .write_str("METADATA\n../recordless.py\n")?;

    let source = context.temp_dir.child("legacy-source");
    let editable = source.child("legacy_editable.egg-info");
    editable.create_dir_all()?;
    editable
        .child("PKG-INFO")
        .write_str("Metadata-Version: 1.0\nName: legacy-editable\nVersion: 1.0.0\n")?;
    editable
        .child("installed-files.txt")
        .write_str("PKG-INFO\n../editable.py\n")?;
    target
        .child("legacy-editable.egg-link")
        .write_str(&format!("{}\n", source.path().display()))?;

    uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("recordless")
        .arg("legacy-editable")
        .arg("--files"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: legacy-editable
    Version: 1.0.0
    Location: [TEMP_DIR]/legacy-source
    Editable project location: [TEMP_DIR]/legacy-source
    Requires:
    Required-by:
    Files:
      legacy_editable.egg-info/PKG-INFO
      editable.py
    ---
    Name: recordless
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:
      recordless-1.0.0.dist-info/METADATA
      recordless.py
    ");
    Ok(())
}

#[test]
fn show_record_precedes_legacy_installed_files() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    for name in ["recorded", "empty-record", "broken-record"] {
        let directory_name = name.replace('-', "_");
        write_dist_info(
            &target,
            &directory_name,
            format!("Metadata-Version: 2.1\nName: {name}\nVersion: 1.0.0\n"),
        )?;
        target
            .child(format!(
                "{directory_name}-1.0.0.dist-info/installed-files.txt"
            ))
            .write_str("../legacy-only.py\n")?;
    }
    target
        .child("recorded-1.0.0.dist-info/RECORD")
        .write_str("recorded.py,,\n")?;
    target
        .child("empty_record-1.0.0.dist-info/RECORD")
        .write_str("")?;
    target
        .child("broken_record-1.0.0.dist-info/RECORD")
        .write_str("recorded.py,,not-a-size\n")?;

    uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("recorded")
        .arg("empty-record")
        .arg("--files"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: empty-record
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:
    ---
    Name: recorded
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:
      recorded.py
    ");
    uv_snapshot!(context.filters(), show(&context, target.path())
        .arg("broken-record")
        .arg("--files"), @"
    exit_code: 2 (failure)
    ----- stdout -----
    Name: broken-record
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:

    ----- stderr -----
    error: RECORD file is invalid
      cause: CSV deserialize error: record 0 (line: 1, byte: 0): field 2: invalid digit found in string
    ");
    Ok(())
}

#[test]
fn show_empty_legacy_installed_files() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    write_dist_info(
        &target,
        "empty",
        "Metadata-Version: 2.1\nName: empty\nVersion: 1.0.0\n",
    )?;
    target
        .child("empty-1.0.0.dist-info/installed-files.txt")
        .write_str("\n\r\n")?;
    uv_snapshot!(context.filters(), show(&context, target.path()).arg("empty").arg("--files"), @"
    exit_code: 0 (success)
    ----- stdout -----
    Name: empty
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:
    ");
    Ok(())
}

#[test]
fn show_invalid_legacy_installed_files() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    for name in ["invalid", "unreadable"] {
        write_dist_info(
            &target,
            name,
            format!("Metadata-Version: 2.1\nName: {name}\nVersion: 1.0.0\n"),
        )?;
    }
    target
        .child("invalid-1.0.0.dist-info/installed-files.txt")
        .write_binary(&[0xff])?;
    target
        .child("unreadable-1.0.0.dist-info/installed-files.txt")
        .create_dir_all()?;
    uv_snapshot!(context.filters(), show(&context, target.path()).arg("invalid").arg("--files"), @"
    exit_code: 2 (failure)
    ----- stdout -----
    Name: invalid
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:

    ----- stderr -----
    error: failed to read from file `[TEMP_DIR]/target/invalid-1.0.0.dist-info/installed-files.txt`: stream did not contain valid UTF-8
    ");
    let mut filters = context.filters();
    filters.push((
        r"Is a directory \(os error 21\)|Access is denied\. \(os error 5\)",
        "[DIRECTORY_ACCESS_ERROR]",
    ));
    #[cfg(not(windows))]
    uv_snapshot!(filters, show(&context, target.path()).arg("unreadable").arg("--files"), @"
    exit_code: 2 (failure)
    ----- stdout -----
    Name: unreadable
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:

    ----- stderr -----
    error: failed to read from file `[TEMP_DIR]/target/unreadable-1.0.0.dist-info/installed-files.txt`: [DIRECTORY_ACCESS_ERROR]
    ");
    #[cfg(windows)]
    uv_snapshot!(filters, show(&context, target.path()).arg("unreadable").arg("--files"), @"
    exit_code: 2 (failure)
    ----- stdout -----
    Name: unreadable
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:

    ----- stderr -----
    error: failed to open file `[TEMP_DIR]/target/unreadable-1.0.0.dist-info/installed-files.txt`: [DIRECTORY_ACCESS_ERROR]
    ");
    Ok(())
}

#[test]
#[cfg(windows)]
fn show_legacy_installed_files_windows_paths() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let target = context.temp_dir.child("target");
    write_dist_info(
        &target,
        "windows_paths",
        "Metadata-Version: 2.1\nName: windows-paths\nVersion: 1.0.0\n",
    )?;
    target
        .child("windows_paths-1.0.0.dist-info/installed-files.txt")
        .write_str(concat!(
            "Z:\\not-installed\\absolute.py\n",
            "Z:drive-relative.py\n",
            r"\rooted\link\..\missing.py",
            "\n",
            r"\\server\share\pkg\link\..\missing.py",
            "\n",
            r"\\?\C:\pkg\link\..\missing.py",
            "\n",
            r"\\?\UNC\server\share\pkg\link\..\missing.py",
            "\n",
        ))?;
    // The path snapshot must retain the verbatim prefixes that temporary-path filters remove.
    let mut filters = context.filters_without_standard_filters();
    filters.retain(|(pattern, _)| *pattern != r"\\\\\?\\");
    uv_snapshot!(filters, windows_filters=false, show(&context, target.path()).arg("windows-paths").arg("--files"), @r"
    exit_code: 0 (success)
    ----- stdout -----
    Name: windows-paths
    Version: 1.0.0
    Location: [TEMP_DIR]/target
    Requires:
    Required-by:
    Files:
      Z:/not-installed/absolute.py
      Z:drive-relative.py
      /rooted/link/../missing.py
      \\server\share/pkg/link/../missing.py
      \\?\C:/pkg/link/../missing.py
      \\?\UNC\server\share/pkg/link/../missing.py
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
