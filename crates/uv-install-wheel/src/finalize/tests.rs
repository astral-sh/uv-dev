use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::Result;
use assert_fs::TempDir;
use data_encoding::BASE64URL_NOPAD;
use sha2::{Digest, Sha256, Sha512};

use uv_distribution_filename::WheelFilename;
use uv_fs::Simplified;
use uv_pypi_types::Scheme;

use super::{RecordUpdate, finalize_scripts};
use crate::wheel::{format_shebang, write_record};
use crate::{InstallState, Layout, LinkMode, RecordEntry, install_wheel, read_record};

fn layout(root: &Path) -> Result<Layout> {
    let scripts = root.join(if cfg!(windows) { "Scripts" } else { "bin" });
    let site_packages = root.join("lib/site-packages");
    fs_err::create_dir_all(&scripts)?;
    fs_err::create_dir_all(&site_packages)?;
    let executable = scripts.join(format!("python{}", std::env::consts::EXE_SUFFIX));
    fs_err::write(&executable, b"interpreter placeholder")?;
    #[cfg(windows)]
    fs_err::write(
        scripts.join("pythonw.exe"),
        b"windowed interpreter placeholder",
    )?;
    Ok(Layout {
        sys_executable: executable,
        python_version: (3, 12),
        os_name: if cfg!(windows) { "nt" } else { "posix" }.into(),
        scheme: Scheme {
            purelib: site_packages.clone(),
            platlib: site_packages,
            scripts,
            data: root.to_path_buf(),
            include: root.join("include"),
        },
    })
}

fn hash(contents: &[u8]) -> String {
    format!(
        "sha256={}",
        BASE64URL_NOPAD.encode(&Sha256::digest(contents))
    )
}

fn install(
    root: &Path,
    layout: &Layout,
    name: &str,
    entrypoints: &str,
    scripts: &[(&str, &[u8])],
) -> Result<PathBuf> {
    let wheel = root.join(name);
    let prefix = format!("{name}-1.0.0");
    let metadata_dir = format!("{prefix}.dist-info");
    fs_err::create_dir_all(wheel.join(&metadata_dir))?;
    let metadata = format!("Metadata-Version: 2.3\nName: {name}\nVersion: 1.0.0\n");
    let mut record = Vec::new();
    for (name, contents) in [
        ("METADATA", metadata.as_bytes()),
        (
            "WHEEL",
            b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n".as_slice(),
        ),
        ("entry_points.txt", entrypoints.as_bytes()),
    ] {
        let path = format!("{metadata_dir}/{name}");
        fs_err::write(wheel.join(&path), contents)?;
        record.push(RecordEntry {
            path,
            hash: Some(hash(contents)),
            size: Some(contents.len() as u64),
        });
    }
    for (name, contents) in scripts {
        let path = format!("{prefix}.data/scripts/{name}");
        let absolute = wheel.join(&path);
        fs_err::create_dir_all(absolute.parent().expect("script has a parent"))?;
        fs_err::write(&absolute, contents)?;
        record.push(RecordEntry {
            path,
            hash: Some(hash(contents)),
            size: Some(contents.len() as u64),
        });
    }
    record.push(RecordEntry {
        path: format!("{metadata_dir}/RECORD"),
        hash: None,
        size: None,
    });
    write_record(&wheel, &prefix, record)?;
    install_wheel::<(), ()>(
        layout,
        false,
        &wheel,
        &WheelFilename::from_str(&format!("{prefix}-py3-none-any.whl"))?,
        None,
        None,
        None,
        None,
        false,
        LinkMode::Copy,
        &InstallState::default(),
    )?;
    Ok(layout.scheme.purelib.join(metadata_dir))
}

fn entrypoint(layout: &Layout, name: &str) -> PathBuf {
    layout.scheme.scripts.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    })
}

fn assert_record(layout: &Layout, dist_info: &Path, path: &Path) -> Result<()> {
    let contents = fs_err::read(path)?;
    let relative = pathdiff::diff_paths(path, &layout.scheme.purelib).expect("script is relative");
    let record = read_record(fs_err::File::open(dist_info.join("RECORD"))?)?;
    let entry = record
        .iter()
        .find(|entry| entry.path == relative.portable_display().to_string())
        .expect("script is recorded");
    assert_eq!(entry.hash.as_deref(), Some(hash(&contents).as_str()));
    assert_eq!(entry.size, Some(contents.len() as u64));
    Ok(())
}

#[test]
fn generated_launchers_and_records_use_the_final_interpreter() -> Result<()> {
    let root = TempDir::new()?;
    #[cfg(unix)]
    let staged = root.path().join(format!("staged-{}", "x".repeat(128)));
    #[cfg(windows)]
    let staged = root.path().join("staged environment");
    let layout = layout(&staged)?;
    let dist_info = install(
        root.path(),
        &layout,
        "launchers",
        "[console_scripts]\nhello = hello:main\n[gui_scripts]\nwindow = window:main\n",
        &[],
    )?;
    let final_executable = root.path().join("final/bin").join(
        layout
            .sys_executable
            .file_name()
            .expect("executable has a name"),
    );
    let console = entrypoint(&layout, "hello");
    let gui = entrypoint(&layout, "window");
    let original = fs_err::read(&console)?;
    let record = dist_info.join("RECORD");
    let original_record = fs_err::read(&record)?;
    let script_alias = root.path().join("cached-script");
    let record_alias = root.path().join("cached-record");
    fs_err::hard_link(&console, &script_alias)?;
    fs_err::hard_link(&record, &record_alias)?;
    let permissions = fs_err::metadata(&console)?.permissions();

    finalize_scripts(&layout, &final_executable, &dist_info, &[])?;

    assert_eq!(fs_err::read(&script_alias)?, original);
    assert_eq!(fs_err::read(&record_alias)?, original_record);
    assert_eq!(fs_err::metadata(&console)?.permissions(), permissions);
    assert_record(&layout, &dist_info, &console)?;
    assert_record(&layout, &dist_info, &gui)?;
    #[cfg(unix)]
    {
        assert!(original.starts_with(b"#!/bin/sh\n"));
        let prefix = format_shebang(&final_executable, &layout.os_name, false);
        assert!(fs_err::read(&console)?.starts_with(prefix.as_bytes()));
        assert!(fs_err::read(&gui)?.starts_with(prefix.as_bytes()));
    }
    #[cfg(windows)]
    {
        let console =
            uv_trampoline_builder::Launcher::try_from_path(&console)?.expect("console launcher");
        let gui = uv_trampoline_builder::Launcher::try_from_path(&gui)?.expect("GUI launcher");
        assert_eq!(console.python_path, final_executable);
        assert_eq!(
            gui.python_path,
            final_executable.with_file_name("pythonw.exe")
        );
    }
    let finalized = fs_err::read(&record)?;
    finalize_scripts(&layout, &final_executable, &dist_info, &[])?;
    assert_eq!(fs_err::read(&record)?, finalized);
    Ok(())
}

#[test]
fn rewritten_prefix_retains_the_binary_script_body() -> Result<()> {
    let root = TempDir::new()?;
    let layout = layout(&root.path().join("staged environment"))?;
    let mut body = b"binary\0\xff payload containing ".to_vec();
    body.extend_from_slice(layout.sys_executable.to_string_lossy().as_bytes());
    let mut source = b"#!python\n".to_vec();
    source.extend_from_slice(&body);
    let dist_info = install(
        root.path(),
        &layout,
        "binary",
        "",
        &[("payload.py", &source)],
    )?;
    let path = layout.scheme.scripts.join("payload.py");
    let final_executable = root.path().join("final python");

    finalize_scripts(&layout, &final_executable, &dist_info, &[])?;

    let newline = if cfg!(windows) { "\r\n" } else { "\n" };
    let prefix = format!(
        "{}{newline}",
        format_shebang(&final_executable, &layout.os_name, false)
    );
    let contents = fs_err::read(&path)?;
    assert_eq!(
        contents.strip_prefix(prefix.as_bytes()),
        Some(body.as_slice())
    );
    assert_record(&layout, &dist_info, &path)?;
    Ok(())
}

#[test]
fn arbitrary_scripts_and_modified_launchers_are_retained() -> Result<()> {
    let root = TempDir::new()?;
    let layout = layout(&root.path().join("staged"))?;
    let native = b"\x7fELF\0\xff unchanged";
    let text = format!(
        "#!/bin/sh\nprintf '%s' '{}'\n",
        layout.sys_executable.display()
    );
    let dist_info = install(
        root.path(),
        &layout,
        "custom",
        "[console_scripts]\nhello = hello:main\n",
        &[("native", native), ("ordinary", text.as_bytes())],
    )?;
    let mut custom = fs_err::read(entrypoint(&layout, "hello"))?;
    *custom.last_mut().expect("launcher is not empty") ^= 0xff;
    fs_err::write(entrypoint(&layout, "hello"), &custom)?;
    let record = fs_err::read(dist_info.join("RECORD"))?;

    finalize_scripts(&layout, &root.path().join("final/python"), &dist_info, &[])?;

    assert_eq!(fs_err::read(entrypoint(&layout, "hello"))?, custom);
    assert_eq!(fs_err::read(layout.scheme.scripts.join("native"))?, native);
    assert_eq!(
        fs_err::read(layout.scheme.scripts.join("ordinary"))?,
        text.as_bytes()
    );
    assert_eq!(fs_err::read(dist_info.join("RECORD"))?, record);
    Ok(())
}

#[test]
fn a_distribution_does_not_finalize_another_providers_scripts() -> Result<()> {
    let root = TempDir::new()?;
    let layout = layout(&root.path().join("staged"))?;
    let first = install(
        root.path(),
        &layout,
        "first",
        "[console_scripts]\nshared = first:main\n",
        &[("shared.py", b"#!python\nfirst provider\n")],
    )?;
    let second = install(
        root.path(),
        &layout,
        "second",
        "[console_scripts]\nshared = second:main\n",
        &[("shared.py", b"#!python\nsecond provider\n")],
    )?;
    let launcher = entrypoint(&layout, "shared");
    let data = layout.scheme.scripts.join("shared.py");
    let original_launcher = fs_err::read(&launcher)?;
    let original_data = fs_err::read(&data)?;
    let original_record = fs_err::read(first.join("RECORD"))?;
    let final_executable = root.path().join("final/python");

    finalize_scripts(&layout, &final_executable, &first, &[])?;
    assert_eq!(fs_err::read(&launcher)?, original_launcher);
    assert_eq!(fs_err::read(&data)?, original_data);
    assert_eq!(fs_err::read(first.join("RECORD"))?, original_record);
    finalize_scripts(&layout, &final_executable, &second, &[])?;
    assert_record(&layout, &second, &launcher)?;
    assert_record(&layout, &second, &data)?;
    let finalized = fs_err::read(&launcher)?;
    finalize_scripts(&layout, &final_executable, &first, &[])?;
    assert_eq!(fs_err::read(&launcher)?, finalized);
    assert_eq!(fs_err::read(first.join("RECORD"))?, original_record);
    Ok(())
}

#[test]
fn identical_providers_update_both_records() -> Result<()> {
    let root = TempDir::new()?;
    let layout = layout(&root.path().join("staged"))?;
    let first = install(
        root.path(),
        &layout,
        "first",
        "[console_scripts]\nshared = common:main\n",
        &[("shared.py", b"#!python\nshared payload\n")],
    )?;
    let second = install(
        root.path(),
        &layout,
        "second",
        "[console_scripts]\nshared = common:main\n",
        &[("shared.py", b"#!python\nshared payload\n")],
    )?;
    let final_executable = root.path().join("final/python");

    finalize_scripts(&layout, &final_executable, &first, &[])?;
    finalize_scripts(&layout, &final_executable, &second, &[])?;

    assert_record(&layout, &first, &entrypoint(&layout, "shared"))?;
    assert_record(&layout, &second, &entrypoint(&layout, "shared"))?;
    assert_record(&layout, &first, &layout.scheme.scripts.join("shared.py"))?;
    assert_record(&layout, &second, &layout.scheme.scripts.join("shared.py"))?;
    Ok(())
}

#[cfg(windows)]
#[test]
fn windowed_data_scripts_retain_crlf_payload_and_pythonw() -> Result<()> {
    let root = TempDir::new()?;
    let layout = layout(&root.path().join("staged"))?;
    let dist_info = install(
        root.path(),
        &layout,
        "windowed",
        "",
        &[("windowed.py", b"#!pythonw\r\nbody\0\xff")],
    )?;
    let path = layout.scheme.scripts.join("windowed.py");
    let final_executable = root.path().join("final/python.exe");
    let old_prefix = format!(
        "{}\r\n",
        format_shebang(
            layout.sys_executable.with_file_name("pythonw.exe"),
            "nt",
            false
        )
    );
    let original = fs_err::read(&path)?;
    let body = original
        .strip_prefix(old_prefix.as_bytes())
        .expect("installer generated pythonw prefix");

    finalize_scripts(&layout, &final_executable, &dist_info, &[])?;

    let prefix = format!(
        "{}\r\n",
        format_shebang(final_executable.with_file_name("pythonw.exe"), "nt", false)
    );
    assert_eq!(
        fs_err::read(&path)?.strip_prefix(prefix.as_bytes()),
        Some(body)
    );
    assert_record(&layout, &dist_info, &path)?;
    Ok(())
}

#[test]
fn generated_file_updates_reconcile_only_matching_provider_records() -> Result<()> {
    let root = TempDir::new()?;
    let layout = layout(&root.path().join("staged"))?;
    let before = b"# generated activation\nVIRTUAL_ENV='staged'\n";
    let after = b"# generated activation\nVIRTUAL_ENV='published'\n";
    let foreign = install(
        root.path(),
        &layout,
        "foreign",
        "",
        &[("activate", b"unrelated activation")],
    )?;
    let owner = install(root.path(), &layout, "owner", "", &[("activate", before)])?;
    let shared = install(root.path(), &layout, "shared", "", &[("activate", before)])?;
    let path = layout.scheme.scripts.join("activate");
    let foreign_record = fs_err::read(foreign.join("RECORD"))?;

    // Wheel RECORD files may use a different SHA2 hash and omit the optional size.
    let mut owner_record = read_record(fs_err::File::open(owner.join("RECORD"))?)?;
    let relative = pathdiff::diff_paths(&path, &layout.scheme.purelib).expect("script is relative");
    let entry = owner_record
        .iter_mut()
        .find(|entry| entry.path == relative.portable_display().to_string())
        .expect("activation is recorded");
    entry.hash = Some(format!(
        "sha512={}",
        BASE64URL_NOPAD.encode(&Sha512::digest(before))
    ));
    entry.size = None;
    write_record(&layout.scheme.purelib, "owner-1.0.0", owner_record)?;
    fs_err::write(&path, after)?;
    let updates = [RecordUpdate {
        path: &path,
        before,
        after,
    }];
    let final_executable = root.path().join("final/python");

    finalize_scripts(&layout, &final_executable, &foreign, &updates)?;
    finalize_scripts(&layout, &final_executable, &owner, &updates)?;
    finalize_scripts(&layout, &final_executable, &shared, &updates)?;

    assert_eq!(fs_err::read(foreign.join("RECORD"))?, foreign_record);
    assert_record(&layout, &owner, &path)?;
    assert_record(&layout, &shared, &path)?;
    assert_eq!(fs_err::read(&path)?, after);
    let finalized_record = fs_err::read(owner.join("RECORD"))?;
    finalize_scripts(&layout, &final_executable, &owner, &updates)?;
    assert_eq!(fs_err::read(owner.join("RECORD"))?, finalized_record);

    fs_err::write(&path, b"later custom activation")?;
    finalize_scripts(&layout, &final_executable, &owner, &updates)?;
    assert_eq!(fs_err::read(owner.join("RECORD"))?, finalized_record);
    assert_eq!(fs_err::read(&path)?, b"later custom activation");
    Ok(())
}

#[cfg(unix)]
#[test]
fn recorded_script_symlinks_are_not_followed() -> Result<()> {
    let root = TempDir::new()?;
    let layout = layout(&root.path().join("staged"))?;
    let dist_info = install(
        root.path(),
        &layout,
        "linked",
        "",
        &[("linked.py", b"#!python\nunchanged\n")],
    )?;
    let path = layout.scheme.scripts.join("linked.py");
    let external = root.path().join("external.py");
    fs_err::rename(&path, &external)?;
    let contents = fs_err::read(&external)?;
    fs_err::os::unix::fs::symlink(&external, &path)?;
    let record = fs_err::read(dist_info.join("RECORD"))?;

    finalize_scripts(&layout, &root.path().join("final/python"), &dist_info, &[])?;

    assert_eq!(fs_err::read_link(&path)?, external);
    assert_eq!(fs_err::read(&external)?, contents);
    assert_eq!(fs_err::read(dist_info.join("RECORD"))?, record);
    Ok(())
}

#[test]
fn generated_file_updates_require_the_current_final_bytes() -> Result<()> {
    let root = TempDir::new()?;
    let layout = layout(&root.path().join("staged"))?;
    let before = b"# generated activation before";
    let after = b"# generated activation after";
    let owner = install(root.path(), &layout, "owner", "", &[("activate", before)])?;
    let path = layout.scheme.scripts.join("activate");
    let original_record = fs_err::read(owner.join("RECORD"))?;
    let mut custom = after.to_vec();
    custom[0] ^= 1;
    fs_err::write(&path, &custom)?;
    let updates = [RecordUpdate {
        path: &path,
        before,
        after,
    }];

    finalize_scripts(&layout, &root.path().join("final/python"), &owner, &updates)?;

    assert_eq!(fs_err::read(owner.join("RECORD"))?, original_record);
    assert_eq!(fs_err::read(&path)?, custom);
    Ok(())
}
