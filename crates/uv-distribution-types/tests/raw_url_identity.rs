use uv_distribution_types::{DistributionId, FileLocation, Identifier, ResourceId, UrlString};

fn url_string(raw: &str) -> UrlString {
    serde_json::from_value(serde_json::Value::String(raw.to_owned())).expect("valid URL string")
}

#[test]
fn absolute_url_identity_and_serialization_use_raw_bytes() {
    for raw in [
        "https://alice:dummy-password@example.test/package.whl",
        "https://dummy-token@example.test/package.whl",
        "ssh://git@example.test/repo.git",
        "git+ssh://git@example.test/repo.git?rev=main#fragment",
        "https://example.test/package.whl?X-Amz%2DSignature=dummy-signature&sig=dummy-sas#fragment",
        "https://alice:dummy-password@[invalid-host/package.whl",
        "https://example.test:invalid/package.whl?sig=dummy-sas#fragment",
    ] {
        let url = url_string(raw);
        let location = FileLocation::AbsoluteUrl(url.clone());

        assert_eq!(url.as_ref(), raw);
        assert_eq!(serde_json::to_value(&url).expect("serialized URL"), raw);
        assert_eq!(
            location.distribution_id(),
            DistributionId::AbsoluteUrl(raw.to_owned())
        );
        assert_eq!(
            location.resource_id(),
            ResourceId::AbsoluteUrl(raw.to_owned())
        );
    }
}

#[test]
fn redacted_urls_do_not_share_identities() {
    let first = url_string(
        "https://alice:dummy-first@example.test/package.whl?X-Amz-Signature=dummy-first",
    );
    let second = url_string(
        "https://alice:dummy-second@example.test/package.whl?X-Amz-Signature=dummy-second",
    );

    assert_eq!(
        first.to_url().expect("valid first URL").to_string(),
        second.to_url().expect("valid second URL").to_string()
    );
    let first = FileLocation::AbsoluteUrl(first);
    let second = FileLocation::AbsoluteUrl(second);
    assert_ne!(first.distribution_id(), second.distribution_id());
    assert_ne!(first.resource_id(), second.resource_id());
}
