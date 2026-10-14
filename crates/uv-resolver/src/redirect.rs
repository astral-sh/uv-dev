use uv_git::GitResolver;
use uv_pep508::VerbatimUrl;
use uv_pypi_types::{ParsedGitDirectoryUrl, ParsedGitPathUrl, ParsedUrl, VerbatimParsedUrl};
use uv_redacted::DisplaySafeUrl;

/// Map a URL to a precise URL, if possible.
pub(crate) fn url_to_precise(mut url: VerbatimParsedUrl, git: &GitResolver) -> VerbatimParsedUrl {
    let (ParsedUrl::GitDirectory(ParsedGitDirectoryUrl { url: git_url, .. })
    | ParsedUrl::GitPath(ParsedGitPathUrl { url: git_url, .. })) = &mut url.parsed_url
    else {
        return url;
    };
    let Some(precise) = git.precise(git_url.clone()) else {
        debug_assert!(false, "Unresolved Git URL: {}, {git_url:?}", url.verbatim);
        return url;
    };
    *git_url = precise;

    let redirect = DisplaySafeUrl::from(url.parsed_url.clone());
    url.verbatim = apply_redirect(&url.verbatim, redirect);
    url
}

/// Given a [`VerbatimUrl`] and a redirect, apply the redirect to the URL while preserving as much
/// of the verbatim representation as possible.
fn apply_redirect(url: &VerbatimUrl, redirect: DisplaySafeUrl) -> VerbatimUrl {
    let redirect = VerbatimUrl::from_url(redirect);

    // The redirect should be the "same" URL, but with a specific commit hash added after the `@`.
    // We take advantage of this to preserve as much of the verbatim representation as possible.
    let Some(given) = url.given() else {
        return redirect;
    };
    let (given, fragment) = given
        .split_once('#')
        .map_or((given, None), |(prefix, suffix)| (prefix, Some(suffix)));
    let Some((_, precise_suffix)) = redirect.raw().path().rsplit_once('@') else {
        return redirect;
    };

    let prefix = match url.raw().path().rsplit_once('@') {
        None => given,
        Some((_, parsed_suffix)) => match given.rsplit_once('@') {
            Some((prefix, given_suffix)) if given_suffix == parsed_suffix => prefix,
            _ => return redirect,
        },
    };
    let mut given = format!("{prefix}@{precise_suffix}");
    if let Some(fragment) = fragment {
        given.push('#');
        given.push_str(fragment);
    }
    redirect.with_given(given)
}

#[cfg(test)]
mod tests {
    use uv_pep508::{VerbatimUrl, VerbatimUrlError};
    use uv_redacted::DisplaySafeUrl;

    use crate::redirect::apply_redirect;

    fn assert_redirect(url: &VerbatimUrl, redirect: DisplaySafeUrl, expected: &VerbatimUrl) {
        let actual = apply_redirect(url, redirect);
        assert_eq!(actual.given(), expected.given());
        assert_eq!(&actual, expected);
    }

    #[test]
    fn test_apply_redirect() -> Result<(), VerbatimUrlError> {
        // If there's no `@` in the original representation, we can just append the precise suffix
        // to the given representation.
        let verbatim = VerbatimUrl::parse_url("https://github.com/flask.git")?
            .with_given("git+https://github.com/flask.git");
        let redirect = DisplaySafeUrl::parse(
            "https://github.com/flask.git@b90a4f1f4a370e92054b9cc9db0efcb864f87ebe",
        )?;

        let expected = VerbatimUrl::parse_url(
            "https://github.com/flask.git@b90a4f1f4a370e92054b9cc9db0efcb864f87ebe",
        )?
        .with_given("git+https://github.com/flask.git@b90a4f1f4a370e92054b9cc9db0efcb864f87ebe");
        assert_redirect(&verbatim, redirect, &expected);

        // If there's an `@` in the original representation, and it's stable between the parsed and
        // given representations, we preserve everything that precedes the `@` in the precise
        // representation.
        let verbatim = VerbatimUrl::parse_url("https://github.com/flask.git@main")?
            .with_given("git+https://${DOMAIN}.com/flask.git@main");
        let redirect = DisplaySafeUrl::parse(
            "https://github.com/flask.git@b90a4f1f4a370e92054b9cc9db0efcb864f87ebe",
        )?;

        let expected = VerbatimUrl::parse_url(
            "https://github.com/flask.git@b90a4f1f4a370e92054b9cc9db0efcb864f87ebe",
        )?
        .with_given("git+https://${DOMAIN}.com/flask.git@b90a4f1f4a370e92054b9cc9db0efcb864f87ebe");
        assert_redirect(&verbatim, redirect, &expected);

        // If there's a conflict after the `@`, discard the original representation.
        let verbatim = VerbatimUrl::parse_url("https://github.com/flask.git@main")?
            .with_given("git+https://github.com/flask.git@${TAG}");
        let redirect = DisplaySafeUrl::parse(
            "https://github.com/flask.git@b90a4f1f4a370e92054b9cc9db0efcb864f87ebe",
        )?;

        let expected = VerbatimUrl::parse_url(
            "https://github.com/flask.git@b90a4f1f4a370e92054b9cc9db0efcb864f87ebe",
        )?;
        assert_redirect(&verbatim, redirect, &expected);

        // We should preserve subdirectory fragments.
        let verbatim = VerbatimUrl::parse_url("https://github.com/flask.git#subdirectory=src")?
            .with_given("git+https://github.com/flask.git#subdirectory=src");
        let redirect = DisplaySafeUrl::parse(
            "https://github.com/flask.git@b90a4f1f4a370e92054b9cc9db0efcb864f87ebe#subdirectory=src",
        )?;

        let expected = VerbatimUrl::parse_url(
            "https://github.com/flask.git@b90a4f1f4a370e92054b9cc9db0efcb864f87ebe#subdirectory=src",
        )?.with_given("git+https://github.com/flask.git@b90a4f1f4a370e92054b9cc9db0efcb864f87ebe#subdirectory=src");

        assert_redirect(&verbatim, redirect, &expected);

        Ok(())
    }
}
