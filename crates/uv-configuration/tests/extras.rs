use anyhow::Result;
use uv_configuration::ExtrasSpecification;
use uv_normalize::{DefaultExtras, ExtraName};

#[test]
fn single_explicit_extra() -> Result<()> {
    let extra = "Foo_Bar".parse::<ExtraName>()?;
    let other = "other".parse::<ExtraName>()?;

    let cases = [
        (ExtrasSpecification::default(), None),
        (
            ExtrasSpecification::from_extra(vec![extra.clone()]),
            Some("foo-bar"),
        ),
        (
            ExtrasSpecification::from_extra(vec![extra.clone(), extra.clone()]),
            None,
        ),
        (
            ExtrasSpecification::from_extra(vec![extra.clone(), other.clone()]),
            None,
        ),
        (
            ExtrasSpecification::from_args(vec![], vec![], false, vec![extra.clone()], false),
            None,
        ),
        (
            ExtrasSpecification::from_args(
                vec![extra.clone()],
                vec![],
                false,
                vec![other.clone()],
                false,
            ),
            None,
        ),
        (
            ExtrasSpecification::from_args(
                vec![extra.clone()],
                vec![other.clone()],
                false,
                vec![],
                false,
            ),
            None,
        ),
        (ExtrasSpecification::from_all_extras(), None),
        (
            ExtrasSpecification::from_args(vec![extra.clone()], vec![], false, vec![], true),
            None,
        ),
        (
            ExtrasSpecification::from_args(vec![extra], vec![], true, vec![], false),
            None,
        ),
    ];

    for (specification, expected) in cases {
        assert_eq!(
            specification
                .history()
                .single_extra()
                .map(ExtraName::as_str),
            expected,
            "{specification:?}",
        );
    }

    Ok(())
}

#[test]
fn single_explicit_extra_with_defaults() -> Result<()> {
    let extra = "foo".parse::<ExtraName>()?;
    let other = "other".parse::<ExtraName>()?;
    let specification = ExtrasSpecification::from_extra(vec![extra.clone()]);

    assert_eq!(
        specification
            .with_defaults(DefaultExtras::default())
            .history()
            .single_extra(),
        Some(&extra),
    );
    assert_eq!(
        specification
            .with_defaults(DefaultExtras::List(vec![other]))
            .history()
            .single_extra(),
        None,
    );
    assert_eq!(
        specification
            .with_defaults(DefaultExtras::All)
            .history()
            .single_extra(),
        None,
    );

    Ok(())
}
