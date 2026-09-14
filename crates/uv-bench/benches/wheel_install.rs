//! Installation and overlapping replacement of real wheel contents.

mod common;

extern crate uv_performance_memory_allocator;

use std::hint::black_box;
use std::path::Path;
use std::str::FromStr;

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use uv_bench::{WHEEL_FIXTURES, fixture_path, is_codspeed_simulation};
use uv_distribution_filename::WheelFilename;
use uv_install_wheel::{InstallState, Layout, LinkMode};
use uv_preview::Preview;
use uv_pypi_types::Scheme;

fn layout(root: &Path) -> Layout {
    let site_packages = root.join("site-packages");
    fs_err::create_dir_all(&site_packages).expect("Failed to create site-packages");
    Layout {
        sys_executable: root.join("bin/python"),
        python_version: (3, 11),
        os_name: "posix".to_string(),
        scheme: Scheme {
            purelib: site_packages.clone(),
            platlib: site_packages,
            scripts: root.join("bin"),
            data: root.to_path_buf(),
            include: root.join("include"),
        },
    }
}

fn install(layout: &Layout, wheel: &Path, filename: &WheelFilename, link_mode: LinkMode) {
    let state = InstallState::new(Preview::default());
    uv_install_wheel::install_wheel(
        layout,
        false,
        wheel,
        filename,
        None,
        None::<&()>,
        None::<&()>,
        Some("uv"),
        true,
        link_mode,
        &state,
    )
    .expect("Failed to install wheel");
    state
        .warn_package_conflicts()
        .expect("Failed to check package conflicts");
}

fn wheel_install(c: &mut Criterion<WallTime>) {
    if is_codspeed_simulation() {
        return;
    }
    let mut group = c.benchmark_group("wheel_install");
    for &(project, filename) in WHEEL_FIXTURES {
        let archive = fixture_path(filename);
        let filename = WheelFilename::from_str(filename).expect("Invalid wheel filename");
        let wheel = tempfile::tempdir().expect("Failed to create wheel directory");
        let files = uv_extract::unzip(
            fs_err::File::open(archive).expect("Failed to open wheel fixture"),
            wheel.path(),
        )
        .expect("Failed to extract wheel fixture");
        uv_install_wheel::validate_and_heal_record(
            wheel.path(),
            files.iter().map(|file| (file.path(), file.size())),
            &filename,
        )
        .expect("Failed to validate wheel fixture");

        for (mode_name, mode) in [("hardlink", LinkMode::Hardlink), ("copy", LinkMode::Copy)] {
            for overlap in [false, true] {
                let operation = if overlap { "replace" } else { "fresh" };
                group.bench_function(
                    BenchmarkId::new(format!("{operation}_{mode_name}"), project),
                    |b| {
                        b.iter_batched(
                            || {
                                let environment = tempfile::tempdir()
                                    .expect("Failed to create installation directory");
                                let layout = layout(environment.path());
                                if overlap {
                                    // Distinct old inodes force the atomic replacement path.
                                    install(&layout, wheel.path(), &filename, LinkMode::Copy);
                                }
                                (environment, layout)
                            },
                            |(environment, layout)| {
                                install(&layout, wheel.path(), &filename, mode);
                                black_box((environment, layout))
                            },
                            BatchSize::PerIteration,
                        );
                    },
                );
            }
        }
    }
    group.finish();
}

criterion_group! {
    name = wheels;
    config = common::walltime_criterion();
    targets = wheel_install
}
criterion_main!(wheels);
