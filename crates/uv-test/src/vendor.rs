//! Pinned registry artifacts used by local test servers.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

use uv_configuration::TrustedHost;
use uv_fs::{LockedFile, LockedFileMode, persist_with_retry_sync, tempfile_in};
use uv_static::EnvVars;

use crate::TestContext;

pub(crate) struct VendorArtifact {
    pub(crate) filename: &'static str,
    url: &'static str,
    local_path: Option<&'static str>,
    pub(crate) sha256: &'static str,
    pub(crate) upload_time: Option<&'static str>,
    bytes: OnceLock<Arc<[u8]>>,
}

impl VendorArtifact {
    pub(crate) fn bytes(&self) -> Result<Arc<[u8]>> {
        if let Some(bytes) = self.bytes.get() {
            return Ok(Arc::clone(bytes));
        }

        let bytes = load_vendor_file(self)?;
        Ok(Arc::clone(self.bytes.get_or_init(|| bytes)))
    }

    #[cfg(test)]
    pub(crate) fn is_loaded(&self) -> bool {
        self.bytes.get().is_some()
    }
}

static VENDOR_ARTIFACTS: [VendorArtifact; 17] = [
    VendorArtifact {
        filename: "calver-2022.6.26-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/f7/39/e421c06f42ca00fa9cf8929c2466e58a837e8e97b8ab3ff4f4ff9a15e33e/calver-2022.6.26-py3-none-any.whl",
        local_path: None,
        sha256: "a1d7fcdd67797afc52ee36ffb8c8adf6643173864306547bfd1380cbce6310a0",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "editables-0.5-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/6b/be/0f2f4a5e8adc114a02b63d92bf8edbfa24db6fc602fca83c885af2479e0e/editables-0.5-py3-none-any.whl",
        local_path: None,
        sha256: "61e5ffa82629e0d8bfe09bc44a07db3c1ab8ed1ce78a6980732870f19b5e7d4c",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "flit_core-3.9.0-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/38/45/618e84e49a6c51e5dd15565ec2fcd82ab273434f236b8f108f065ded517a/flit_core-3.9.0-py3-none-any.whl",
        local_path: None,
        sha256: "7aada352fb0c7f5538c4fafeddf314d3a6a92ee8e2b1de70482329e42de70301",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "futzed_bz2-0.1.0-py3-none-any.whl",
        url: "https://github.com/astral-sh/futzed-wheels/releases/download/v2026.02.09.2/futzed_bz2-0.1.0-py3-none-any.whl",
        local_path: None,
        sha256: "3c106ea0433a90f767271c322a8cb8bf5145f210adf8a357152796475b00b316",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "futzed_lzma-0.1.0-py3-none-any.whl",
        url: "https://github.com/astral-sh/futzed-wheels/releases/download/v2026.02.09.2/futzed_lzma-0.1.0-py3-none-any.whl",
        local_path: None,
        sha256: "9f39b4686926ffbed723450d700cca524001af259949eb5e4b973e7846df23d2",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "hatchling-1.20.0-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/87/4a/7d22a92b55809c579d8deafb0dabeb102b411a0ef9439949cccef3071527/hatchling-1.20.0-py3-none-any.whl",
        local_path: None,
        sha256: "872c63aa7e8aca85e8dba07b05c6a9b28d5a149fe00638f1a47e36930197248f",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "packaging-23.2-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/ec/1a/610693ac4ee14fcdf2d9bf3c493370e4f2ef7ae2e19217d7a237ff42367d/packaging-23.2-py3-none-any.whl",
        local_path: None,
        sha256: "8c491190033a9af7e1d931d0b5dacc2ef47509b34dd0de67ed209b5203fc88c7",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "packaging-23.2.tar.gz",
        url: "https://files.pythonhosted.org/packages/fb/2b/9b9c33ffed44ee921d0967086d653047286054117d584f1b1a7c22ceaf7b/packaging-23.2.tar.gz",
        local_path: None,
        sha256: "048fb0e9405036518eaaf48a55953c750c11e1a1b68e0dd1a9d62ed0c092cfc5",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "pathspec-0.12.1-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/cc/20/ff623b09d963f88bfde16306a54e12ee5ea43e9b597108672ff3a408aad6/pathspec-0.12.1-py3-none-any.whl",
        local_path: None,
        sha256: "a0d503e138a4c123b27490a4f7beda6a01c6f288df0e4a8b79c7eb0dc7b4cc08",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "pip-24.0-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/8a/6a/19e9fe04fca059ccf770861c7d5721ab4c2aebc539889e97c7977528a53b/pip-24.0-py3-none-any.whl",
        local_path: None,
        sha256: "ba0d021a166865d2265246961bec0152ff124de910c5cc39f1156ce3fa7c69dc",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "pluggy-1.3.0-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/05/b8/42ed91898d4784546c5f06c60506400548db3f7a4b3fb441cba4e5c17952/pluggy-1.3.0-py3-none-any.whl",
        local_path: None,
        sha256: "d89c696a773f8bd377d18e5ecda92b7a3793cbe66c87060a6fb58c7b6e1061f7",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "setuptools-69.0.2-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/bb/e1/ed2dd0850446b8697ad28d118df885ad04140c64ace06c4bd559f7c8a94f/setuptools-69.0.2-py3-none-any.whl",
        local_path: None,
        sha256: "1e8fdff6797d3865f37397be788a4e3cba233608e9b509382a2777d25ebde7f2",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "setuptools_scm-8.0.4-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/0e/a3/b9a8b0adfe672bf0df5901707aa929d30a97ee390ba651910186776746d2/setuptools_scm-8.0.4-py3-none-any.whl",
        local_path: None,
        sha256: "b47844cd2a84b83b3187a5782c71128c28b4c94cad8bfb871da2784a5cb54c4f",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "tomli-2.0.1-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/97/75/10a9ebee3fd790d20926a90a2547f0bf78f371b2f13aa822c759680ca7b9/tomli-2.0.1-py3-none-any.whl",
        local_path: None,
        sha256: "939de3e7a6161af0c887ef91b7d41a53e7c5a1ca976325f429cb46ea9bc30ecc",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "trove_classifiers-2023.11.29-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/f8/50/e223fe762fe21fefb7a3f37c10e9693ea5f63cb54b5ae39daa876b780abc/trove_classifiers-2023.11.29-py3-none-any.whl",
        local_path: None,
        sha256: "02307750cbbac2b3d13078662f8a5bf077732bf506e9c33c97204b7f68f3699e",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "typing_extensions-4.9.0-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/b7/f4/6a90020cd2d93349b442bfcb657d0dc91eee65491600b2cb1d388bc98e6b/typing_extensions-4.9.0-py3-none-any.whl",
        local_path: None,
        sha256: "af72aea155e91adfc61c3ae9e0e342dbc0cba726d6cba4b6c72c1f34e47291cd",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "wheel-0.42.0-py3-none-any.whl",
        url: "https://files.pythonhosted.org/packages/c7/c3/55076fc728723ef927521abaa1955213d094933dc36d4a2008d5101e1af5/wheel-0.42.0-py3-none-any.whl",
        local_path: None,
        sha256: "177f9c9b0d45c47873b619f5b650346d632cdc35fb5e4d25058e09c9e581433d",
        upload_time: None,
        bytes: OnceLock::new(),
    },
];

static LOCAL_VENDOR_ARTIFACTS: [VendorArtifact; 38] = [
    VendorArtifact {
        filename: "poetry_core-2.1.2-py3-none-any.whl",
        url: "",
        local_path: Some("poetry_core-2.1.2-py3-none-any.whl"),
        sha256: "ecb1e8f7d4f071a21cd0feb8c19bd1aec80de6fb0e82aa9d809a591e544431b4",
        upload_time: Some("2025-03-29T20:38:15.256960Z"),
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "poetry_core-2.1.3-py3-none-any.whl",
        url: "",
        local_path: Some("poetry_core-2.1.3-py3-none-any.whl"),
        sha256: "2c704f05016698a54ca1d327f46ce2426d72eaca6ff614132c8477c292266771",
        upload_time: Some("2025-05-04T12:43:09.814666Z"),
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "hatchling-1.22.4-py3-none-any.whl",
        url: "",
        local_path: Some("hatchling-1.22.4-py3-none-any.whl"),
        sha256: "f56da5bfc396af7b29daa3164851dd04991c994083f56cb054b5003675caecdc",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "trove_classifiers-2024.3.3-py3-none-any.whl",
        url: "",
        local_path: Some("trove_classifiers-2024.3.3-py3-none-any.whl"),
        sha256: "3a84096861b385ec422c79995d1f6435dde47a9b63adaa3c886e53232ba7e6e0",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "Flask-1.0.4.tar.gz",
        url: "",
        local_path: Some("Flask-1.0.4.tar.gz"),
        sha256: "ed1330220a321138de53ec7c534c3d90cf2f7af938c7880fc3da13aa46bf870f",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "calver-2022.6.26-py3-none-any.whl",
        url: "",
        local_path: Some("calver-2022.6.26-py3-none-any.whl"),
        sha256: "a1d7fcdd67797afc52ee36ffb8c8adf6643173864306547bfd1380cbce6310a0",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "cbwheeldiff2-0.0.1-py2.py3-none-any.whl",
        url: "",
        local_path: Some("cbwheeldiff2-0.0.1-py2.py3-none-any.whl"),
        sha256: "d02fc1ef06f41aa1f1624b4a7ffb318d3f01c80734085e0697e75eec2d47b7f1",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "cbwheelstreamtest-0.0.1-py2.py3-none-any.whl",
        url: "",
        local_path: Some("cbwheelstreamtest-0.0.1-py2.py3-none-any.whl"),
        sha256: "e8eb04a5a33b6348f90b3e990026bda7ff35e8c8fef59fe329c7ae59eeb86af4",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "cbwheelziptest-0.0.2-py2.py3-none-any.whl",
        url: "",
        local_path: Some("cbwheelziptest-0.0.2-py2.py3-none-any.whl"),
        sha256: "b43c064c0e217b2386fe9d3757e8ea6294315d810b85f76060a0d9454584195e",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "editables-0.5-py3-none-any.whl",
        url: "",
        local_path: Some("editables-0.5-py3-none-any.whl"),
        sha256: "61e5ffa82629e0d8bfe09bc44a07db3c1ab8ed1ce78a6980732870f19b5e7d4c",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "flit_core-3.9.0-py3-none-any.whl",
        url: "",
        local_path: Some("flit_core-3.9.0-py3-none-any.whl"),
        sha256: "7aada352fb0c7f5538c4fafeddf314d3a6a92ee8e2b1de70482329e42de70301",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "flit_core-3.9.0.tar.gz",
        url: "",
        local_path: Some("flit_core-3.9.0.tar.gz"),
        sha256: "72ad266176c4a3fcfab5f2930d76896059851240570ce9a98733b658cb786eba",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "hatch_vcs-0.4.0-py3-none-any.whl",
        url: "",
        local_path: Some("hatch_vcs-0.4.0-py3-none-any.whl"),
        sha256: "b8a2b6bee54cf6f9fc93762db73890017ae59c9081d1038a41f16235ceaf8b2c",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "hatchling-1.20.0-py3-none-any.whl",
        url: "",
        local_path: Some("hatchling-1.20.0-py3-none-any.whl"),
        sha256: "872c63aa7e8aca85e8dba07b05c6a9b28d5a149fe00638f1a47e36930197248f",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "hatchling-1.20.0.tar.gz",
        url: "",
        local_path: Some("hatchling-1.20.0.tar.gz"),
        sha256: "0e0893cbe3d5f9275fc0e5b629087fc23b17abd7065e4db0a310e0a0237bc945",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "install-1.3.5-py3-none-any.whl",
        url: "",
        local_path: Some("install-1.3.5-py3-none-any.whl"),
        sha256: "0d3fadf4aa62c95efe8d34757c8507eb46177f86c016c21c6551eafc6a53d5a9",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "packaging-23.2-py3-none-any.whl",
        url: "",
        local_path: Some("packaging-23.2-py3-none-any.whl"),
        sha256: "8c491190033a9af7e1d931d0b5dacc2ef47509b34dd0de67ed209b5203fc88c7",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "packaging-23.2.tar.gz",
        url: "",
        local_path: Some("packaging-23.2.tar.gz"),
        sha256: "048fb0e9405036518eaaf48a55953c750c11e1a1b68e0dd1a9d62ed0c092cfc5",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "packaging-24.0-py3-none-any.whl",
        url: "",
        local_path: Some("packaging-24.0-py3-none-any.whl"),
        sha256: "2ddfb553fdf02fb784c234c7ba6ccc288296ceabec964ad2eae3777778130bc5",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "packaging-24.0.tar.gz",
        url: "",
        local_path: Some("packaging-24.0.tar.gz"),
        sha256: "eb82c5e3e56209074766e6885bb04b8c38a0c015d0a30036ebe7ece34c9989e9",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "pathspec-0.12.1-py3-none-any.whl",
        url: "",
        local_path: Some("pathspec-0.12.1-py3-none-any.whl"),
        sha256: "a0d503e138a4c123b27490a4f7beda6a01c6f288df0e4a8b79c7eb0dc7b4cc08",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "pip-24.0-py3-none-any.whl",
        url: "",
        local_path: Some("pip-24.0-py3-none-any.whl"),
        sha256: "ba0d021a166865d2265246961bec0152ff124de910c5cc39f1156ce3fa7c69dc",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "pip-24.1b1-py3-none-any.whl",
        url: "",
        local_path: Some("pip-24.1b1-py3-none-any.whl"),
        sha256: "752516cffafef5cf29d3fb2f06c978f27e98709654689cf277b2fce984c7a591",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "pluggy-1.3.0-py3-none-any.whl",
        url: "",
        local_path: Some("pluggy-1.3.0-py3-none-any.whl"),
        sha256: "d89c696a773f8bd377d18e5ecda92b7a3793cbe66c87060a6fb58c7b6e1061f7",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "pluggy-1.4.0-py3-none-any.whl",
        url: "",
        local_path: Some("pluggy-1.4.0-py3-none-any.whl"),
        sha256: "7db9f7b503d67d1c5b95f59773ebb58a8c1c288129a88665838012cfb07b8981",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "poetry-1.8.2-py3-none-any.whl",
        url: "",
        local_path: Some("poetry-1.8.2-py3-none-any.whl"),
        sha256: "b42b400d9a803af6e788a30a6f3e9998020b77860e28df20647eb10b6f414910",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "poetry_core-1.9.0-py3-none-any.whl",
        url: "",
        local_path: Some("poetry_core-1.9.0-py3-none-any.whl"),
        sha256: "4e0c9c6ad8cf89956f03b308736d84ea6ddb44089d16f2adc94050108ec1f5a1",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "pyo3_mixed-2.1.5.tar.gz",
        url: "",
        local_path: Some("pyo3_mixed-2.1.5.tar.gz"),
        sha256: "95e23fb0aa330b9a7dcc8029d3cc35c3364a8beb97cdb32aa66128d305d822c1",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "setuptools-69.0.2-py3-none-any.whl",
        url: "",
        local_path: Some("setuptools-69.0.2-py3-none-any.whl"),
        sha256: "1e8fdff6797d3865f37397be788a4e3cba233608e9b509382a2777d25ebde7f2",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "setuptools-69.2.0-py3-none-any.whl",
        url: "",
        local_path: Some("setuptools-69.2.0-py3-none-any.whl"),
        sha256: "c21c49fb1042386df081cb5d86759792ab89efca84cf114889191cd09aacc80c",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "setuptools_scm-8.0.4-py3-none-any.whl",
        url: "",
        local_path: Some("setuptools_scm-8.0.4-py3-none-any.whl"),
        sha256: "b47844cd2a84b83b3187a5782c71128c28b4c94cad8bfb871da2784a5cb54c4f",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "sklearn-0.0.post12.tar.gz",
        url: "",
        local_path: Some("sklearn-0.0.post12.tar.gz"),
        sha256: "54cff9e20839b7b202321178228af4d9388bedf78425d9299fd9ee170d68802e",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "suds_community-0.8.5-py3-none-any.whl",
        url: "",
        local_path: Some("suds_community-0.8.5-py3-none-any.whl"),
        sha256: "cdb89667c4532d104586bf77fb5fc3659ad3fe3307b08b43c8388d8d363ae8c9",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "tomli-2.0.1-py3-none-any.whl",
        url: "",
        local_path: Some("tomli-2.0.1-py3-none-any.whl"),
        sha256: "939de3e7a6161af0c887ef91b7d41a53e7c5a1ca976325f429cb46ea9bc30ecc",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "trove_classifiers-2023.11.29-py3-none-any.whl",
        url: "",
        local_path: Some("trove_classifiers-2023.11.29-py3-none-any.whl"),
        sha256: "02307750cbbac2b3d13078662f8a5bf077732bf506e9c33c97204b7f68f3699e",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "typing_extensions-4.9.0-py3-none-any.whl",
        url: "",
        local_path: Some("typing_extensions-4.9.0-py3-none-any.whl"),
        sha256: "af72aea155e91adfc61c3ae9e0e342dbc0cba726d6cba4b6c72c1f34e47291cd",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "wheel-0.42.0-py3-none-any.whl",
        url: "",
        local_path: Some("wheel-0.42.0-py3-none-any.whl"),
        sha256: "177f9c9b0d45c47873b619f5b650346d632cdc35fb5e4d25058e09c9e581433d",
        upload_time: None,
        bytes: OnceLock::new(),
    },
    VendorArtifact {
        filename: "wsgiref-0.1.2.zip",
        url: "",
        local_path: Some("wsgiref-0.1.2.zip"),
        sha256: "c7e610c800957046c04c8014aab8cce8f0b9f0495c8cd349e57c1f7cabf40e79",
        upload_time: None,
        bytes: OnceLock::new(),
    },
];

pub(crate) fn vendor_artifacts() -> impl Iterator<Item = &'static VendorArtifact> {
    VENDOR_ARTIFACTS
        .iter()
        .filter(|remote| {
            !LOCAL_VENDOR_ARTIFACTS
                .iter()
                .any(|local| local.filename == remote.filename)
        })
        .chain(LOCAL_VENDOR_ARTIFACTS.iter())
}

fn vendor_cache_dir() -> PathBuf {
    TestContext::test_bucket_dir().join("vendor")
}

fn load_vendor_file(artifact: &VendorArtifact) -> Result<Arc<[u8]>> {
    if let Some(relative_path) = artifact.local_path {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test/vendor")
            .join(relative_path);
        let bytes = fs_err::read(&path)
            .with_context(|| format!("failed to read vendor artifact `{}`", path.display()))?;
        verify_bytes(artifact, &bytes)?;
        return Ok(bytes.into());
    }

    // HTTP responders run in a Tokio runtime; load on another thread instead of nesting runtimes.
    let bytes = std::thread::scope(|scope| {
        scope
            .spawn(|| load_vendor_file_async(artifact))
            .join()
            .map_err(|_| anyhow!("vendor artifact loader panicked"))
    })??;

    Ok(bytes.into())
}

#[tokio::main(flavor = "current_thread")]
async fn load_vendor_file_async(artifact: &VendorArtifact) -> Result<Vec<u8>> {
    let cache_dir = vendor_cache_dir();
    fs_err::create_dir_all(&cache_dir)
        .with_context(|| format!("failed to create vendor cache at `{}`", cache_dir.display()))?;
    let path = cache_dir.join(artifact.filename);
    ensure_cached_artifact(artifact, &path).await?;
    let bytes = fs_err::read(&path)
        .with_context(|| format!("failed to read cached vendor artifact `{}`", path.display()))?;
    verify_bytes(artifact, &bytes)?;
    Ok(bytes)
}

async fn ensure_cached_artifact(artifact: &VendorArtifact, path: &Path) -> Result<()> {
    if cached_artifact_matches(artifact, path) {
        return Ok(());
    }

    let lock_path = artifact_lock_path(path)?;
    let _lock = LockedFile::acquire(
        &lock_path,
        LockedFileMode::Exclusive,
        format_args!("vendor artifact `{}`", artifact.filename),
    )
    .await
    .with_context(|| {
        format!(
            "failed to lock cached vendor artifact `{}`",
            artifact.filename
        )
    })?;

    if cached_artifact_matches(artifact, path) {
        return Ok(());
    }

    let trusted_hosts = std::env::var(EnvVars::UV_INSECURE_HOST)
        .unwrap_or_default()
        .split(' ')
        .filter(|host| !host.is_empty())
        .map(TrustedHost::from_str)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let client = uv_client::BaseClientBuilder::default()
        .allow_insecure_host(trusted_hosts)
        .deny_pypi_for_tests(true)
        .build()
        .context("failed to build vendor artifact client")?;

    let url = artifact
        .url
        .parse()
        .with_context(|| format!("invalid vendor artifact URL `{}`", artifact.url))?;
    let response = client
        .for_host(&url)
        .get(reqwest::Url::from(url))
        .send()
        .await
        .with_context(|| format!("failed to download `{}`", artifact.url))?
        .error_for_status()
        .with_context(|| format!("failed to download `{}`", artifact.url))?;
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("failed to read `{}`", artifact.url))?;
    verify_bytes(artifact, &bytes)?;

    let parent = path
        .parent()
        .context("vendor artifact cache path should have a parent")?;
    let mut temp = tempfile_in(parent).with_context(|| {
        format!(
            "failed to create vendor temp file in `{}`",
            parent.display()
        )
    })?;
    temp.write_all(&bytes)
        .with_context(|| format!("failed to write `{}`", artifact.filename))?;
    temp.as_file()
        .sync_all()
        .with_context(|| format!("failed to sync `{}`", artifact.filename))?;

    match persist_with_retry_sync(temp, path) {
        Ok(()) => Ok(()),
        Err(_)
            if let Ok(bytes) = fs_err::read(path)
                && verify_bytes(artifact, &bytes).is_ok() =>
        {
            Ok(())
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to persist cached vendor artifact `{}`",
                path.display()
            )
        }),
    }
}

fn cached_artifact_matches(artifact: &VendorArtifact, path: &Path) -> bool {
    fs_err::read(path).is_ok_and(|bytes| verify_bytes(artifact, &bytes).is_ok())
}

fn artifact_lock_path(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .context("vendor artifact cache path should have a parent")?;
    let mut filename = path
        .file_name()
        .context("vendor artifact cache path should have a filename")?
        .to_os_string();
    filename.push(".lock");
    Ok(parent.join(filename))
}

fn verify_bytes(artifact: &VendorArtifact, bytes: &[u8]) -> Result<()> {
    let actual = hex::encode(Sha256::digest(bytes));
    if actual == artifact.sha256 {
        Ok(())
    } else {
        bail!(
            "hash mismatch for cached vendor artifact `{}`: expected `{}`, found `{}`",
            artifact.filename,
            artifact.sha256,
            actual
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pypi_artifacts_are_local_and_pinned() -> Result<()> {
        for artifact in vendor_artifacts() {
            if let Some(relative_path) = artifact.local_path {
                let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../test/vendor")
                    .join(relative_path);
                verify_bytes(artifact, &fs_err::read(path)?)?;
            } else {
                assert!(artifact.url.starts_with("https://github.com/"));
            }
        }
        Ok(())
    }

    #[test]
    fn artifact_lock_path_is_per_artifact() {
        let path = Path::new("vendor").join("example-1.0.0-py3-none-any.whl");

        assert_eq!(
            artifact_lock_path(&path).expect("path should have a parent"),
            Path::new("vendor").join("example-1.0.0-py3-none-any.whl.lock")
        );
    }

    #[test]
    fn cached_bytes_are_held_per_artifact() {
        let bytes = OnceLock::new();
        bytes
            .set(Arc::from(b"available".as_slice()))
            .expect("artifact should not already have cached bytes");
        let loaded = VendorArtifact {
            filename: "available.whl",
            url: "unused",
            local_path: None,
            sha256: "unused",
            upload_time: None,
            bytes,
        };
        let unloaded = VendorArtifact {
            filename: "unrequested.whl",
            url: "unused",
            local_path: None,
            sha256: "unused",
            upload_time: None,
            bytes: OnceLock::new(),
        };

        assert_eq!(
            loaded
                .bytes()
                .expect("preloaded bytes should be returned")
                .as_ref(),
            b"available"
        );
        assert!(unloaded.bytes.get().is_none());
    }
}
