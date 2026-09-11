use std::io;

use uv_fs::HardlinkScanner;

#[test]
fn disabled_scanner_uses_the_ordinary_fallback() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    let mut scanner = HardlinkScanner::disabled();
    assert!(
        scanner
            .files_with_one_hardlink(&root.path().join("missing"))?
            .is_none()
    );
    Ok(())
}
