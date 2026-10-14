//! Serialize tests that register managed interpreters in the real user registry.

use std::ffi::c_void;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use uv_fs::{LockedFile, LockedFileMode};
use uv_test::TestContext;
use windows_registry::{CURRENT_USER, Key};

const COMPANY_KEY: &str = r"Software\Python\Astral";
// HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)
const FILE_NOT_FOUND: i32 = 0x8007_0002_u32.cast_signed();

/// A guard that removes only registrations belonging to this test's managed directory.
pub(super) struct RegistryFixture {
    managed_dir: PathBuf,
    armed: bool,
    // The lock must be held until cleanup finishes, including during unwinding.
    _lock: LockedFile,
}

impl RegistryFixture {
    pub(super) async fn acquire(context: &TestContext) -> anyhow::Result<Self> {
        let lock = LockedFile::acquire(
            std::env::temp_dir().join("uv-test-python-registry.lock"),
            LockedFileMode::Exclusive,
            COMPANY_KEY,
        )
        .await?;
        ensure!(
            open_company_key()?.is_none(),
            "refusing to modify existing HKCU\\{COMPANY_KEY} registry entries"
        );
        Ok(Self {
            managed_dir: context.temp_dir.join("managed"),
            armed: true,
            _lock: lock,
        })
    }

    pub(super) fn cleanup(mut self) -> anyhow::Result<()> {
        self.armed = false;
        self.remove_owned_entries()
    }

    fn remove_owned_entries(&self) -> anyhow::Result<()> {
        let Some(company) = open_company_key()? else {
            return Ok(());
        };
        let managed_dir = dunce::canonicalize(&self.managed_dir).ok();
        // Collect first: deleting a key shifts the indices used by the registry iterator.
        let tags = company.keys()?.collect::<Vec<_>>();
        let mut unowned = Vec::new();
        let mut removed_owned = false;
        for tag in tags {
            let owned = company
                .open(format!(r"{tag}\InstallPath"))
                .and_then(|install_path| install_path.get_string(""))
                .ok()
                .and_then(|install_path| {
                    dunce::canonicalize(Path::new(&install_path).parent()?).ok()
                })
                .is_some_and(|parent| managed_dir.as_ref() == Some(&parent));
            if owned {
                let key = format!(r"{COMPANY_KEY}\{tag}");
                remove_empty_key(&format!(r"{key}\InstallPath"))
                    .with_context(|| format!("failed to remove test registration path {tag}"))?;
                remove_empty_key(&key)
                    .with_context(|| format!("failed to remove test registration {tag}"))?;
                removed_owned = true;
            } else {
                unowned.push(tag);
            }
        }
        ensure!(
            unowned.is_empty(),
            "refusing to remove unowned HKCU\\{COMPANY_KEY} registry entries: {unowned:?}"
        );
        ensure!(
            removed_owned,
            "refusing to remove HKCU\\{COMPANY_KEY} without an owned test registration"
        );
        ensure!(
            company.values()?.all(|(name, value)| {
                let expected = match name.as_str() {
                    "DisplayName" => "Astral Software Inc.",
                    "SupportUrl" => "https://github.com/astral-sh/uv",
                    _ => return false,
                };
                String::try_from(value).is_ok_and(|value| value == expected)
            }),
            "refusing to remove unexpected HKCU\\{COMPANY_KEY} registry values"
        );
        drop(company);
        remove_empty_key(COMPANY_KEY).context("failed to remove test registry key")
    }
}

impl Drop for RegistryFixture {
    fn drop(&mut self) {
        if self.armed
            && let Err(error) = self.remove_owned_entries()
        {
            let _ = writeln!(
                io::stderr().lock(),
                "failed to clean up test registry entries: {error:#}"
            );
        }
    }
}

fn open_company_key() -> anyhow::Result<Option<Key>> {
    match CURRENT_USER.open(COMPANY_KEY) {
        Ok(key) => Ok(Some(key)),
        Err(error) if error.code().0 == FILE_NOT_FOUND => Ok(None),
        Err(error) => Err(error).context("failed to inspect user registry"),
    }
}

#[expect(
    unsafe_code,
    reason = "Win32 provides nonrecursive registry key deletion"
)]
fn remove_empty_key(key: &str) -> io::Result<()> {
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegDeleteKeyW(key: *mut c_void, subkey: *const u16) -> i32;
    }

    let name = key.encode_utf16().chain([0]).collect::<Vec<_>>();
    // SAFETY: `CURRENT_USER` is a valid predefined registry handle, and `name` is a live,
    // NUL-terminated UTF-16 string. Unlike `remove_tree`, this call cannot remove subkeys created
    // by another process between our ownership check and deletion.
    let status = unsafe { RegDeleteKeyW(CURRENT_USER.as_raw(), name.as_ptr()) };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status))
    }
}
