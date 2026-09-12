use uv_configuration::{DependencyGroups, DependencyGroupsWithDefaults, DevMode};
use uv_normalize::{DefaultGroups, GroupName};

#[test]
fn non_group_dependencies_from_args() -> anyhow::Result<()> {
    let name: GroupName = "quality".parse()?;
    let defaults = [
        DefaultGroups::default(),
        DefaultGroups::List(vec![name.clone()]),
        DefaultGroups::All,
    ];

    for dev_mode in [
        None,
        Some(DevMode::Include),
        Some(DevMode::Exclude),
        Some(DevMode::Only),
    ] {
        for group in [vec![], vec![name.clone()]] {
            for no_group in [vec![], vec![name.clone()]] {
                for only_group in [vec![], vec![name.clone()]] {
                    for no_default_groups in [false, true] {
                        for all_groups in [false, true] {
                            // Include/exclude/default flags do not suppress requirements outside
                            // dependency groups. Only the `--only-*` flags do so.
                            let expected = dev_mode != Some(DevMode::Only) && only_group.is_empty();
                            let groups = DependencyGroups::from_args(
                                dev_mode,
                                group.clone(),
                                no_group.clone(),
                                no_default_groups,
                                only_group.clone(),
                                all_groups,
                            );
                            assert_eq!(
                                groups.includes_non_group_dependencies(),
                                expected,
                                "{groups:?}"
                            );
                            for defaults in &defaults {
                                let groups = groups.with_defaults(defaults.clone());
                                assert_eq!(
                                    groups.includes_non_group_dependencies(),
                                    expected,
                                    "{groups:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn non_group_dependencies_from_convenience_constructors() -> anyhow::Result<()> {
    assert!(DependencyGroups::default().includes_non_group_dependencies());
    assert!(DependencyGroupsWithDefaults::none().includes_non_group_dependencies());
    assert!(DependencyGroups::from_group("quality".parse()?).includes_non_group_dependencies());
    assert!(DependencyGroups::from_all_groups().includes_non_group_dependencies());

    for (dev_mode, expected) in [
        (DevMode::Include, true),
        (DevMode::Exclude, true),
        (DevMode::Only, false),
    ] {
        let groups = DependencyGroups::from_dev_mode(dev_mode);
        assert_eq!(groups.includes_non_group_dependencies(), expected);
        assert_eq!(
            groups
                .with_defaults(DefaultGroups::All)
                .includes_non_group_dependencies(),
            expected
        );
    }
    Ok(())
}

#[test]
fn non_group_dependencies_from_dev_flags() {
    for dev in [false, true] {
        for no_dev in [false, true] {
            for only_dev in [false, true] {
                let groups = DependencyGroups::from_args(
                    DevMode::from_args(dev, no_dev, only_dev),
                    vec![],
                    vec![],
                    false,
                    vec![],
                    false,
                );
                assert_eq!(groups.includes_non_group_dependencies(), !only_dev);
            }
        }
    }
}
