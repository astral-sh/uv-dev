//! Shared interpreter-query responses for platform-specific discovery tests.

use std::path::Path;

use anyhow::Result;
use indoc::indoc;

use crate::{ImplementationName, PythonVersion};

/// Return the fixed metadata emitted by a mock interpreter.
pub(crate) fn mock_interpreter_response(
    path: &Path,
    version: &PythonVersion,
    implementation: ImplementationName,
    system: bool,
    free_threaded: bool,
) -> Result<String> {
    let json = indoc! {r##"
        {
            "result": "success",
            "platform": {
                "os": {
                    "name": "manylinux",
                    "major": 2,
                    "minor": 38
                },
                "arch": "x86_64"
            },
            "manylinux_compatible": true,
            "standalone": true,
            "markers": {
                "implementation_name": "{IMPLEMENTATION}",
                "implementation_version": "{FULL_VERSION}",
                "os_name": "posix",
                "platform_machine": "x86_64",
                "platform_python_implementation": "{IMPLEMENTATION}",
                "platform_release": "6.5.0-13-generic",
                "platform_system": "Linux",
                "platform_version": "#13-Ubuntu SMP PREEMPT_DYNAMIC Fri Nov  3 12:16:05 UTC 2023",
                "python_full_version": "{FULL_VERSION}",
                "python_version": "{VERSION}",
                "sys_platform": "linux"
            },
            "sys_base_exec_prefix": "/home/ferris/.pyenv/versions/{FULL_VERSION}",
            "sys_base_prefix": "/home/ferris/.pyenv/versions/{FULL_VERSION}",
            "sys_prefix": "{PREFIX}",
            "sys_executable": "{PATH}",
            "sys_path": [
                "/home/ferris/.pyenv/versions/{FULL_VERSION}/lib/python{VERSION}/lib/python{VERSION}",
                "/home/ferris/.pyenv/versions/{FULL_VERSION}/lib/python{VERSION}/site-packages"
            ],
            "site_packages": [
                "/home/ferris/.pyenv/versions/{FULL_VERSION}/lib/python{VERSION}/site-packages"
            ],
            "stdlib": "/home/ferris/.pyenv/versions/{FULL_VERSION}/lib/python{VERSION}",
            "extension_suffixes": [".cpython-{VERSION}-x86_64-linux-gnu.so", ".abi3.so", ".so"],
            "scheme": {
                "data": "/home/ferris/.pyenv/versions/{FULL_VERSION}",
                "include": "/home/ferris/.pyenv/versions/{FULL_VERSION}/include",
                "platlib": "/home/ferris/.pyenv/versions/{FULL_VERSION}/lib/python{VERSION}/site-packages",
                "purelib": "/home/ferris/.pyenv/versions/{FULL_VERSION}/lib/python{VERSION}/site-packages",
                "scripts": "/home/ferris/.pyenv/versions/{FULL_VERSION}/bin"
            },
            "virtualenv": {
                "data": "",
                "include": "include",
                "platlib": "lib/python{VERSION}/site-packages",
                "purelib": "lib/python{VERSION}/site-packages",
                "scripts": "bin"
            },
            "pointer_size": "64",
            "gil_disabled": {FREE_THREADED},
            "debug_enabled": false
        }
    "##};

    let json = if system {
        json.replace("{PREFIX}", "/home/ferris/.pyenv/versions/{FULL_VERSION}")
    } else {
        json.replace("{PREFIX}", "/home/ferris/projects/uv/.venv")
    };

    let json = json
        .replace("\"{PATH}\"", &serde_json::to_string(path)?)
        .replace("{FULL_VERSION}", &version.to_string())
        .replace(
            "{VERSION}",
            &format!("{}.{}", version.major(), version.minor()),
        )
        .replace("{FREE_THREADED}", &free_threaded.to_string())
        .replace("{IMPLEMENTATION}", implementation.long_name());

    #[cfg(windows)]
    let json = {
        let mut response: serde_json::Value = serde_json::from_str(&json)?;
        response["platform"] = serde_json::json!({
            "os": { "name": "windows" },
            "arch": std::env::consts::ARCH,
        });
        response["manylinux_compatible"] = false.into();
        response["markers"]["os_name"] = "nt".into();
        response["markers"]["platform_machine"] = match std::env::consts::ARCH {
            "x86_64" => "AMD64",
            "aarch64" => "ARM64",
            architecture => architecture,
        }
        .into();
        response["markers"]["platform_system"] = "Windows".into();
        response["markers"]["sys_platform"] = "win32".into();
        serde_json::to_string(&response)?
    };

    Ok(json)
}

#[test]
fn mock_interpreter_response_escapes_executable_path() -> Result<()> {
    let path = Path::new(r"C:\Python with spaces\python.bat");
    let response = mock_interpreter_response(
        path,
        &"3.12.1".parse().expect("Test uses a valid Python version"),
        ImplementationName::CPython,
        true,
        false,
    )?;
    let response: serde_json::Value = serde_json::from_str(&response)?;
    assert_eq!(response["sys_executable"], serde_json::to_value(path)?);
    Ok(())
}
