use std::io;

use uv_cache::{Cache, CacheBucket};
use uv_pypi_types::ResolutionMetadata;

#[test]
fn package_cleanup_preserves_linked_wheels_bucket() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    fs_err::create_dir(cache.root())?;
    let external = root.path().join("external");
    let sentinel = external.join("pypi/demo/payload");
    fs_err::create_dir_all(sentinel.parent().expect("payload has a parent"))?;
    fs_err::write(&sentinel, "external contents")?;
    let link = cache.bucket(CacheBucket::Wheels);
    fs_err::create_dir_all(link.parent().expect("bucket has a parent"))?;
    uv_fs::create_symlink(&external, &link)?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    assert!(fs_err::symlink_metadata(link)?.is_symlink());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_wheels_pypi_namespace() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    fs_err::create_dir(cache.root())?;
    let external = root.path().join("external");
    let sentinel = external.join("demo/payload");
    fs_err::create_dir_all(sentinel.parent().expect("payload has a parent"))?;
    fs_err::write(&sentinel, "external contents")?;
    let link = cache.bucket(CacheBucket::Wheels).join("pypi");
    fs_err::create_dir_all(link.parent().expect("bucket has a parent"))?;
    uv_fs::create_symlink(&external, &link)?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    assert!(fs_err::symlink_metadata(link)?.is_symlink());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_wheels_index_namespace() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    fs_err::create_dir(cache.root())?;
    let external = root.path().join("external");
    let sentinel = external.join("index-id/demo/payload");
    fs_err::create_dir_all(sentinel.parent().expect("payload has a parent"))?;
    fs_err::write(&sentinel, "external contents")?;
    let link = cache.bucket(CacheBucket::Wheels).join("index");
    fs_err::create_dir_all(link.parent().expect("bucket has a parent"))?;
    uv_fs::create_symlink(&external, &link)?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    assert!(fs_err::symlink_metadata(link)?.is_symlink());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_wheels_url_namespace() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    fs_err::create_dir(cache.root())?;
    let external = root.path().join("external");
    let sentinel = external.join("url-id/demo/payload");
    fs_err::create_dir_all(sentinel.parent().expect("payload has a parent"))?;
    fs_err::write(&sentinel, "external contents")?;
    let link = cache.bucket(CacheBucket::Wheels).join("url");
    fs_err::create_dir_all(link.parent().expect("bucket has a parent"))?;
    uv_fs::create_symlink(&external, &link)?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    assert!(fs_err::symlink_metadata(link)?.is_symlink());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_source_bucket() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    fs_err::create_dir(cache.root())?;
    let external = root.path().join("external");
    let sentinel = external.join("pypi/demo/payload");
    fs_err::create_dir_all(sentinel.parent().expect("payload has a parent"))?;
    fs_err::write(&sentinel, "external contents")?;
    let link = cache.bucket(CacheBucket::SourceDistributions);
    fs_err::create_dir_all(link.parent().expect("bucket has a parent"))?;
    uv_fs::create_symlink(&external, &link)?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    assert!(fs_err::symlink_metadata(link)?.is_symlink());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_source_pypi_namespace() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    fs_err::create_dir(cache.root())?;
    let external = root.path().join("external");
    let sentinel = external.join("demo/payload");
    fs_err::create_dir_all(sentinel.parent().expect("payload has a parent"))?;
    fs_err::write(&sentinel, "external contents")?;
    let link = cache.bucket(CacheBucket::SourceDistributions).join("pypi");
    fs_err::create_dir_all(link.parent().expect("bucket has a parent"))?;
    uv_fs::create_symlink(&external, &link)?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    assert!(fs_err::symlink_metadata(link)?.is_symlink());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_source_index_namespace() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    fs_err::create_dir(cache.root())?;
    let external = root.path().join("external");
    let sentinel = external.join("index-id/demo/payload");
    fs_err::create_dir_all(sentinel.parent().expect("payload has a parent"))?;
    fs_err::write(&sentinel, "external contents")?;
    let link = cache.bucket(CacheBucket::SourceDistributions).join("index");
    fs_err::create_dir_all(link.parent().expect("bucket has a parent"))?;
    uv_fs::create_symlink(&external, &link)?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    assert!(fs_err::symlink_metadata(link)?.is_symlink());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_simple_bucket() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    fs_err::create_dir(cache.root())?;
    let external = root.path().join("external");
    let sentinel = external.join("pypi/demo.rkyv");
    fs_err::create_dir_all(sentinel.parent().expect("payload has a parent"))?;
    fs_err::write(&sentinel, "external contents")?;
    let link = cache.bucket(CacheBucket::Simple);
    fs_err::create_dir_all(link.parent().expect("bucket has a parent"))?;
    uv_fs::create_symlink(&external, &link)?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    assert!(fs_err::symlink_metadata(link)?.is_symlink());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_simple_pypi_namespace() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    fs_err::create_dir(cache.root())?;
    let external = root.path().join("external");
    let sentinel = external.join("demo.rkyv");
    fs_err::create_dir_all(sentinel.parent().expect("payload has a parent"))?;
    fs_err::write(&sentinel, "external contents")?;
    let link = cache.bucket(CacheBucket::Simple).join("pypi");
    fs_err::create_dir_all(link.parent().expect("bucket has a parent"))?;
    uv_fs::create_symlink(&external, &link)?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    assert!(fs_err::symlink_metadata(link)?.is_symlink());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_simple_index_namespace() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    fs_err::create_dir(cache.root())?;
    let external = root.path().join("external");
    let sentinel = external.join("index-id/demo.rkyv");
    fs_err::create_dir_all(sentinel.parent().expect("payload has a parent"))?;
    fs_err::write(&sentinel, "external contents")?;
    let link = cache.bucket(CacheBucket::Simple).join("index");
    fs_err::create_dir_all(link.parent().expect("bucket has a parent"))?;
    uv_fs::create_symlink(&external, &link)?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert_eq!(fs_err::read_to_string(sentinel)?, "external contents");
    assert!(fs_err::symlink_metadata(link)?.is_symlink());
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
fn package_cleanup_removes_owned_url_source() -> io::Result<()> {
    let metadata =
        ResolutionMetadata::parse_metadata(b"Metadata-Version: 2.1\nName: demo\nVersion: 1.0\n")
            .map_err(io::Error::other)?;
    let metadata = rmp_serde::to_vec_named(&metadata).map_err(io::Error::other)?;
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    let bucket = cache.bucket(CacheBucket::SourceDistributions);
    fs_err::create_dir_all(&bucket)?;
    let source = bucket.join("url");
    let revision = source.join("resource/revision");
    fs_err::create_dir_all(&revision)?;
    fs_err::write(revision.join("metadata.msgpack"), &metadata)?;
    let sentinel = revision.join("payload");
    fs_err::write(&sentinel, "contents")?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert!(!sentinel.exists());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_url_source() -> io::Result<()> {
    let metadata =
        ResolutionMetadata::parse_metadata(b"Metadata-Version: 2.1\nName: demo\nVersion: 1.0\n")
            .map_err(io::Error::other)?;
    let metadata = rmp_serde::to_vec_named(&metadata).map_err(io::Error::other)?;
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    let bucket = cache.bucket(CacheBucket::SourceDistributions);
    fs_err::create_dir_all(&bucket)?;
    let source = root.path().join("external");
    fs_err::create_dir(&source)?;
    uv_fs::create_symlink(&source, bucket.join("url"))?;
    let revision = source.join("resource/revision");
    fs_err::create_dir_all(&revision)?;
    fs_err::write(revision.join("metadata.msgpack"), &metadata)?;
    let sentinel = revision.join("payload");
    fs_err::write(&sentinel, "contents")?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert!(sentinel.exists());
    Ok(())
}

#[test]
fn package_cleanup_removes_owned_path_source() -> io::Result<()> {
    let metadata =
        ResolutionMetadata::parse_metadata(b"Metadata-Version: 2.1\nName: demo\nVersion: 1.0\n")
            .map_err(io::Error::other)?;
    let metadata = rmp_serde::to_vec_named(&metadata).map_err(io::Error::other)?;
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    let bucket = cache.bucket(CacheBucket::SourceDistributions);
    fs_err::create_dir_all(&bucket)?;
    let source = bucket.join("path");
    let revision = source.join("resource/revision");
    fs_err::create_dir_all(&revision)?;
    fs_err::write(revision.join("metadata.msgpack"), &metadata)?;
    let sentinel = revision.join("payload");
    fs_err::write(&sentinel, "contents")?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert!(!sentinel.exists());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_path_source() -> io::Result<()> {
    let metadata =
        ResolutionMetadata::parse_metadata(b"Metadata-Version: 2.1\nName: demo\nVersion: 1.0\n")
            .map_err(io::Error::other)?;
    let metadata = rmp_serde::to_vec_named(&metadata).map_err(io::Error::other)?;
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    let bucket = cache.bucket(CacheBucket::SourceDistributions);
    fs_err::create_dir_all(&bucket)?;
    let source = root.path().join("external");
    fs_err::create_dir(&source)?;
    uv_fs::create_symlink(&source, bucket.join("path"))?;
    let revision = source.join("resource/revision");
    fs_err::create_dir_all(&revision)?;
    fs_err::write(revision.join("metadata.msgpack"), &metadata)?;
    let sentinel = revision.join("payload");
    fs_err::write(&sentinel, "contents")?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert!(sentinel.exists());
    Ok(())
}

#[test]
fn package_cleanup_removes_owned_git_source() -> io::Result<()> {
    let metadata =
        ResolutionMetadata::parse_metadata(b"Metadata-Version: 2.1\nName: demo\nVersion: 1.0\n")
            .map_err(io::Error::other)?;
    let metadata = rmp_serde::to_vec_named(&metadata).map_err(io::Error::other)?;
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    let bucket = cache.bucket(CacheBucket::SourceDistributions);
    fs_err::create_dir_all(&bucket)?;
    let source = bucket.join("git");
    let revision = source.join("resource/revision");
    fs_err::create_dir_all(&revision)?;
    fs_err::write(revision.join("metadata.msgpack"), &metadata)?;
    let sentinel = revision.join("payload");
    fs_err::write(&sentinel, "contents")?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert!(!sentinel.exists());
    Ok(())
}

#[test]
fn package_cleanup_preserves_linked_git_source() -> io::Result<()> {
    let metadata =
        ResolutionMetadata::parse_metadata(b"Metadata-Version: 2.1\nName: demo\nVersion: 1.0\n")
            .map_err(io::Error::other)?;
    let metadata = rmp_serde::to_vec_named(&metadata).map_err(io::Error::other)?;
    let root = tempfile::tempdir()?;
    let cache = Cache::from_path(root.path().join("cache"));
    let bucket = cache.bucket(CacheBucket::SourceDistributions);
    fs_err::create_dir_all(&bucket)?;
    let source = root.path().join("external");
    fs_err::create_dir(&source)?;
    uv_fs::create_symlink(&source, bucket.join("git"))?;
    let revision = source.join("resource/revision");
    fs_err::create_dir_all(&revision)?;
    fs_err::write(revision.join("metadata.msgpack"), &metadata)?;
    let sentinel = revision.join("payload");
    fs_err::write(&sentinel, "contents")?;

    cache.remove(&"demo".parse().map_err(io::Error::other)?)?;

    assert!(sentinel.exists());
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
