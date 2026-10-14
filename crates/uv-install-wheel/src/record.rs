use data_encoding::BASE64URL_NOPAD;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, digest::Output};

/// Line in a RECORD file
/// <https://www.python.org/dev/peps/pep-0376/#record>
///
/// ```csv
/// tqdm/cli.py,sha256=x_c8nmc4Huc-lKEsAXj78ZiyqSJ9hJ71j7vltY67icw,10509
/// tqdm-4.62.3.dist-info/RECORD,,
/// ```
#[derive(Deserialize, Serialize, PartialOrd, PartialEq, Ord, Eq)]
pub struct RecordEntry {
    pub path: String,
    pub hash: Option<String>,
    pub size: Option<u64>,
}

/// Encode a SHA-256 digest for the hash column of a wheel's RECORD file.
pub(crate) fn encode_sha256(digest: &Output<Sha256>) -> String {
    format!("sha256={}", BASE64URL_NOPAD.encode_display(digest))
}
