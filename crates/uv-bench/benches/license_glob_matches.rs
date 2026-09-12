//! Benchmarks for tracking which PEP 639 license globs matched during a source-tree walk.

use std::hint::black_box;
use std::path::Path;

use criterion::{Criterion, criterion_group, criterion_main, measurement::WallTime};
use globset::{Glob, GlobMatcher};
use uv_globfilter::{GlobDirFilter, PortableGlobParser};

fn individual_matchers(globs: &[Glob]) -> Vec<GlobMatcher> {
    globs.iter().map(Glob::compile_matcher).collect()
}

fn repeated_scan(matchers: &[GlobMatcher], files: &[String]) -> Vec<bool> {
    let mut matched = vec![false; matchers.len()];
    for file in files {
        for (matched, matcher) in matched.iter_mut().zip(matchers) {
            if !*matched && matcher.is_match(file) {
                *matched = true;
            }
        }
    }
    matched
}

fn combined_matches(matcher: &GlobDirFilter, glob_count: usize, files: &[String]) -> Vec<bool> {
    let mut matched = vec![false; glob_count];
    let mut unmatched = glob_count;
    let mut matches = Vec::new();
    for file in files {
        if unmatched == 0 {
            break;
        }
        matcher.matching_globs_into(Path::new(file), &mut matches);
        for index in &matches {
            if !matched[*index] {
                matched[*index] = true;
                unmatched -= 1;
            }
        }
    }
    matched
}

fn license_glob_matches(criterion: &mut Criterion<WallTime>) {
    let mut group = criterion.benchmark_group("license glob matches");
    for count in [64, 256, 512, 1024, 4096] {
        let files = (0..count)
            .map(|index| format!("licenses/LICENSE-{index:04}"))
            .collect::<Vec<_>>();

        // Each file matches one pattern, with the patterns in reverse walk order. This retains an
        // unmatched pattern until the final file and exercises the quadratic scan faithfully.
        let globs = (0..count)
            .rev()
            .map(|index| {
                PortableGlobParser::Pep639
                    .parse(&format!("licenses/LICENSE-{index:04}"))
                    .expect("benchmark glob should be valid")
            })
            .collect::<Vec<_>>();
        if count <= 512 {
            let individual = individual_matchers(&globs);
            group.bench_function(format!("unique/repeated/{count}"), |benchmark| {
                benchmark.iter(|| repeated_scan(black_box(&individual), black_box(&files)));
            });
        }
        let combined = GlobDirFilter::from_globs(globs).expect("benchmark glob set should build");
        group.bench_function(format!("unique/combined/{count}"), |benchmark| {
            benchmark.iter(|| combined_matches(black_box(&combined), count, black_box(&files)));
        });

        // One broad glob includes every file, while every other pattern remains unmatched. This
        // exercises the error path where matching cannot stop early and preserves the identity of
        // the first unmatched pattern.
        let mut globs = (0..count)
            .map(|index| {
                PortableGlobParser::Pep639
                    .parse(&format!("licenses/missing-{index:04}"))
                    .expect("benchmark glob should be valid")
            })
            .collect::<Vec<_>>();
        globs.push(
            PortableGlobParser::Pep639
                .parse("licenses/LICENSE-*")
                .expect("benchmark glob should be valid"),
        );
        let glob_count = globs.len();
        if count <= 512 {
            let individual = individual_matchers(&globs);
            group.bench_function(format!("unmatched/repeated/{count}"), |benchmark| {
                benchmark.iter(|| repeated_scan(black_box(&individual), black_box(&files)));
            });
        }
        let combined = GlobDirFilter::from_globs(globs).expect("benchmark glob set should build");
        group.bench_function(format!("unmatched/combined/{count}"), |benchmark| {
            benchmark
                .iter(|| combined_matches(black_box(&combined), glob_count, black_box(&files)));
        });

        // All patterns overlap on every file. The combined path can stop tracking once the first
        // file has matched them all; the repeated scan still visits every flag for every file.
        let globs = (0..count)
            .map(|_| {
                PortableGlobParser::Pep639
                    .parse("licenses/LICENSE-*")
                    .expect("benchmark glob should be valid")
            })
            .collect::<Vec<_>>();
        if count <= 512 {
            let individual = individual_matchers(&globs);
            group.bench_function(format!("overlap/repeated/{count}"), |benchmark| {
                benchmark.iter(|| repeated_scan(black_box(&individual), black_box(&files)));
            });
        }
        let combined = GlobDirFilter::from_globs(globs).expect("benchmark glob set should build");
        group.bench_function(format!("overlap/combined/{count}"), |benchmark| {
            benchmark.iter(|| combined_matches(black_box(&combined), count, black_box(&files)));
        });
    }
    group.finish();
}

criterion_group!(license_glob_matches_group, license_glob_matches);
criterion_main!(license_glob_matches_group);
