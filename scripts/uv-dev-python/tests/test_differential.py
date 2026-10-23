"""Compare shared packaging semantics through the Python bindings."""

import string

from hypothesis import given, settings
from hypothesis import strategies as st
from packaging.markers import Marker as PackagingMarker
from packaging.requirements import Requirement as PackagingRequirement
from packaging.specifiers import SpecifierSet
from packaging.utils import canonicalize_name
from packaging.version import Version as PackagingVersion
from uv_dev import (
    Marker,
    Requirement,
    Version,
    VersionSpecifiers,
    normalize_package_name,
)


@st.composite
def versions(draw: st.DrawFn) -> str:
    release = draw(st.lists(st.integers(0, 200), min_size=1, max_size=4))
    value = ".".join(map(str, release))
    epoch = draw(st.integers(0, 3))
    if epoch:
        value = f"{epoch}!{value}"
    pre = draw(st.sampled_from(["", "a", "b", "rc"]))
    if pre:
        value += f"{pre}{draw(st.integers(0, 20))}"
    post = draw(st.one_of(st.none(), st.integers(0, 20)))
    if post is not None:
        value += f".post{post}"
    dev = draw(st.one_of(st.none(), st.integers(0, 20)))
    if dev is not None:
        value += f".dev{dev}"
    local = draw(st.sampled_from(["", "+abc", "+ABC.1", "+1.2"]))
    return value + local


names = st.lists(
    st.text(string.ascii_letters + string.digits, min_size=1, max_size=12),
    min_size=1,
    max_size=4,
).flatmap(
    lambda parts: st.sampled_from(["-", "_", ".", "__", "._"]).map(
        lambda sep: sep.join(parts)
    )
)


@settings(max_examples=200, derandomize=True)
@given(left=versions(), right=versions())
def test_versions_match_packaging(left: str, right: str) -> None:
    uv_left, uv_right = Version(left), Version(right)
    packaging_left, packaging_right = PackagingVersion(left), PackagingVersion(right)
    assert str(uv_left) == str(packaging_left)
    assert uv_left.release == packaging_left.release
    assert uv_left.is_prerelease == packaging_left.is_prerelease
    assert (uv_left < uv_right) == (packaging_left < packaging_right)
    assert (uv_left == uv_right) == (packaging_left == packaging_right)


@settings(max_examples=200, derandomize=True)
@given(name=names, extra=names, major=st.integers(0, 20), minor=st.integers(0, 20))
def test_requirement_fields_match_packaging(
    name: str, extra: str, major: int, minor: int
) -> None:
    value = f"{name}[{extra}]>={major}.{minor}; python_version >= '3.12'"
    uv_requirement = Requirement(value)
    packaging_requirement = PackagingRequirement(value)
    assert uv_requirement.name == canonicalize_name(packaging_requirement.name)
    assert set(uv_requirement.extras) == {
        canonicalize_name(item) for item in packaging_requirement.extras
    }
    assert str(uv_requirement.specifiers) == str(packaging_requirement.specifier)
    assert normalize_package_name(name) == canonicalize_name(name, validate=True)


@settings(max_examples=200, derandomize=True)
@given(major=st.integers(0, 20), minor=st.integers(0, 20), candidate=versions())
def test_specifier_comparisons_match_packaging(
    major: int, minor: int, candidate: str
) -> None:
    value = f">={major}.{minor},!={major}.{minor + 1},<{major + 1}"
    # Resolver prerelease admission is separate from the Rust comparison API.
    assert (Version(candidate) in VersionSpecifiers(value)) == SpecifierSet(
        value
    ).contains(candidate, prereleases=True)


@settings(max_examples=200, derandomize=True)
@given(
    minor=st.integers(6, 16),
    patch=st.integers(0, 20),
    cutoff=st.integers(6, 16),
    platform=st.sampled_from(["linux", "win32", "darwin"]),
    extra=st.sampled_from(["", "test", "fast-mode"]),
)
def test_marker_evaluation_matches_packaging(
    environment: dict[str, str],
    minor: int,
    patch: int,
    cutoff: int,
    platform: str,
    extra: str,
) -> None:
    environment = environment.copy()
    environment.update(
        python_version=f"3.{minor}",
        python_full_version=f"3.{minor}.{patch}",
        implementation_version=f"3.{minor}.{patch}",
        sys_platform=platform,
    )
    expression = f"python_version >= '3.{cutoff}' and (sys_platform == 'linux' or extra == 'fast-mode')"
    assert Marker(expression).evaluate(
        environment, extras=[extra] if extra else []
    ) == (PackagingMarker(expression).evaluate({**environment, "extra": extra}))
