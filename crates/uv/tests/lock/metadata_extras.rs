use anyhow::Result;
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use indoc::indoc;

use uv_test::uv_snapshot;

/// Reordering or repeating backend extra headers does not change a freshly resolved lock.
#[test]
fn refreshed_extra_availability_is_a_set() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []
        dynamic = ["optional-dependencies"]

        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            root = Path(__file__).parent
            with (root / "metadata_calls").open("a") as calls:
                calls.write("called\n")
            directory = Path(metadata_directory) / "project-1.0.0.dist-info"
            directory.mkdir()
            metadata = "Metadata-Version: 2.3\nName: project\nVersion: 1.0.0\n"
            for extra in (root / "extras.txt").read_text().splitlines():
                metadata += f"Provides-Extra: {extra}\n"
            (directory / "METADATA").write_text(metadata)
            return directory.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    let extras = context.temp_dir.child("extras.txt");
    extras.write_str("zebra\nempty\nalpha\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    let original = context.read("uv.lock");
    let calls = context.read("metadata_calls").lines().count();

    extras.write_str("alpha\nzebra\nempty\nalpha\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert!(context.read("metadata_calls").lines().count() > calls);
    assert_eq!(original, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert_eq!(original, context.read("uv.lock"));

    // Empty extras remain selectable even when declaration metadata is omitted.
    context
        .sync()
        .args([
            "--offline",
            "--frozen",
            "--no-install-project",
            "--extra",
            "empty",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--frozen", "--no-install-project", "--extra", "missing",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `missing` is not defined in the `optional-dependencies` table for `project`
    ");

    extras.write_str("alpha\nzebra\nempty\nadded\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(original, context.read("uv.lock"));

    extras.write_str("alpha\nzebra\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(original, context.read("uv.lock"));
    Ok(())
}

/// Reordering or repeating backend extra headers does not change a freshly resolved lock.
#[test]
fn refreshed_extra_availability_is_a_set_normalized() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []
        dynamic = ["optional-dependencies"]

        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            root = Path(__file__).parent
            with (root / "metadata_calls").open("a") as calls:
                calls.write("called\n")
            directory = Path(metadata_directory) / "project-1.0.0.dist-info"
            directory.mkdir()
            metadata = "Metadata-Version: 2.3\nName: project\nVersion: 1.0.0\n"
            for extra in (root / "extras.txt").read_text().splitlines():
                metadata += f"Provides-Extra: {extra}\n"
            (directory / "METADATA").write_text(metadata)
            return directory.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    let extras = context.temp_dir.child("extras.txt");
    extras.write_str("zebra\nempty\nalpha\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lockfile-normalization"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    let original = context.read("uv.lock");
    let calls = context.read("metadata_calls").lines().count();

    extras.write_str("alpha\nzebra\nempty\nalpha\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lockfile-normalization"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert!(context.read("metadata_calls").lines().count() > calls);
    assert_eq!(original, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lockfile-normalization"])
        .arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert_eq!(original, context.read("uv.lock"));

    // Empty extras remain selectable even when declaration metadata is omitted.
    context
        .sync()
        .args([
            "--offline",
            "--frozen",
            "--no-install-project",
            "--extra",
            "empty",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--frozen", "--no-install-project", "--extra", "missing",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `missing` is not defined in the `optional-dependencies` table for `project`
    ");

    extras.write_str("alpha\nzebra\nempty\nadded\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lockfile-normalization"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(original, context.read("uv.lock"));

    extras.write_str("alpha\nzebra\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lockfile-normalization"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(original, context.read("uv.lock"));
    Ok(())
}

/// Reordering or repeating backend extra headers does not change a freshly resolved lock.
#[test]
fn refreshed_extra_availability_is_a_set_without_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []
        dynamic = ["optional-dependencies"]

        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            root = Path(__file__).parent
            with (root / "metadata_calls").open("a") as calls:
                calls.write("called\n")
            directory = Path(metadata_directory) / "project-1.0.0.dist-info"
            directory.mkdir()
            metadata = "Metadata-Version: 2.3\nName: project\nVersion: 1.0.0\n"
            for extra in (root / "extras.txt").read_text().splitlines():
                metadata += f"Provides-Extra: {extra}\n"
            (directory / "METADATA").write_text(metadata)
            return directory.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    let extras = context.temp_dir.child("extras.txt");
    extras.write_str("zebra\nempty\nalpha\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lock-without-metadata"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    let original = context.read("uv.lock");
    let calls = context.read("metadata_calls").lines().count();

    extras.write_str("alpha\nzebra\nempty\nalpha\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lock-without-metadata"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert!(context.read("metadata_calls").lines().count() > calls);
    assert_eq!(original, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lock-without-metadata"])
        .arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert_eq!(original, context.read("uv.lock"));

    // Empty extras remain selectable even when declaration metadata is omitted.
    context
        .sync()
        .args([
            "--offline",
            "--frozen",
            "--no-install-project",
            "--extra",
            "empty",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--frozen", "--no-install-project", "--extra", "missing",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `missing` is not defined in the `optional-dependencies` table for `project`
    ");

    extras.write_str("alpha\nzebra\nempty\nadded\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lock-without-metadata"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(original, context.read("uv.lock"));

    extras.write_str("alpha\nzebra\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lock-without-metadata"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(original, context.read("uv.lock"));
    Ok(())
}

/// Reordering or repeating backend extra headers does not change a freshly resolved lock.
#[test]
fn refreshed_extra_availability_is_a_set_normalized_without_metadata() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    context
        .temp_dir
        .child("pyproject.toml")
        .write_str(indoc! {r#"
        [project]
        name = "project"
        version = "1.0.0"
        requires-python = ">=3.12"
        dependencies = []
        dynamic = ["optional-dependencies"]

        [build-system]
        requires = []
        build-backend = "backend"
        backend-path = ["."]
    "#})?;
    context.temp_dir.child("backend.py").write_str(indoc! {r#"
        from pathlib import Path

        def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
            root = Path(__file__).parent
            with (root / "metadata_calls").open("a") as calls:
                calls.write("called\n")
            directory = Path(metadata_directory) / "project-1.0.0.dist-info"
            directory.mkdir()
            metadata = "Metadata-Version: 2.3\nName: project\nVersion: 1.0.0\n"
            for extra in (root / "extras.txt").read_text().splitlines():
                metadata += f"Provides-Extra: {extra}\n"
            (directory / "METADATA").write_text(metadata)
            return directory.name

        prepare_metadata_for_build_editable = prepare_metadata_for_build_wheel
    "#})?;
    let extras = context.temp_dir.child("extras.txt");
    extras.write_str("zebra\nempty\nalpha\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lockfile-normalization,lock-without-metadata"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    let original = context.read("uv.lock");
    let calls = context.read("metadata_calls").lines().count();

    extras.write_str("alpha\nzebra\nempty\nalpha\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lockfile-normalization,lock-without-metadata"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert!(context.read("metadata_calls").lines().count() > calls);
    assert_eq!(original, context.read("uv.lock"));
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lockfile-normalization,lock-without-metadata"])
        .arg("--refresh"), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    ");
    assert_eq!(original, context.read("uv.lock"));

    // Empty extras remain selectable even when declaration metadata is omitted.
    context
        .sync()
        .args([
            "--offline",
            "--frozen",
            "--no-install-project",
            "--extra",
            "empty",
        ])
        .assert()
        .success();
    uv_snapshot!(context.filters(), context.sync().args([
        "--offline", "--frozen", "--no-install-project", "--extra", "missing",
    ]), @"
    exit_code: 2 (failure)
    ----- stderr -----
    error: Extra `missing` is not defined in the `optional-dependencies` table for `project`
    ");

    extras.write_str("alpha\nzebra\nempty\nadded\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lockfile-normalization,lock-without-metadata"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(original, context.read("uv.lock"));

    extras.write_str("alpha\nzebra\n")?;
    uv_snapshot!(context.filters(), context.lock()
        .args(["--no-index", "--no-build-isolation"])
        .args(["--preview-features", "lockfile-normalization,lock-without-metadata"])
        .args(["--locked", "--refresh"]), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Resolved 1 package in [TIME]
    error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

    hint: To update the lockfile, run `uv lock`.
    ");
    assert_eq!(original, context.read("uv.lock"));
    Ok(())
}
