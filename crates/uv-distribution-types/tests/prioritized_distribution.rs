use std::error::Error;
use std::sync::Arc;

use uv_distribution_filename::{SourceDistExtension, WheelFilename};
use uv_distribution_types::HashComparison::{Matched, Mismatched, Missing};
use uv_distribution_types::{
    File, HashComparison, IncompatibleSource, IncompatibleWheel, IndexUrl, PrioritizedDist,
    PythonRequirementKind, RegistryBuiltWheel, RegistrySourceDist, ResolvedDistRef,
    SourceDistCompatibility, WheelCompatibility,
};
use uv_platform_tags::{IncompatibleTag, TagPriority};
use uv_pypi_types::{CoreMetadata, Hashes, PypiFile};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn file(filename: &str) -> TestResult<File> {
    Ok(File::try_from_pypi(
        PypiFile {
            core_metadata: Some(CoreMetadata::Bool(true)),
            filename: filename.into(),
            hashes: Hashes::default(),
            requires_python: None,
            size: None,
            upload_time: None,
            url: filename.into(),
            yanked: None,
        },
        &"https://example.invalid/".into(),
    )?)
}

fn wheel(filename: &str) -> TestResult<RegistryBuiltWheel> {
    Ok(RegistryBuiltWheel {
        filename: filename.parse()?,
        file: Box::new(file(filename)?),
        index: IndexUrl::parse("https://example.invalid/simple", None)?,
        size_is_authoritative: false,
    })
}

fn source(filename: &str) -> TestResult<RegistrySourceDist> {
    Ok(RegistrySourceDist {
        name: "example".parse()?,
        version: "1.0".parse()?,
        file: Box::new(file(filename)?),
        ext: SourceDistExtension::from_path(filename)?,
        index: IndexUrl::parse("https://example.invalid/simple", None)?,
        wheels: vec![],
        size_is_authoritative: false,
    })
}

fn compatible_wheel(
    filename: &str,
    hash: HashComparison,
    priority: Option<usize>,
) -> TestResult<WheelCompatibility> {
    Ok(WheelCompatibility::Compatible(
        hash,
        priority.map(TagPriority::try_from).transpose()?,
        filename.parse::<WheelFilename>()?.build_tag().cloned(),
    ))
}

#[derive(Debug, PartialEq, Eq)]
enum Selection<'a> {
    Installed,
    Wheel(&'a str),
    Source(&'a str),
}

impl<'a> From<ResolvedDistRef<'a>> for Selection<'a> {
    fn from(dist: ResolvedDistRef<'a>) -> Self {
        match dist {
            ResolvedDistRef::Installed { .. } => Self::Installed,
            ResolvedDistRef::InstallableRegistrySourceDist { sdist, .. } => {
                Self::Source(&sdist.file.filename)
            }
            ResolvedDistRef::InstallableRegistryBuiltDist { wheel, .. } => {
                Self::Wheel(&wheel.file.filename)
            }
        }
    }
}

fn selection(prioritized: &PrioritizedDist) -> Option<(Selection<'_>, Selection<'_>)> {
    prioritized
        .get()
        .map(|dist| (dist.for_resolution().into(), dist.for_installation().into()))
}

#[test]
fn source_and_wheel_hash_precedence() -> TestResult {
    let wheel_filename = "example-1.0-py3-none-any.whl";
    let source_filename = "example-1.0.tar.gz";
    for (wheel_hash, source_hash, prefer_source) in [
        (Mismatched, Mismatched, false),
        (Mismatched, Missing, true),
        (Mismatched, Matched, true),
        (Missing, Mismatched, false),
        (Missing, Missing, false),
        (Missing, Matched, true),
        (Matched, Mismatched, false),
        (Matched, Missing, false),
        (Matched, Matched, false),
    ] {
        let wheel_compatibility = compatible_wheel(wheel_filename, wheel_hash, Some(1))?;
        let source_compatibility = SourceDistCompatibility::Compatible(source_hash);

        let mut wheel_first = PrioritizedDist::from_built(
            wheel(wheel_filename)?,
            vec![],
            wheel_compatibility.clone(),
        );
        wheel_first.insert_source(source(source_filename)?, [], source_compatibility.clone());

        let mut source_first =
            PrioritizedDist::from_source(source(source_filename)?, vec![], source_compatibility);
        source_first.insert_built(wheel(wheel_filename)?, [], wheel_compatibility);

        for prioritized in [wheel_first, source_first] {
            let expected = if prefer_source {
                (
                    Selection::Source(source_filename),
                    Selection::Source(source_filename),
                )
            } else {
                (
                    Selection::Wheel(wheel_filename),
                    Selection::Wheel(wheel_filename),
                )
            };
            assert_eq!(
                selection(&prioritized),
                Some(expected),
                "wheel hash: {wheel_hash:?}; source hash: {source_hash:?}",
            );
        }
    }
    Ok(())
}

#[test]
fn compatible_wheels_rank_hash_then_tag_then_build() -> TestResult {
    // Each entry must outrank the previous entry, even if a lower-priority field decreases.
    let candidates = [
        ("example-1.0-9-py2-none-any.whl", Mismatched, Some(9)),
        ("example-1.0-py2-none-any.whl", Missing, None),
        ("example-1.0-9-py3-none-any.whl", Missing, Some(1)),
        ("example-1.0-cp311-none-any.whl", Missing, Some(2)),
        ("example-1.0-1-py3-none-any.whl", Missing, Some(2)),
        ("example-1.0-2-py3-none-any.whl", Missing, Some(2)),
        ("example-1.0-cp312-none-any.whl", Matched, None),
    ];
    for (lower_index, &(lower_filename, lower_hash, lower_priority)) in
        candidates.iter().enumerate()
    {
        for &(higher_filename, higher_hash, higher_priority) in &candidates[lower_index + 1..] {
            let lower = compatible_wheel(lower_filename, lower_hash, lower_priority)?;
            let higher = compatible_wheel(higher_filename, higher_hash, higher_priority)?;
            for (first_filename, first, second_filename, second) in [
                (
                    lower_filename,
                    lower.clone(),
                    higher_filename,
                    higher.clone(),
                ),
                (higher_filename, higher, lower_filename, lower),
            ] {
                let mut prioritized =
                    PrioritizedDist::from_built(wheel(first_filename)?, vec![], first);
                prioritized.insert_built(wheel(second_filename)?, [], second);
                assert_eq!(
                    selection(&prioritized),
                    Some((
                        Selection::Wheel(higher_filename),
                        Selection::Wheel(higher_filename),
                    )),
                );
            }
        }
    }
    Ok(())
}

#[test]
fn compatible_wheel_ties_retain_the_first_file() -> TestResult {
    let filenames = [
        "example-1.0-py2-none-any.whl",
        "example-1.0-py3-none-any.whl",
    ];
    for (first, second) in [(filenames[0], filenames[1]), (filenames[1], filenames[0])] {
        let mut prioritized = PrioritizedDist::from_built(
            wheel(first)?,
            vec![],
            compatible_wheel(first, HashComparison::Matched, Some(1))?,
        );
        prioritized.insert_built(
            wheel(second)?,
            [],
            compatible_wheel(second, HashComparison::Matched, Some(1))?,
        );
        assert_eq!(
            selection(&prioritized),
            Some((Selection::Wheel(first), Selection::Wheel(first))),
        );
    }
    Ok(())
}

#[test]
fn compatible_sources_rank_hashes_and_retain_ties() -> TestResult {
    let candidates = [
        ("example-1.0.tar.bz2", Mismatched),
        ("example-1.0.zip", Missing),
        ("example-1.0.tar.gz", Matched),
    ];
    for (lower_index, &(lower_filename, lower_hash)) in candidates.iter().enumerate() {
        for &(higher_filename, higher_hash) in &candidates[lower_index + 1..] {
            for (first_filename, first_hash, second_filename, second_hash) in [
                (lower_filename, lower_hash, higher_filename, higher_hash),
                (higher_filename, higher_hash, lower_filename, lower_hash),
            ] {
                let mut prioritized = PrioritizedDist::from_source(
                    source(first_filename)?,
                    vec![],
                    SourceDistCompatibility::Compatible(first_hash),
                );
                prioritized.insert_source(
                    source(second_filename)?,
                    [],
                    SourceDistCompatibility::Compatible(second_hash),
                );
                assert_eq!(
                    selection(&prioritized),
                    Some((
                        Selection::Source(higher_filename),
                        Selection::Source(higher_filename),
                    )),
                );
            }
        }
    }

    for (first, second) in [
        ("example-1.0.tar.gz", "example-1.0.zip"),
        ("example-1.0.zip", "example-1.0.tar.gz"),
    ] {
        let mut prioritized = PrioritizedDist::from_source(
            source(first)?,
            vec![],
            SourceDistCompatibility::Compatible(Matched),
        );
        prioritized.insert_source(
            source(second)?,
            [],
            SourceDistCompatibility::Compatible(Matched),
        );
        assert_eq!(
            selection(&prioritized),
            Some((Selection::Source(first), Selection::Source(first))),
        );
    }
    Ok(())
}

#[test]
fn compatible_distributions_outrank_incompatible_distributions() -> TestResult {
    let wheel_filename = "example-1.0-py3-none-any.whl";
    let source_filename = "example-1.0.tar.gz";
    let mut prioritized = PrioritizedDist::default();
    assert!(prioritized.is_empty());
    assert_eq!(selection(&prioritized), None);

    prioritized.insert_built(
        wheel(wheel_filename)?,
        [],
        WheelCompatibility::Incompatible(IncompatibleWheel::NoBinary),
    );
    prioritized.insert_source(
        source(source_filename)?,
        [],
        SourceDistCompatibility::Incompatible(IncompatibleSource::NoBuild),
    );
    assert!(!prioritized.is_empty());
    assert_eq!(selection(&prioritized), None);
    assert_eq!(
        prioritized.incompatible_wheel(),
        Some(&IncompatibleWheel::NoBinary),
    );
    assert_eq!(
        prioritized.incompatible_source(),
        Some(&IncompatibleSource::NoBuild),
    );

    prioritized.insert_built(
        wheel(wheel_filename)?,
        [],
        compatible_wheel(wheel_filename, HashComparison::Mismatched, None)?,
    );
    prioritized.insert_built(
        wheel("example-1.0-py2-none-any.whl")?,
        [],
        WheelCompatibility::Incompatible(IncompatibleWheel::NoBinary),
    );
    assert_eq!(prioritized.incompatible_wheel(), None);
    assert_eq!(
        selection(&prioritized),
        Some((
            Selection::Wheel(wheel_filename),
            Selection::Wheel(wheel_filename),
        )),
    );
    Ok(())
}

#[test]
fn incompatible_wheel_metadata_can_resolve_a_source_install() -> TestResult {
    let wheel_filename = "example-1.0-cp312-cp312-win_amd64.whl";
    let source_filename = "example-1.0.tar.gz";
    let mut wheel = wheel(wheel_filename)?;
    wheel.file.requires_python = Some(Arc::new(">=3.9".parse()?));
    let mut source = source(source_filename)?;
    source.file.requires_python = Some(Arc::new(">=3.12".parse()?));
    let mut prioritized = PrioritizedDist::from_built(
        wheel,
        vec![],
        WheelCompatibility::Incompatible(IncompatibleWheel::Tag(IncompatibleTag::Platform)),
    );
    prioritized.insert_source(
        source,
        [],
        SourceDistCompatibility::Compatible(HashComparison::Matched),
    );
    assert_eq!(
        selection(&prioritized),
        Some((
            Selection::Wheel(wheel_filename),
            Selection::Source(source_filename),
        )),
    );
    assert_eq!(
        prioritized
            .get()
            .and_then(|dist| dist.requires_python().map(ToString::to_string)),
        Some(">=3.12".to_string()),
    );
    Ok(())
}

#[test]
fn incompatible_wheel_ranking_preserves_the_closest_match() -> TestResult {
    let candidates = [
        IncompatibleWheel::NoBinary,
        IncompatibleWheel::RequiresPython(">=3.12".parse()?, PythonRequirementKind::Target),
        IncompatibleWheel::Tag(IncompatibleTag::Python),
        IncompatibleWheel::Tag(IncompatibleTag::Platform),
    ];
    for (lower_index, lower) in candidates.iter().enumerate() {
        for higher in &candidates[lower_index + 1..] {
            for (first, second) in [(lower, higher), (higher, lower)] {
                let mut prioritized = PrioritizedDist::from_built(
                    wheel("example-1.0-py2-none-any.whl")?,
                    vec![],
                    WheelCompatibility::Incompatible(first.clone()),
                );
                prioritized.insert_built(
                    wheel("example-1.0-py3-none-any.whl")?,
                    [],
                    WheelCompatibility::Incompatible(second.clone()),
                );
                assert_eq!(prioritized.incompatible_wheel(), Some(higher));
                assert_eq!(selection(&prioritized), None);
            }
        }
    }
    Ok(())
}
