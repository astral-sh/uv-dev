use std::array::IntoIter;
use std::cell::RefCell;
use std::error::Error;
use std::str::FromStr;

use uv_distribution_filename::WheelFilename;
use uv_metadata::{Error as MetadataError, find_archive_dist_info};

type TestResult = Result<(), Box<dyn Error>>;
type MetadataResult = Result<(u32, &'static str), MetadataError>;

fn demo_wheel() -> Result<WheelFilename, Box<dyn Error>> {
    Ok(WheelFilename::from_str("demo-1.0-py3-none-any.whl")?)
}

fn expect_error<T>(result: Result<T, MetadataError>) -> Result<MetadataError, Box<dyn Error>> {
    result
        .err()
        .ok_or_else(|| "expected a metadata error".into())
}

#[test]
fn non_metadata_paths_are_ignored() -> TestResult {
    let filename = demo_wheel()?;
    let files = [
        (1, "demo/__init__.py"),
        (2, "demo-1.0.dist-info/WHEEL"),
        (3, "demo-1.0.dist-info/nested/METADATA"),
        (4, "demo-1.0.DIST-INFO/METADATA"),
        (5, "demo-1.0.dist-info/METADATA/"),
        (6, "METADATA"),
    ];

    let error = expect_error(find_archive_dist_info(&filename, files.into_iter()))?;
    assert_eq!(error.to_string(), "No .dist-info directory found");
    let MetadataError::MissingDistInfo = error else {
        return Err("expected MissingDistInfo".into());
    };
    Ok(())
}

#[test]
fn unique_metadata_preserves_payload_and_borrow() -> TestResult {
    let filename = demo_wheel()?;
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
fn wrong_package_name_preserves_error_fields() -> TestResult {
    let filename = demo_wheel()?;
    let files = [(17, "other-1.0.dist-info/METADATA")];

    let error = expect_error(find_archive_dist_info(&filename, files.into_iter()))?;
    assert_eq!(
        error.to_string(),
        "The .dist-info directory other-1.0 does not start with the normalized package name: demo"
    );
    let MetadataError::MissingDistInfoPackageName(prefix, package) = error else {
        return Err("expected MissingDistInfoPackageName".into());
    };
    assert_eq!((prefix.as_str(), package.as_str()), ("other-1.0", "demo"));
    Ok(())
}

#[test]
fn multiple_metadata_preserves_every_prefix_in_input_order() -> TestResult {
    let filename = demo_wheel()?;
    let files = [
        (11, "other-2.0.dist-info/METADATA"),
        (13, "demo-1.0.dist-info/WHEEL"),
        (17, ".dist-info/METADATA"),
        (19, "other-2.0.dist-info/METADATA"),
        (23, "demo/__init__.py"),
        (29, "demo-1.0.dist-info/METADATA"),
    ];

    let error = expect_error(find_archive_dist_info(&filename, files.into_iter()))?;
    assert_eq!(
        error.to_string(),
        "Multiple .dist-info directories found: other-2.0, , other-2.0, demo-1.0"
    );
    let MetadataError::MultipleDistInfo(prefixes) = error else {
        return Err("expected MultipleDistInfo".into());
    };
    assert_eq!(prefixes, "other-2.0, , other-2.0, demo-1.0");
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
enum Event {
    Next(Option<u32>),
    Dropped,
    Returned,
}

struct TracedIterator<'a, const N: usize> {
    steps: IntoIter<Option<(u32, &'static str)>, N>,
    events: &'a RefCell<Vec<Event>>,
}

impl<const N: usize> Iterator for TracedIterator<'_, N> {
    type Item = (u32, &'static str);

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.steps.next().flatten();
        self.events
            .borrow_mut()
            .push(Event::Next(item.map(|(payload, _)| payload)));
        item
    }
}

impl<const N: usize> Drop for TracedIterator<'_, N> {
    fn drop(&mut self) {
        self.events.borrow_mut().push(Event::Dropped);
    }
}

fn observe<const N: usize>(
    filename: &WheelFilename,
    steps: [Option<(u32, &'static str)>; N],
) -> (MetadataResult, Vec<Event>) {
    let events = RefCell::new(Vec::new());
    let result = find_archive_dist_info(
        filename,
        TracedIterator {
            steps: steps.into_iter(),
            events: &events,
        },
    );
    events.borrow_mut().push(Event::Returned);
    (result, events.into_inner())
}

#[test]
fn missing_metadata_stops_at_the_first_none() -> TestResult {
    let filename = demo_wheel()?;
    let (result, events) = observe(
        &filename,
        [
            Some((1, "demo/__init__.py")),
            None,
            Some((99, "demo-1.0.dist-info/METADATA")),
        ],
    );

    assert_eq!(
        result.map_err(|error| error.to_string()),
        Err("No .dist-info directory found".to_owned())
    );
    assert_eq!(
        events,
        [
            Event::Next(Some(1)),
            Event::Next(None),
            Event::Dropped,
            Event::Returned,
        ]
    );
    Ok(())
}

#[test]
fn unique_metadata_consumes_to_the_first_none() -> TestResult {
    let filename = demo_wheel()?;
    let (result, events) = observe(
        &filename,
        [
            Some((1, "demo-1.0.dist-info/WHEEL")),
            Some((2, "demo-1.0.dist-info/METADATA")),
            Some((3, "demo/__init__.py")),
            None,
            Some((99, "other-1.0.dist-info/METADATA")),
        ],
    );

    assert_eq!(result?, (2, "demo-1.0"));
    assert_eq!(
        events,
        [
            Event::Next(Some(1)),
            Event::Next(Some(2)),
            Event::Next(Some(3)),
            Event::Next(None),
            Event::Dropped,
            Event::Returned,
        ]
    );
    Ok(())
}

#[test]
fn multiple_metadata_consumes_to_the_first_none() -> TestResult {
    let filename = demo_wheel()?;
    let (result, events) = observe(
        &filename,
        [
            Some((1, "other-1.0.dist-info/METADATA")),
            Some((2, "demo-1.0.dist-info/METADATA")),
            Some((3, "demo/__init__.py")),
            Some((4, "other-1.0.dist-info/METADATA")),
            None,
            Some((99, "after-1.0.dist-info/METADATA")),
        ],
    );

    assert_eq!(
        result.map_err(|error| error.to_string()),
        Err("Multiple .dist-info directories found: other-1.0, demo-1.0, other-1.0".to_owned())
    );
    assert_eq!(
        events,
        [
            Event::Next(Some(1)),
            Event::Next(Some(2)),
            Event::Next(Some(3)),
            Event::Next(Some(4)),
            Event::Next(None),
            Event::Dropped,
            Event::Returned,
        ]
    );
    Ok(())
}

#[test]
fn package_name_error_consumes_to_the_first_none() -> TestResult {
    let filename = demo_wheel()?;
    let (result, events) = observe(
        &filename,
        [
            Some((1, "other-1.0.dist-info/METADATA")),
            None,
            Some((99, "demo-1.0.dist-info/METADATA")),
        ],
    );

    assert_eq!(
        result.map_err(|error| error.to_string()),
        Err(
            "The .dist-info directory other-1.0 does not start with the normalized package name: demo"
                .to_owned()
        )
    );
    assert_eq!(
        events,
        [
            Event::Next(Some(1)),
            Event::Next(None),
            Event::Dropped,
            Event::Returned,
        ]
    );
    Ok(())
}
