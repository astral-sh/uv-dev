//! Requirements files with a UTF-16 byte-order mark. See astral-sh/uv#2276.

use std::collections::BTreeMap;

use anyhow::Result;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use byteorder::{BigEndian, ByteOrder, LittleEndian};

use uv_test::packse::generate_wheel;
use uv_test::{TestContext, uv_snapshot};

const REQUIREMENTS: &str = "# café 🐍\ntomli<=2.0.1\n";

fn tomli_wheels(context: &TestContext) -> Result<ChildPath> {
    let wheels = context.temp_dir.child("wheels");
    wheels.create_dir_all()?;
    let name = "tomli".parse()?;
    for version in ["2.0.1", "2.2.1"] {
        let (filename, contents) = generate_wheel(
            &name,
            &version.parse()?,
            &[],
            &BTreeMap::new(),
            None,
            "py3-none-any",
            &[],
        );
        wheels.child(filename).write_binary(&contents)?;
    }
    Ok(wheels)
}

fn encode_utf16_with_bom<T: ByteOrder>(input: &str) -> Vec<u8> {
    let code_units: Vec<_> = std::iter::once(0xfeff)
        .chain(input.encode_utf16())
        .collect();
    let mut bytes = vec![0; code_units.len() * 2];
    T::write_u16_into(&code_units, &mut bytes);
    bytes
}

#[test]
fn install_utf16le_requirements() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = tomli_wheels(&context)?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_binary(&encode_utf16_with_bom::<LittleEndian>(REQUIREMENTS))?;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["-r", "requirements.txt", "--no-index", "--find-links"])
        .arg(wheels.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1
    ");
    context.assert_installed("tomli", "2.0.1");
    Ok(())
}

#[test]
fn install_utf16be_requirements() -> Result<()> {
    let context = uv_test::test_context!("3.12");
    let wheels = tomli_wheels(&context)?;
    context
        .temp_dir
        .child("requirements.txt")
        .write_binary(&encode_utf16_with_bom::<BigEndian>(REQUIREMENTS))?;

    uv_snapshot!(context.filters(), context.pip_install()
        .args(["-r", "requirements.txt", "--no-index", "--find-links"])
        .arg(wheels.path()), @"
    exit_code: 0 (success)
    ----- stderr -----
    Resolved 1 package in [TIME]
    Prepared 1 package in [TIME]
    Installed 1 package in [TIME]
     + tomli==2.0.1
    ");
    context.assert_installed("tomli", "2.0.1");
    Ok(())
}
