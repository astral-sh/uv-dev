import pytest


@pytest.fixture(scope="module")
def environment() -> dict[str, str]:
    """A target environment independent of the host running the tests."""
    return {
        "implementation_name": "cpython",
        "implementation_version": "3.14.0",
        "os_name": "posix",
        "platform_machine": "x86_64",
        "platform_python_implementation": "CPython",
        "platform_release": "6.8.0",
        "platform_system": "Linux",
        "platform_version": "test",
        "python_full_version": "3.14.0",
        "python_version": "3.14",
        "sys_platform": "linux",
    }
