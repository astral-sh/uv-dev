use std::collections::BTreeMap;
use std::str::FromStr;

use anyhow::Result;
use data_encoding::BASE64URL_NOPAD;
use sha2::{Digest, Sha256};

use uv_distribution_filename::WheelFilename;
use uv_install_wheel::{InstallState, Layout, LinkMode, install_wheel, read_record};
use uv_pypi_types::Scheme;

/// Scripts shorter than the `#!python` probe must be installed without rewriting their bytes.
#[test]
fn install_short_data_scripts() -> Result<()> {
    let temp_dir = assert_fs::TempDir::new()?;
    let wheel = temp_dir.path().join("wheel");
    let environment = temp_dir.path().join("environment");
    let site_packages = environment.join("site-packages");
    let scripts = environment.join("bin");
    let dist_info = "short_data_scripts-1.0.0.dist-info";
    let data_dir = "short_data_scripts-1.0.0.data";

    fs_err::create_dir_all(wheel.join(dist_info))?;
    fs_err::create_dir_all(wheel.join(data_dir).join("scripts"))?;

    let mut files = BTreeMap::from([
        (
            format!("{dist_info}/METADATA"),
            b"Metadata-Version: 2.1\nName: short-data-scripts\nVersion: 1.0.0\n".to_vec(),
        ),
        (
            format!("{dist_info}/WHEEL"),
            b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n".to_vec(),
        ),
    ]);
    for length in 0..b"#!python".len() {
        files.insert(
            format!("{data_dir}/scripts/short-{length}"),
            b"#!python"[..length].to_vec(),
        );
    }
    files.insert(
        format!("{data_dir}/scripts/short-binary"),
        vec![0xff, 0, 0xfe, 0x80, 1, 2, 3],
    );

    let record_path = format!("{dist_info}/RECORD");
    let mut expected_record = BTreeMap::new();
    let mut record_writer = csv::WriterBuilder::new()
        .has_headers(false)
        .from_path(wheel.join(&record_path))?;
    for (path, contents) in &files {
        fs_err::write(wheel.join(path), contents)?;
        let hash = format!(
            "sha256={}",
            BASE64URL_NOPAD.encode(&Sha256::digest(contents))
        );
        let size = contents.len() as u64;
        record_writer.serialize((path, &hash, size))?;

        let installed_path = path
            .strip_prefix(&format!("{data_dir}/scripts/"))
            .map_or_else(|| path.clone(), |name| format!("../bin/{name}"));
        expected_record.insert(installed_path, (Some(hash), Some(size)));
    }
    record_writer.serialize((&record_path, None::<String>, None::<u64>))?;
    record_writer.flush()?;
    drop(record_writer);
    expected_record.insert(record_path.clone(), (None, None));

    let layout = Layout {
        sys_executable: scripts.join("python"),
        python_version: (3, 12),
        os_name: if cfg!(windows) { "nt" } else { "posix" }.to_string(),
        scheme: Scheme {
            purelib: site_packages.clone(),
            platlib: site_packages.clone(),
            scripts: scripts.clone(),
            data: environment.clone(),
            include: environment.join("include"),
        },
    };
    let filename = WheelFilename::from_str("short_data_scripts-1.0.0-py3-none-any.whl")?;
    install_wheel::<(), ()>(
        &layout,
        false,
        &wheel,
        &filename,
        None,
        None,
        None,
        None,
        false,
        LinkMode::Copy,
        &InstallState::default(),
    )?;

    for (path, contents) in &files {
        assert_eq!(fs_err::read(wheel.join(path))?, *contents);
        if let Some(name) = path.strip_prefix(&format!("{data_dir}/scripts/")) {
            assert_eq!(fs_err::read(scripts.join(name))?, *contents);
        }
    }
    assert!(!site_packages.join(data_dir).exists());

    let installed_record = read_record(fs_err::File::open(site_packages.join(record_path))?)?;
    assert_eq!(installed_record.len(), expected_record.len());
    let installed_record = installed_record
        .into_iter()
        .map(|entry| (entry.path, (entry.hash, entry.size)))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(installed_record, expected_record);
    Ok(())
}
