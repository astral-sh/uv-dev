use uv_python::PyVenvConfiguration;

fn check_duplicate_system_site_packages(value: &str, expected: bool) -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("pyvenv.cfg");
    let content = format!(
        "include-system-site-packages = {value}\nother = keep\ninclude-system-site-packages = {}\n",
        !expected
    );
    let updated = PyVenvConfiguration::set(&content, "include-system-site-packages", value);
    fs_err::write(&path, &updated)?;
    assert_eq!(
        PyVenvConfiguration::parse(&path)?.include_system_site_packages(),
        expected
    );
    assert_eq!(
        updated,
        format!(
            "include-system-site-packages = {value}\nother = keep\ninclude-system-site-packages = {value}\n"
        )
    );
    Ok(())
}

#[test]
fn set_duplicate_system_site_packages_enabled() -> anyhow::Result<()> {
    check_duplicate_system_site_packages("true", true)
}

#[test]
fn set_duplicate_system_site_packages_disabled() -> anyhow::Result<()> {
    check_duplicate_system_site_packages("false", false)
}

#[test]
fn set_all_exact_matching_keys() {
    for key in ["home", "extends-environment"] {
        let content =
            format!(" {key}\t=first\n{key}=second=part\n{key}-suffix = keep\n{key}=third\n");
        assert_eq!(
            PyVenvConfiguration::set(&content, key, "new=value"),
            format!(
                "{key} = new=value\n{key} = new=value\n{key}-suffix = keep\n{key} = new=value\n"
            )
        );
    }

    assert_eq!(
        PyVenvConfiguration::set(
            "home = first\nHome = preserve\nhome = second\n",
            "home",
            "new"
        ),
        "home = new\nHome = preserve\nhome = new\n"
    );
}

#[test]
fn set_preserves_existing_line_normalization() {
    for (content, key, value, expected) in [
        ("", "home", "new", "home = new\n"),
        ("other = keep", "home", "new", "other = keep\nhome = new\n"),
        ("home=old", "home", "new", "home = new\n"),
        (
            "home = first\r\nother = keep\r\nhome=second\r\n",
            "home",
            "new",
            "home = new\nother = keep\nhome = new\n",
        ),
        (
            "other = keep\n\n",
            "home",
            "new",
            "other = keep\n\nhome = new\n",
        ),
    ] {
        assert_eq!(PyVenvConfiguration::set(content, key, value), expected);
    }
}
