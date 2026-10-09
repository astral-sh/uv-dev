use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;

use anyhow::Result;
use assert_fs::TempDir;

use uv_pypi_types::Scheme;

use super::{finalize, render};

fn scheme() -> Scheme {
    let site_packages = if cfg!(windows) {
        "Lib/site-packages"
    } else {
        "lib/python3.12/site-packages"
    };
    Scheme {
        purelib: PathBuf::from(site_packages),
        platlib: PathBuf::from(site_packages),
        scripts: PathBuf::from(if cfg!(windows) { "Scripts" } else { "bin" }),
        data: PathBuf::new(),
        include: PathBuf::from("include"),
    }
}

fn install(root: &Path, scheme: &Scheme) -> Result<PathBuf> {
    let scripts = root.join(&scheme.scripts);
    fs_err::create_dir_all(&scripts)?;
    for (name, contents) in render(root, scheme, None, false)? {
        fs_err::write(scripts.join(name), contents)?;
    }
    Ok(scripts)
}

#[test]
fn generated_activators_follow_the_published_environment() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().join("staged environment");
    let destination = directory.path().join("published 'environment");
    let scheme = scheme();
    let scripts = install(&root, &scheme)?;
    let original = fs_err::read(scripts.join("activate"))?;
    let alias = directory.path().join("old-activation");
    fs_err::hard_link(scripts.join("activate"), &alias)?;
    let permissions = fs_err::metadata(scripts.join("activate"))?.permissions();

    let updates = finalize(&root, &scripts, &scheme, &destination, false)?;
    let activate = updates
        .iter()
        .find(|update| update.path() == scripts.join("activate"))
        .expect("activation update");
    assert_eq!(activate.before(), original);
    assert_eq!(activate.after(), fs_err::read(scripts.join("activate"))?);
    let repeated = finalize(&root, &scripts, &scheme, &destination, false)?;
    let repeated_activate = repeated
        .iter()
        .find(|update| update.path() == activate.path())
        .expect("already finalized activation update");
    assert_eq!(repeated_activate.before(), activate.before());
    assert_eq!(repeated_activate.after(), activate.after());

    for (name, expected) in render(&destination, &scheme, None, false)? {
        assert_eq!(
            fs_err::read(scripts.join(name))?,
            expected.as_bytes(),
            "{name}"
        );
    }
    assert_eq!(fs_err::read(&alias)?, original);
    assert_eq!(
        fs_err::metadata(scripts.join("activate"))?.permissions(),
        permissions
    );
    fs_err::rename(&root, &destination)?;
    #[cfg(unix)]
    {
        let output = Command::new("sh")
            .args([
                "-c",
                ". \"$1\"; printf '%s' \"$VIRTUAL_ENV\"",
                "activation-test",
            ])
            .arg(destination.join(&scheme.scripts).join("activate"))
            .output()?;
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, destination.as_os_str().as_encoded_bytes());
    }
    Ok(())
}

#[test]
fn modified_and_missing_activators_are_retained() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().join("staged");
    let scheme = scheme();
    let scripts = install(&root, &scheme)?;
    let custom_binary = b"custom\0\xff activation";
    fs_err::write(scripts.join("activate.fish"), custom_binary)?;
    let mut modified = fs_err::read(scripts.join("activate"))?;
    modified[0] = b'!';
    fs_err::write(scripts.join("activate"), &modified)?;
    fs_err::remove_file(scripts.join("activate.csh"))?;

    finalize(
        &root,
        &scripts,
        &scheme,
        &directory.path().join("final"),
        false,
    )?;

    assert_eq!(fs_err::read(scripts.join("activate"))?, modified);
    assert_eq!(fs_err::read(scripts.join("activate.fish"))?, custom_binary);
    assert!(!scripts.join("activate.csh").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn activation_symlinks_are_not_followed() -> Result<()> {
    let directory = TempDir::new()?;
    let root = directory.path().join("staged");
    let scheme = scheme();
    let scripts = install(&root, &scheme)?;
    let path = scripts.join("activate");
    let external = directory.path().join("external");
    fs_err::rename(&path, &external)?;
    let original = fs_err::read(&external)?;
    fs_err::os::unix::fs::symlink(&external, &path)?;

    finalize(
        &root,
        &scripts,
        &scheme,
        &directory.path().join("final"),
        false,
    )?;

    assert_eq!(fs_err::read_link(&path)?, external);
    assert_eq!(fs_err::read(external)?, original);
    Ok(())
}
