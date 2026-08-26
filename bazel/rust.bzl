"""Cargo-backed Rust targets for the Bazel build experiment."""

load("@crates//:data.bzl", "DEP_DATA")
load("@crates//:defs.bzl", "RESOLVED_PLATFORMS", "all_crate_deps")
load("@rules_rust//cargo:defs.bzl", "cargo_build_script")
load("@rules_rust//rust:defs.bzl", "rust_binary", "rust_library", "rust_proc_macro", "rust_test")

def _crate_features(package):
    features = package["crate_features"]
    platform_features = package["crate_features_by_platform"]
    if platform_features:
        features = features + select(platform_features | {"//conditions:default": []})
    return features

def _aliases(package, kinds):
    # DEP_DATA also contains dev/build aliases. Restrict them to the selected
    # dependency kinds so they cannot add unrelated edges or dependency cycles.
    dependencies = []
    platforms = {}
    for kind in kinds:
        dependencies.extend(package[kind])
        for platform, deps in package[kind + "_by_platform"].items():
            platforms.setdefault(platform, []).extend(deps)

    aliases = package["aliases"]
    common = {dep: aliases[dep] for dep in dependencies if dep in aliases}
    if not platforms:
        return common
    return select({
        platform: common | {dep: aliases[dep] for dep in deps if dep in aliases}
        for platform, deps in platforms.items()
    } | {"//conditions:default": common})

def uv_rust_crate(
        name,
        version,
        edition,
        cargo_env,
        crate_name = None,
        crate_root = "src/lib.rs",
        proc_macro = False,
        build_script_data = [],
        compile_data = [],
        binary = None,
        unit_test = False):
    """Define a workspace library and the experiment's selected binary/tests.

    Dependency lists, aliases and feature cfgs come from rules_rs's resolution of
    the existing Cargo manifests and lockfile. That resolution unifies workspace
    features; it is not a separate per-target Cargo feature resolution.

    Args:
        name: Library target name, matching the Cargo package directory.
        version: Cargo package version, including inherited workspace values.
        edition: Cargo Rust edition.
        cargo_env: Cargo package metadata exposed to Rust code and build scripts.
        crate_name: Optional library crate-name override.
        crate_root: Library entry point, or None for a binary-only package.
        proc_macro: Whether the library is a procedural macro.
        build_script_data: Explicit non-source inputs for build.rs.
        compile_data: Explicit files read by include_str!/include_bytes!.
        binary: Optional binary entry point.
        unit_test: Whether to expose this library's unit tests.
    """
    package = DEP_DATA[native.package_name()]
    crate_features = _crate_features(package)
    normal_aliases = _aliases(package, ["deps"])
    build_script_deps = []

    native.exports_files(["Cargo.toml"])

    if native.glob(["build.rs"], allow_empty = True):
        build_script = name + "-build-script"
        cargo_build_script(
            name = build_script,
            pkg_name = cargo_env["CARGO_PKG_NAME"],
            srcs = ["build.rs"],
            crate_features = crate_features,
            edition = edition,
            version = version,
            deps = all_crate_deps(build = True),
            aliases = _aliases(package, ["build_deps"]),
            data = ["Cargo.toml"] + build_script_data,
            rustc_env = cargo_env,
            build_script_env = cargo_env,
        )
        build_script_deps = [":" + build_script]

    if crate_root:
        library = rust_proc_macro if proc_macro else rust_library
        library(
            name = name,
            crate_name = crate_name or name.replace("-", "_"),
            crate_root = crate_root,
            srcs = native.glob(["src/**/*.rs"], exclude = ["src/bin/**", "src/main.rs"]),
            crate_features = crate_features,
            edition = edition,
            version = version,
            deps = all_crate_deps() + build_script_deps,
            aliases = normal_aliases,
            compile_data = compile_data,
            rustc_env = cargo_env,
            target_compatible_with = RESOLVED_PLATFORMS,
            visibility = ["//visibility:public"],
        )

    if binary:
        # Keep //crates/uv:uv as the library for generated workspace dependency
        # labels; use a distinct target name but preserve the executable name.
        rust_binary(
            name = name + "-bin" if crate_root else name,
            binary_name = name,
            crate_name = name.replace("-", "_"),
            crate_root = binary,
            srcs = [binary],
            crate_features = crate_features,
            edition = edition,
            version = version,
            deps = ([":" + name] if crate_root else []) + all_crate_deps() + build_script_deps,
            aliases = normal_aliases,
            rustc_env = cargo_env,
            target_compatible_with = RESOLVED_PLATFORMS,
            visibility = ["//visibility:public"],
        )

    if unit_test and crate_root:
        rust_test(
            name = name + "-unit-tests",
            crate = ":" + name,
            crate_features = crate_features,
            edition = edition,
            version = version,
            deps = all_crate_deps(normal = True, normal_dev = True) + build_script_deps,
            aliases = _aliases(package, ["deps", "dev_deps"]),
            compile_data = compile_data,
            rustc_env = cargo_env,
            size = "small",
            target_compatible_with = RESOLVED_PLATFORMS,
            visibility = ["//visibility:public"],
        )
