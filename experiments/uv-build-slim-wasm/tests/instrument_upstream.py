"""Install differential hooks into a disposable, pinned upstream checkout."""

import argparse
import shutil
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("checkout", type=Path)
    args = parser.parse_args()
    root = args.checkout / "crates/uv-build-backend/src"
    if "mod parity_probe;" in (root / "lib.rs").read_text():
        parser.error(
            "this checkout is already instrumented; use a fresh pinned archive"
        )
    shutil.copyfile(
        Path(__file__).with_name("parity_probe.rs"), root / "parity_probe.rs"
    )
    library = root / "lib.rs"
    library.write_text("#[cfg(test)]\nmod parity_probe;\n" + library.read_text())
    wheel = root / "wheel.rs"
    source = wheel.read_text()
    for name in ("build_wheel", "build_editable", "metadata"):
        source = source.replace("pub fn " + name + "(", "fn native_" + name + "(", 1)
    for name, operation in [
        ("build_wheel", "build-wheel"),
        ("build_editable", "build-editable"),
    ]:
        source += """
pub fn NAME(source_tree: &Path, wheel_dir: &Path, metadata_directory: Option<&Path>, uv_version: &str, show_warnings: bool) -> Result<WheelFilename,Error> {
    #[cfg(test)]
    let probe=crate::parity_probe::prepare(wheel_dir);
    let result=native_NAME(source_tree,wheel_dir,metadata_directory,uv_version,show_warnings);
    #[cfg(test)]
    if let Some(probe)=probe { probe.check("OPERATION",source_tree,metadata_directory,uv_version,false,&result); }
    result
}
""".replace("NAME", name).replace("OPERATION", operation)
    source += """
pub fn metadata(source_tree: &Path, metadata_directory: &Path, uv_version: &str) -> Result<String,Error> {
    #[cfg(test)]
    let probe=crate::parity_probe::prepare(metadata_directory);
    let result=native_metadata(source_tree,metadata_directory,uv_version);
    #[cfg(test)]
    if let Some(probe)=probe { probe.check("prepare-metadata-for-build-wheel",source_tree,None,uv_version,false,&result); }
    result
}
"""
    wheel.write_text(source)
    sdist = root / "source_dist.rs"
    source = sdist.read_text().replace(
        "pub fn build_source_dist(", "fn native_build_source_dist(", 1
    )
    source += """
pub fn build_source_dist(source_tree: &Path, source_dist_directory: &Path, uv_version: &str, show_warnings: bool, tar_backend: TarBackend) -> Result<SourceDistFilename,Error> {
    #[cfg(test)]
    let probe=crate::parity_probe::prepare(source_dist_directory);
    let result=native_build_source_dist(source_tree,source_dist_directory,uv_version,show_warnings,tar_backend);
    #[cfg(test)]
    if let Some(probe)=probe { probe.check("build-sdist",source_tree,None,uv_version,matches!(tar_backend,TarBackend::TokioTar),&result); }
    result
}
"""
    sdist.write_text(source)


if __name__ == "__main__":
    main()
