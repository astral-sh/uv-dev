use std::cell::Cell;
use std::error::Error as StdError;
use std::str::FromStr;

use uv_distribution_filename::WheelFilename;
use uv_metadata::{Error, find_archive_dist_info};

#[test]
fn metadata_cardinality_preserves_payload_and_borrow() -> Result<(), Box<dyn StdError>> {
    let filename = WheelFilename::from_str("demo-1.0-py3-none-any.whl")?;
    let ignored = [
        (1, "demo/__init__.py"),
        (2, "demo-1.0.dist-info/WHEEL"),
        (3, "demo-1.0.dist-info/nested/METADATA"),
        (4, "demo-1.0.DIST-INFO/METADATA"),
        (5, "demo-1.0.dist-info/METADATA/"),
        (6, "METADATA"),
    ];
    let error = find_archive_dist_info(&filename, ignored.into_iter())
        .expect_err("none of these paths is a dist-info METADATA file");
    assert!(matches!(error, Error::MissingDistInfo));

    let metadata_path = String::from("demo-1.0.dist-info/METADATA");
    let files = [
        (13, "demo-1.0.dist-info/WHEEL"),
        (31, metadata_path.as_str()),
        (47, "demo/__init__.py"),
    ];
    let (payload, prefix) = find_archive_dist_info(&filename, files.into_iter())?;
    assert_eq!((payload, prefix), (31, "demo-1.0"));
    assert_eq!(prefix.as_ptr(), metadata_path.as_ptr());
    Ok(())
}

#[test]
fn multiple_metadata_preserves_order_and_precedence() -> Result<(), Box<dyn StdError>> {
    let filename = WheelFilename::from_str("demo-1.0-py3-none-any.whl")?;
    let files = [
        (11, "other-2.0.dist-info/METADATA"),
        (13, "demo-1.0.dist-info/WHEEL"),
        (17, ".dist-info/METADATA"),
        (19, "other-2.0.dist-info/METADATA"),
        (23, "demo/__init__.py"),
        (29, "demo-1.0.dist-info/METADATA"),
    ];
    let error = find_archive_dist_info(&filename, files.into_iter())
        .expect_err("multiple METADATA files must take precedence over the package name");
    assert_eq!(
        error.to_string(),
        "Multiple .dist-info directories found: other-2.0, , other-2.0, demo-1.0"
    );
    assert!(matches!(error, Error::MultipleDistInfo(_)));
    Ok(())
}

#[test]
fn metadata_search_stops_at_the_first_none() -> Result<(), Box<dyn StdError>> {
    let filename = WheelFilename::from_str("demo-1.0-py3-none-any.whl")?;
    for (steps, expected) in [
        (
            &[
                Some("demo/__init__.py"),
                None,
                Some("demo-1.0.dist-info/METADATA"),
            ][..],
            Err("No .dist-info directory found"),
        ),
        (
            &[
                Some("demo-1.0.dist-info/METADATA"),
                None,
                Some("other-1.0.dist-info/METADATA"),
            ][..],
            Ok((0, "demo-1.0")),
        ),
        (
            &[
                Some("demo-1.0.dist-info/METADATA"),
                Some("other-1.0.dist-info/METADATA"),
                None,
                Some("after-1.0.dist-info/METADATA"),
            ][..],
            Err("Multiple .dist-info directories found: demo-1.0, other-1.0"),
        ),
    ] {
        let expected_calls = steps
            .iter()
            .position(Option::is_none)
            .expect("each case has a None sentinel")
            + 1;
        let calls = Cell::new(0usize);
        let mut remaining = steps.iter().copied().enumerate();
        let files = std::iter::from_fn(|| {
            calls.set(calls.get() + 1);
            remaining
                .next()
                .and_then(|(index, path)| path.map(|path| (index, path)))
        });
        assert_eq!(
            find_archive_dist_info(&filename, files).map_err(|error| error.to_string()),
            expected.map_err(str::to_owned),
        );
        assert_eq!(calls.get(), expected_calls);
    }
    Ok(())
}
