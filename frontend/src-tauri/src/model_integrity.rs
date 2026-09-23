//! Checks that a downloaded model is exactly the file it was pinned to.
//!
//! Models are parsed by native code — ggml, ONNX Runtime, llama.cpp — whose
//! file parsers have had memory-safety bugs. A model fetched from a mutable
//! `resolve/main` URL with nothing checked is therefore code from whoever
//! controls that repository today. Every URL below names a commit, and every
//! file its SHA-256, taken from the Hugging Face API for that commit (the LFS
//! object id is the SHA-256 of the content; small files were hashed and their
//! git blob ids matched against the tree).
//!
//! The hash is computed while the file streams in, so a multi-gigabyte model
//! is not read a second time.

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

/// What a file must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pinned {
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

const fn pin(file: &'static str, size: u64, sha256: &'static str) -> Pinned {
    Pinned { file, size, sha256 }
}

// --- Whisper (ggerganov/whisper.cpp) ---------------------------------------

pub const WHISPER_BASE: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1";

pub const WHISPER: &[Pinned] = &[
    pin("ggml-tiny.bin", 77_691_713, "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21"),
    pin("ggml-base.bin", 147_951_465, "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"),
    pin("ggml-small.bin", 487_601_967, "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"),
    pin("ggml-medium.bin", 1_533_763_059, "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208"),
    pin("ggml-large-v3-turbo.bin", 1_624_555_275, "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69"),
    pin("ggml-large-v3.bin", 3_095_033_483, "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2"),
    pin("ggml-tiny-q5_1.bin", 32_152_673, "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7"),
    pin("ggml-base-q5_1.bin", 59_707_625, "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898"),
    pin("ggml-small-q5_1.bin", 190_085_487, "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb"),
    pin("ggml-medium-q5_0.bin", 539_212_467, "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f"),
    pin("ggml-large-v3-turbo-q5_0.bin", 574_041_195, "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"),
    pin("ggml-large-v3-q5_0.bin", 1_081_140_203, "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1"),
];

// --- Parakeet (istupakov/parakeet-tdt-0.6b-*-onnx) --------------------------

pub const PARAKEET_V2_BASE: &str =
    "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/0bbb45a3365852604aef28b538a8f066f4ccaa85";
pub const PARAKEET_V3_BASE: &str =
    "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce";

pub const PARAKEET_V2: &[Pinned] = &[
    pin("encoder-model.int8.onnx", 652_184_014, "3e0581fda6ab843888b51e56d7ee78b6d5bc3237ec113af1f732d1d5286aa155"),
    pin("decoder_joint-model.int8.onnx", 8_998_286, "a449f49acd68979d418651dd2dcb737cc0f1bf0225e009e29ee326354edbf7d3"),
    pin("nemo128.onnx", 139_764, "a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f"),
    pin("vocab.txt", 9_384, "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d"),
    pin("encoder-model.onnx", 41_770_866, "3987bcd28175d829d12888a996a84e8f62a0e374d9ffd640662c1515adc679d3"),
    pin("encoder-model.onnx.data", 2_435_420_160, "4dab7362d4874d85965045b1e41b2d61dd2cc0fb25671a7f6b3dc47bf120cc41"),
    pin("decoder_joint-model.onnx", 35_792_059, "cbb52a07bd70ab5b67f8439d4b3cd8704b18467b4430bcacb5adabe154b8d191"),
];

pub const PARAKEET_V3: &[Pinned] = &[
    pin("encoder-model.int8.onnx", 652_183_999, "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09"),
    pin("decoder_joint-model.int8.onnx", 18_202_004, "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70"),
    pin("nemo128.onnx", 139_764, "a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f"),
    pin("vocab.txt", 93_939, "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d"),
    pin("encoder-model.onnx", 41_770_866, "98a74b21b4cc0017c1e7030319a4a96f4a9506e50f0708f3a516d02a77c96bb1"),
    pin("encoder-model.onnx.data", 2_435_420_160, "9a22d372c51455c34f13405da2520baefb7125bd16981397561423ed32d24f36"),
    pin("decoder_joint-model.onnx", 72_520_893, "e978ddf6688527182c10fde2eb4b83068421648985ef23f7a86be732be8706c1"),
];

// --- GigaAM (istupakov/gigaam-v3-onnx) --------------------------------------

pub const GIGAAM_BASE: &str =
    "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/322c3b29492673eb7d0b434bfa9dfb8653e34d02";

pub const GIGAAM: &[Pinned] = &[
    pin("v3_e2e_rnnt_encoder.int8.onnx", 224_570_477, "4e0e076a6076cd110277e529b8ac8f32cd5297f7fbebad5341ae8ddb7d00817b"),
    pin("v3_e2e_rnnt_decoder.int8.onnx", 1_159_170, "89014e134865615b91e037157e46e389b1271e6072460efc010ea08e61e23146"),
    pin("v3_e2e_rnnt_joint.int8.onnx", 687_791, "ade116563dbf66e503b0994efab6b5861412743e52bf31c39fc3fffa3783d5d1"),
    pin("v3_e2e_rnnt_vocab.txt", 13_354, "39abae20e692998290c574e606f11a9edef2902a1995463fcff63d1490cf22b7"),
];

// --- Summary models (GGUF) --------------------------------------------------

/// Pinned URL and content of each built-in summary model, by file name.
pub const SUMMARY: &[(&str, Pinned)] = &[
    (
        "https://huggingface.co/unsloth/Qwen3.5-2B-GGUF/resolve/f6d5376be1edb4d416d56da11e5397a961aca8ae/Qwen3.5-2B-Q4_K_M.gguf",
        pin("Qwen3.5-2B-Q4_K_M.gguf", 1_280_835_840, "aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223"),
    ),
    (
        "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/e87f176479d0855a907a41277aca2f8ee7a09523/Qwen3.5-4B-Q4_K_M.gguf",
        pin("Qwen3.5-4B-Q4_K_M.gguf", 2_740_937_888, "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4"),
    ),
    (
        "https://huggingface.co/bartowski/google_gemma-3-4b-it-GGUF/resolve/71506238f970075ca85125cd749c28b1b0eee84e/google_gemma-3-4b-it-Q4_K_M.gguf",
        pin("google_gemma-3-4b-it-Q4_K_M.gguf", 2_489_758_112, "4996030242583a40aa151ff93f49ed787ac8c25e4120c3ae4588b2e2a7d1ae94"),
    ),
    (
        "https://huggingface.co/bartowski/google_gemma-3-1b-it-GGUF/resolve/116f76234503685a98f572982177b11d44ec8ff1/google_gemma-3-1b-it-Q8_0.gguf",
        pin("google_gemma-3-1b-it-Q8_0.gguf", 1_069_306_624, "375e12a4a18929a641f9744b060d4a7cf4e279530750555828ec0c117870bc96"),
    ),
];

/// The pin for `file` in `table`.
pub fn find(table: &'static [Pinned], file: &str) -> Option<&'static Pinned> {
    table.iter().find(|pinned| pinned.file == file)
}

/// The pinned URL and content of a built-in summary model.
pub fn summary_model(file: &str) -> Option<(&'static str, &'static Pinned)> {
    SUMMARY
        .iter()
        .find(|(_, pinned)| pinned.file == file)
        .map(|(url, pinned)| (*url, pinned))
}

/// A SHA-256 fed as the bytes arrive.
#[derive(Default)]
pub struct Check {
    hasher: Sha256,
    bytes: u64,
}

impl Check {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, chunk: &[u8]) {
        self.hasher.update(chunk);
        self.bytes += chunk.len() as u64;
    }

    /// Compares what arrived with the pin. On a mismatch `path` is deleted, so
    /// a file that is not the model never sits where the model is looked for.
    pub fn finish(self, pinned: &Pinned, path: &Path) -> Result<()> {
        let actual = format!("{:x}", self.hasher.finalize());
        if self.bytes == pinned.size && actual == pinned.sha256 {
            return Ok(());
        }
        let _ = std::fs::remove_file(path);
        Err(anyhow!(
            "{} failed its integrity check ({} bytes, sha256 {}; expected {} bytes, sha256 {}) and was deleted",
            pinned.file,
            self.bytes,
            actual,
            pinned.size,
            pinned.sha256
        ))
    }
}

/// Hashes a file already on disk against its pin, off the async runtime.
pub async fn verify_file(pinned: &'static Pinned, path: &Path) -> Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(&path)?;
        let mut check = Check::new();
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            check.update(&buffer[..read]);
        }
        check.finish(pinned, &path)
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pin_is_a_well_formed_sha256() {
        let all = WHISPER
            .iter()
            .chain(PARAKEET_V2)
            .chain(PARAKEET_V3)
            .chain(GIGAAM)
            .chain(SUMMARY.iter().map(|(_, pinned)| pinned));
        for pinned in all {
            assert_eq!(pinned.sha256.len(), 64, "{}", pinned.file);
            assert!(pinned.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
            assert!(pinned.size > 0);
        }
        for (url, pinned) in SUMMARY {
            assert!(url.ends_with(pinned.file));
            assert!(!url.contains("/resolve/main/"));
        }
    }

    #[test]
    fn a_matching_file_passes_and_a_different_one_is_deleted() {
        const HELLO: Pinned = Pinned {
            file: "hello.txt",
            size: 5,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, b"hello").unwrap();

        let mut good = Check::new();
        good.update(b"hel");
        good.update(b"lo");
        good.finish(&HELLO, &path).unwrap();
        assert!(path.exists());

        let mut bad = Check::new();
        bad.update(b"hellO");
        let error = bad.finish(&HELLO, &path).unwrap_err();
        assert!(error.to_string().contains("integrity check"));
        assert!(!path.exists());
    }
}
