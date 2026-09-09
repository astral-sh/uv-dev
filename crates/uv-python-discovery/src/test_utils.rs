//! Shared interpreter-query responses for platform-specific discovery tests.

use std::path::Path;

use indoc::indoc;

use uv_python_types::{ImplementationName, PythonVersion};

/// Return the fixed metadata emitted by a mock interpreter.
pub(crate) fn mock_interpreter_response(
    path: &Path,
    version: &PythonVersion,
    implementation: ImplementationName,
    system: bool,
    free_threaded: bool,
) -> String {
    #[cfg(not(windows))]
    let (platform_os, platform_arch, manylinux_compatible) = (
        r#"{"name":"manylinux","major":2,"minor":38}"#,
        "x86_64",
        true,
    );
    #[cfg(windows)]
    let (platform_os, platform_arch, manylinux_compatible) =
        (r#"{"name":"windows"}"#, std::env::consts::ARCH, false);
    #[cfg(not(windows))]
    let (os_name, platform_machine, platform_system, sys_platform) =
        ("posix", "x86_64", "Linux", "linux");
    #[cfg(windows)]
    let (os_name, platform_machine, platform_system, sys_platform) = (
        "nt",
        match std::env::consts::ARCH {
            "x86_64" => "AMD64",
            "aarch64" => "ARM64",
            architecture => architecture,
        },
        "Windows",
        "win32",
    );

    let json = indoc! {r##"
        {
            "result": "success",
            "platform": {
                "os": {PLATFORM_OS},
                "arch": "{PLATFORM_ARCH}"
            },
            "manylinux_compatible": {MANYLINUX_COMPATIBLE},
            "standalone": true,
            "markers": {
                "implementation_name": "{IMPLEMENTATION}",
                "implementation_version": "{FULL_VERSION}",
                "os_name": "{OS_NAME}",
                "platform_machine": "{PLATFORM_MACHINE}",
                "platform_python_implementation": "{IMPLEMENTATION}",
                "platform_release": "6.5.0-13-generic",
                "platform_system": "{PLATFORM_SYSTEM}",
                "platform_version": "#13-Ubuntu SMP PREEMPT_DYNAMIC Fri Nov  3 12:16:05 UTC 2023",
                "python_full_version": "{FULL_VERSION}",
                "python_version": "{VERSION}",
                "sys_platform": "{SYS_PLATFORM}"
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

    json.replace("\"{PATH}\"", &format!("{path:?}"))
        .replace("{FULL_VERSION}", &version.to_string())
        .replace(
            "{VERSION}",
            &format!("{}.{}", version.major(), version.minor()),
        )
        .replace("{PLATFORM_OS}", platform_os)
        .replace("{PLATFORM_ARCH}", platform_arch)
        .replace("{MANYLINUX_COMPATIBLE}", &manylinux_compatible.to_string())
        .replace("{OS_NAME}", os_name)
        .replace("{PLATFORM_MACHINE}", platform_machine)
        .replace("{PLATFORM_SYSTEM}", platform_system)
        .replace("{SYS_PLATFORM}", sys_platform)
        .replace("{FREE_THREADED}", &free_threaded.to_string())
        .replace("{IMPLEMENTATION}", implementation.long_name())
}

#[test]
fn mock_interpreter_response_escapes_executable_path() {
    let path = Path::new(r"C:\Python with spaces\python.bat");
    let response = mock_interpreter_response(
        path,
        &"3.12.1".parse().expect("Test uses a valid Python version"),
        ImplementationName::CPython,
        true,
        false,
    );
    assert_eq!(
        response
            .lines()
            .find(|line| line.trim_start().starts_with("\"sys_executable\"")),
        Some(r#"    "sys_executable": "C:\\Python with spaces\\python.bat","#),
    );
}
