use std::error::Error;

use uv_git_types::{GitLfs, GitOid, GitReference, GitUrl};
use uv_redacted::DisplaySafeUrl;

const REPOSITORY: &str =
    "https://fake-user:fake-password@git.example.invalid/repo.git?keep=1#fragment";

fn references(revision: &str) -> [GitReference; 5] {
    [
        GitReference::Branch(revision.to_string()),
        GitReference::Tag(revision.to_string()),
        GitReference::BranchOrTag(revision.to_string()),
        GitReference::BranchOrTagOrCommit(revision.to_string()),
        GitReference::NamedRef(revision.to_string()),
    ]
}

fn assert_conversion(
    reference: GitReference,
    precise: Option<GitOid>,
    path: &str,
) -> Result<(), Box<dyn Error>> {
    let description = format!("{reference:?}");
    let git = GitUrl::from_fields(
        DisplaySafeUrl::parse(REPOSITORY)?,
        reference,
        precise,
        GitLfs::Disabled,
    )?;
    assert_eq!(git.url().as_str(), REPOSITORY);

    let expected =
        format!("https://fake-user:fake-password@git.example.invalid{path}?keep=1#fragment");
    let actual = DisplaySafeUrl::from(git);
    assert_eq!(actual.as_str(), expected, "{description}");
    assert_eq!(
        actual.to_string(),
        expected.replace("fake-password", "****"),
        "{description}"
    );
    Ok(())
}

#[test]
fn conversion_encodes_valued_references() -> Result<(), Box<dyn Error>> {
    let cases = [
        ("", "", "/repo.git@"),
        ("HEAD", "HEAD", "/repo.git@HEAD"),
        (
            "refs/pull/493/head-1.2_3~4",
            "refs/pull/493/head-1.2_3~4",
            "/repo.git@refs/pull/493/head-1.2_3~4",
        ),
        (
            "topic@a?b#c%d",
            "topic%40a%3Fb%23c%25d",
            "/repo.git@topic%40a%3Fb%23c%25d",
        ),
        (
            "literal%2Fslash",
            "literal%252Fslash",
            "/repo.git@literal%252Fslash",
        ),
        (
            "é/🚀",
            "%C3%A9/%F0%9F%9A%80",
            "/repo.git@%C3%A9/%F0%9F%9A%80",
        ),
        ("a\\b\n\0", "a%5Cb%0A%00", "/repo.git@a%5Cb%0A%00"),
        (
            " spaced + : ",
            "%20spaced%20%2B%20%3A%20",
            "/repo.git@%20spaced%20%2B%20%3A%20",
        ),
        // Percent signs are encoded before the URL path setter normalizes dot segments.
        ("topic/./tail", "topic/./tail", "/repo.git@topic/tail"),
        ("topic/../tail", "topic/../tail", "/tail"),
        (
            "topic/%2e%2e/tail",
            "topic/%252e%252e/tail",
            "/repo.git@topic/%252e%252e/tail",
        ),
    ];
    for (revision, encoded, path) in cases {
        for reference in references(revision) {
            assert_eq!(reference.as_url_rev().as_deref(), Some(encoded));
            assert_conversion(reference, None, path)?;
        }
    }
    Ok(())
}

#[test]
fn conversion_omits_the_default_branch() -> Result<(), Box<dyn Error>> {
    assert_eq!(GitReference::DefaultBranch.as_url_rev(), None);
    assert_conversion(GitReference::DefaultBranch, None, "/repo.git")
}

#[test]
fn precise_commit_takes_precedence() -> Result<(), Box<dyn Error>> {
    let precise = "0dacfd662c64cb4ceb16e6cf65a157a8b715b979".parse::<GitOid>()?;
    let path = format!("/repo.git@{precise}");
    for reference in references("ignored@?#%/../ref")
        .into_iter()
        .chain([GitReference::DefaultBranch])
    {
        assert_conversion(reference, Some(precise), &path)?;
    }

    assert_conversion(
        GitReference::BranchOrTagOrCommit(precise.as_str().to_ascii_uppercase()),
        Some(precise),
        &path,
    )
}
