use std::io;

use uv_cache::{Cache, CacheBucket};
use uv_pypi_types::ResolutionMetadata;

#[test]
fn package_cleanup_preserves_linked_namespaces() -> io::Result<()> {
    for (bucket, namespace, payload) in [
        (CacheBucket::Wheels, "", "pypi/demo/payload"),
        (CacheBucket::Wheels, "pypi", "demo/payload"),
        (CacheBucket::Wheels, "index", "index-id/demo/payload"),
        (CacheBucket::Wheels, "url", "url-id/demo/payload"),
        (CacheBucket::SourceDistributions, "", "pypi/demo/payload"),
        (CacheBucket::SourceDistributions, "pypi", "demo/payload"),
        (
            CacheBucket::SourceDistributions,
            "index",
            "index-id/demo/payload",
        ),
        (CacheBucket::Simple, "", "pypi/demo.rkyv"),
        (CacheBucket::Simple, "pypi", "demo.rkyv"),
        (CacheBucket::Simple, "index", "index-id/demo.rkyv"),
    ] {
        let root = tempfile::tempdir()?;
        let cache = Cache::from_path(root.path().join("cache"));
        fs_err::create_dir(cache.root())?;
        let external = root.path().join("external");
        let sentinel = external.join(payload);
        fs_err::create_dir_all(sentinel.parent().expect("payload has a parent"))?;
        fs_err::write(&sentinel, "external contents")?;
        let link = if namespace.is_empty() {
            cache.bucket(bucket)
        } else {
            cache.bucket(bucket).join(namespace)
        };
        fs_err::create_dir_all(link.parent().expect("bucket has a parent"))?;
        uv_fs::create_symlink(&external, &link)?;

        cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

        assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
        assert!(fs_err::symlink_metadata(link)?.is_symlink());
    }
    Ok(())
}

#[test]
fn file_pruning_preserves_linked_bucket() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    fs_err::create_dir(cache.root())?;
    let external = root.path().join("external");
    fs_err::create_dir(&external)?;
    let sentinel = external.join("object");
    fs_err::write(&sentinel, "external contents")?;
    uv_fs::create_symlink(&external, cache.bucket(CacheBucket::Files))?;

    cache.prune_archive_files()?;

    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    Ok(())
}

#[test]
fn package_cleanup_handles_source_namespaces_without_following_links() -> io::Result<()> {
    let metadata =
        ResolutionMetadata::parse_metadata(b"Metadata-Version: 2.1\nName: demo\nVersion: 1.0\n")
            .map_err(io::Error::other)?;
    let metadata = rmp_serde::to_vec_named(&metadata).map_err(io::Error::other)?;
    for namespace in ["url", "path", "git"] {
        for linked in [false, true] {
            let root = tempfile::tempdir()?;
            let cache = Cache::from_path(root.path().join("cache"));
            let bucket = cache.bucket(CacheBucket::SourceDistributions);
            fs_err::create_dir_all(&bucket)?;
            let source = if linked {
                let external = root.path().join("external");
                fs_err::create_dir(&external)?;
                uv_fs::create_symlink(&external, bucket.join(namespace))?;
                external
            } else {
                bucket.join(namespace)
            };
            let revision = source.join("resource/revision");
            fs_err::create_dir_all(&revision)?;
            fs_err::write(revision.join("metadata.msgpack"), &metadata)?;
            let sentinel = revision.join("payload");
            fs_err::write(&sentinel, "contents")?;

            cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

            assert_eq!(sentinel.exists(), linked);
        }
    }
    Ok(())
}

#[test]
fn package_cleanup_supports_root_links_and_unlinks_entry_links() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let target = root.path().join("cache-target");
    fs_err::create_dir(&target)?;
    let cache = Cache::from_path(root.path().join("cache-link"));
    uv_fs::create_symlink(&target, cache.root())?;
    let package = cache.bucket(CacheBucket::Wheels).join("pypi/demo");
    fs_err::create_dir_all(&package)?;
    fs_err::write(package.join("payload"), "cache contents")?;
    let external = root.path().join("external");
    fs_err::create_dir(&external)?;
    let sentinel = external.join("sentinel");
    fs_err::write(&sentinel, "external contents")?;
    uv_fs::create_symlink(&external, package.join("entry-link"))?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert!(!package.exists());
    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    assert!(cache.root().is_dir());
    assert!(fs_err::symlink_metadata(cache.root())?.is_symlink());
    Ok(())
}
