use uv_distribution_types::{
    IncompatibleDist, IncompatibleSource, IncompatibleWheel, PythonRequirementKind,
};
use uv_pep440::VersionSpecifiers;
use uv_pep508::MarkerTree;
use uv_platform_tags::IncompatibleTag;
use uv_pypi_types::Yanked;

#[test]
fn singular_and_plural_incompatibility_messages() {
    let requires_python: VersionSpecifiers = ">=3.12".parse().unwrap();
    let cases = [
        (
            IncompatibleDist::Wheel(IncompatibleWheel::NoBinary),
            "has no source distribution",
            "have no source distribution",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::Tag(IncompatibleTag::Invalid)),
            "has no wheels with valid tags",
            "have no wheels with valid tags",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::Tag(IncompatibleTag::Python)),
            "has no wheels with a matching Python implementation tag",
            "have no wheels with a matching Python implementation tag",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::Tag(IncompatibleTag::Abi)),
            "has no wheels with a matching Python ABI tag",
            "have no wheels with a matching Python ABI tag",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::Tag(IncompatibleTag::FreethreadedAbi)),
            "has no wheels with a free-threading compatible ABI tag",
            "have no wheels with a free-threading compatible ABI tag",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::Tag(IncompatibleTag::AbiPythonVersion)),
            "has no wheels with a matching Python version tag",
            "have no wheels with a matching Python version tag",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::Tag(IncompatibleTag::Platform)),
            "has no wheels with a matching platform tag",
            "have no wheels with a matching platform tag",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::Yanked(Yanked::Bool(true))),
            "was yanked",
            "were yanked",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::Yanked(Yanked::Reason(
                "  broken release...  ".into(),
            ))),
            "was yanked (reason: broken release)",
            "were yanked (reason: broken release)",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::ExcludeNewer(Some(0))),
            "was published after the exclude newer time",
            "were published after the exclude newer time",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::ExcludeNewer(None)),
            "has no publish time",
            "have no publish time",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::RequiresPython(
                requires_python.clone(),
                PythonRequirementKind::Installed,
            )),
            "requires Python >=3.12",
            "require Python >=3.12",
        ),
        (
            IncompatibleDist::Wheel(IncompatibleWheel::MissingPlatform(MarkerTree::TRUE)),
            "has no compatible wheels",
            "have no compatible wheels",
        ),
        (
            IncompatibleDist::Source(IncompatibleSource::NoBuild),
            "has no usable wheels",
            "have no usable wheels",
        ),
        (
            IncompatibleDist::Source(IncompatibleSource::Yanked(Yanked::Bool(true))),
            "was yanked",
            "were yanked",
        ),
        (
            IncompatibleDist::Source(IncompatibleSource::Yanked(Yanked::Reason(
                "  broken release...  ".into(),
            ))),
            "was yanked (reason: broken release)",
            "were yanked (reason: broken release)",
        ),
        (
            IncompatibleDist::Source(IncompatibleSource::ExcludeNewer(Some(0))),
            "was published after the exclude newer time",
            "were published after the exclude newer time",
        ),
        (
            IncompatibleDist::Source(IncompatibleSource::ExcludeNewer(None)),
            "has no publish time",
            "have no publish time",
        ),
        (
            IncompatibleDist::Source(IncompatibleSource::RequiresPython(
                requires_python,
                PythonRequirementKind::Target,
            )),
            "requires Python >=3.12",
            "require Python >=3.12",
        ),
        (
            IncompatibleDist::Source(IncompatibleSource::NotPep625Filename),
            "has a non-PEP 625-compliant source distribution filename",
            "have a non-PEP 625-compliant source distribution filename",
        ),
        (
            IncompatibleDist::Unavailable,
            "has no available distributions",
            "have no available distributions",
        ),
    ];

    for (incompatibility, singular, plural) in cases {
        assert_eq!(incompatibility.singular_message(), singular);
        assert_eq!(incompatibility.plural_message(), plural);
    }
}
