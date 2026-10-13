// ============================================================================
// FFmpeg Binary Bundling
// ============================================================================
// Download and bundle FFmpeg binaries at build-time to eliminate runtime download delays
//
// The bundled FFmpeg ships inside the installer and processes decrypted audio,
// so every archive and every extracted binary is pinned by SHA-256. Nothing is
// extracted or executed before its hash matches; a mismatch fails the build.
//
// FFmpeg is GPL. Its license and the build's own description (version, source
// commit, configuration, the libraries linked in) come out of the same pinned
// archive into `licenses/ffmpeg/`, which the installer carries next to the
// rest of the third-party notices.

use sha2::{Digest, Sha256};
use std::io::Read;

/// A pinned FFmpeg download for one target.
struct FfmpegSource {
    url: &'static str,
    /// SHA-256 of the archive at `url`, as published for the release asset.
    archive_sha256: &'static str,
    /// SHA-256 of the ffmpeg executable inside the archive.
    binary_sha256: &'static str,
    /// License and build notes shipped with the binary.
    notices: &'static [Notice],
}

/// A text file from the archive that goes into the installer as it is.
struct Notice {
    /// File name in the archive's top folder.
    in_archive: &'static str,
    /// File name under `licenses/ffmpeg/`.
    install_as: &'static str,
    sha256: &'static str,
}

/// Where the notices are written; `tauri.conf.json` bundles this folder.
fn notices_dir() -> std::path::PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR environment variable not set");
    std::path::PathBuf::from(manifest_dir).join("licenses").join("ffmpeg")
}

/// Whether every notice is already in place with its pinned content.
fn notices_present(source: &FfmpegSource) -> bool {
    let dir = notices_dir();
    source
        .notices
        .iter()
        .all(|notice| sha256_file(&dir.join(notice.install_as)).as_deref() == Ok(notice.sha256))
}

/// Download and bundle FFmpeg binary for current target platform
/// Checks cache first, downloads only if missing or not matching the pinned hash
pub fn ensure_ffmpeg_binary() {
    let target = std::env::var("TARGET")
        .or_else(|_| std::env::var("HOST"))
        .expect("Neither TARGET nor HOST environment variable set");

    println!("cargo:warning=🎬 Checking FFmpeg binary for target: {}", target);

    let binary_name = if target.contains("windows") {
        format!("ffmpeg-{}.exe", target)
    } else {
        format!("ffmpeg-{}", target)
    };

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR environment variable not set");
    let binaries_dir = std::path::PathBuf::from(&manifest_dir).join("binaries");
    let binary_path = binaries_dir.join(&binary_name);

    let source = get_ffmpeg_source_for_target(&target)
        .unwrap_or_else(|e| panic!("⚠️  {}", e));

    // Cache check: the binary is hashed before it is ever run
    if binary_path.exists() {
        println!("cargo:warning=🔍 Found cached FFmpeg binary: {}", binary_name);
        match sha256_file(&binary_path) {
            Ok(actual) if actual == source.binary_sha256 => {
                println!("cargo:warning=🔒 FFmpeg binary SHA-256 matches pinned value: {}", actual);
                if !notices_present(&source) {
                    // The binary is fine and stays until a verified one replaces it
                    println!("cargo:warning=📄 FFmpeg license files missing, downloading the archive for them...");
                    if let Err(e) = download_and_extract_ffmpeg(&target, &source, &binary_path) {
                        panic!("⚠️  Failed to download FFmpeg: {}", e);
                    }
                    if !verify_ffmpeg_binary(&binary_path) {
                        panic!("⚠️  Downloaded FFmpeg binary verification failed!");
                    }
                    return;
                } else if verify_ffmpeg_binary(&binary_path) {
                    println!("cargo:warning=✅ FFmpeg binary already cached and verified: {}", binary_name);
                    return;
                } else {
                    println!("cargo:warning=⚠️  Cached FFmpeg binary failed to run, re-downloading...");
                }
            }
            Ok(actual) => {
                println!(
                    "cargo:warning=⚠️  Cached FFmpeg binary SHA-256 mismatch (expected {}, got {}), deleting without running it and re-downloading...",
                    source.binary_sha256, actual
                );
            }
            Err(e) => {
                println!("cargo:warning=⚠️  Cannot hash cached FFmpeg binary ({}), re-downloading...", e);
            }
        }
        let _ = std::fs::remove_file(&binary_path);
    }

    println!("cargo:warning=📥 Downloading FFmpeg for {}", target);

    // Create binaries directory if it doesn't exist
    if !binaries_dir.exists() {
        std::fs::create_dir_all(&binaries_dir)
            .expect("Failed to create binaries directory");
    }

    // Download and extract
    match download_and_extract_ffmpeg(&target, &source, &binary_path) {
        Ok(()) => {
            println!("cargo:warning=✅ FFmpeg binary downloaded successfully: {}", binary_name);

            // Hash already checked in write_verified; now it is safe to run
            if !verify_ffmpeg_binary(&binary_path) {
                panic!("⚠️  Downloaded FFmpeg binary verification failed!");
            }
        }
        Err(e) => {
            panic!("⚠️  Failed to download FFmpeg: {}", e);
        }
    }
}

/// Download the pinned archive and take the binary and the notices out of it,
/// in memory: nothing from the archive reaches the disk before its hash matches.
fn download_and_extract_ffmpeg(
    target: &str,
    source: &FfmpegSource,
    output_path: &std::path::PathBuf,
) -> Result<(), String> {
    let url = source.url;

    println!("cargo:warning=⬇️  Downloading from: {}", url);

    // Download with timeout (using reqwest from build-dependencies)
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600)) // 10 min timeout for large downloads
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Failed to download: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);
    println!("cargo:warning=📦 Download size: {:.1} MB", total_size as f64 / 1_048_576.0);

    let content = response.bytes()
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Verify before anything is extracted
    let actual = sha256_hex(&content);
    if actual != source.archive_sha256 {
        return Err(format!(
            "FFmpeg archive SHA-256 mismatch for {}: expected {}, got {}. Refusing to extract.",
            url, source.archive_sha256, actual
        ));
    }
    println!("cargo:warning=🔒 FFmpeg archive SHA-256 matches pinned value: {}", actual);
    println!("cargo:warning=📂 Extracting FFmpeg binary and license files...");

    let executable_name = if target.contains("windows") { "ffmpeg.exe" } else { "ffmpeg" };
    let mut wanted: Vec<&str> = vec![executable_name];
    wanted.extend(source.notices.iter().map(|notice| notice.in_archive));
    let found = if url.ends_with(".zip") {
        files_from_zip(&content, &wanted)?
    } else if url.ends_with(".tar.xz") || url.ends_with(".txz") {
        files_from_tar_xz(&content, &wanted)?
    } else {
        return Err(format!("Unsupported archive format: {}", url));
    };

    let take = |name: &str| {
        found
            .iter()
            .find(|(found_name, _)| found_name == name)
            .map(|(_, bytes)| bytes.as_slice())
            .ok_or_else(|| format!("'{}' not found in the FFmpeg archive", name))
    };

    write_verified(take(executable_name)?, source.binary_sha256, output_path)?;
    let dir = notices_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create {:?}: {}", dir, e))?;
    for notice in source.notices {
        write_verified(take(notice.in_archive)?, notice.sha256, &dir.join(notice.install_as))?;
    }

    // Set executable permissions on Unix systems
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to set executable permissions: {}", e))?;
    }

    println!("cargo:warning=✨ Extraction complete");
    Ok(())
}

/// Writes `bytes` to `path` only if they are the pinned content, through a
/// temporary name, and checks the file that landed.
fn write_verified(bytes: &[u8], sha256: &str, path: &std::path::Path) -> Result<(), String> {
    let actual = sha256_hex(bytes);
    if actual != sha256 {
        return Err(format!(
            "{:?} from the FFmpeg archive: SHA-256 mismatch, expected {}, got {}",
            path.file_name().unwrap_or_default(),
            sha256,
            actual
        ));
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, bytes).map_err(|e| format!("Failed to write {:?}: {}", partial, e))?;
    std::fs::rename(&partial, path).map_err(|e| format!("Failed to place {:?}: {}", path, e))?;
    // The file on disk is what gets run and bundled
    if sha256_file(path)? != sha256 {
        let _ = std::fs::remove_file(path);
        return Err(format!("{:?} changed on its way to disk", path));
    }
    Ok(())
}

/// Whether an archive entry is one of `wanted`: matched by file name, at most
/// two folders deep (`ffmpeg-x/bin/ffmpeg.exe`, `ffmpeg-x/LICENSE`, `ffmpeg`).
fn wanted_name(path: &std::path::Path, wanted: &[&str]) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    (path.components().count() <= 3 && wanted.contains(&name)).then(|| name.to_string())
}

/// The wanted files of a ZIP held in memory, by file name.
fn files_from_zip(content: &[u8], wanted: &[&str]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(content))
        .map_err(|e| format!("Failed to read ZIP archive: {}", e))?;
    let mut found = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry {}: {}", i, e))?;
        if file.is_dir() {
            continue;
        }
        // enclosed_name() refuses path traversal ("../")
        let Some(name) = file.enclosed_name().and_then(|path| wanted_name(&path, wanted)) else {
            continue;
        };
        if found.iter().any(|(seen, _)| seen == &name) {
            return Err(format!("'{}' appears twice in the FFmpeg archive", name));
        }
        let mut bytes = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut bytes)
            .map_err(|e| format!("Failed to extract {}: {}", name, e))?;
        found.push((name, bytes));
    }
    Ok(found)
}

/// The wanted files of a TAR.XZ held in memory, by file name.
fn files_from_tar_xz(content: &[u8], wanted: &[&str]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = tar::Archive::new(xz2::read::XzDecoder::new(std::io::Cursor::new(content)));
    let mut found = Vec::new();
    for entry in archive.entries().map_err(|e| format!("Failed to read TAR: {}", e))? {
        let mut entry = entry.map_err(|e| format!("Failed to read TAR entry: {}", e))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(|e| format!("Bad TAR entry name: {}", e))?.into_owned();
        let Some(name) = wanted_name(&path, wanted) else {
            continue;
        };
        if found.iter().any(|(seen, _)| seen == &name) {
            return Err(format!("'{}' appears twice in the FFmpeg archive", name));
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)
            .map_err(|e| format!("Failed to extract {}: {}", name, e))?;
        found.push((name, bytes));
    }
    Ok(found)
}

/// Get the pinned FFmpeg download for specific target triple
///
/// Archive hashes equal the SHA-256 digests GitHub publishes for the release
/// assets. Windows comes straight from gyan.dev's official build
/// (GyanD/codexffmpeg release 8.0.1); the other targets still come from
/// Zackriya-Solutions/ffmpeg-binaries 0.0.1. Binary hashes are of the
/// executable inside each archive.
fn get_ffmpeg_source_for_target(target: &str) -> Result<FfmpegSource, String> {
    // Platform-specific URLs
    let source = if target.contains("windows") {
        // Windows: gyan.dev essentials build
        FfmpegSource {
            url: "https://github.com/GyanD/codexffmpeg/releases/download/8.0.1/ffmpeg-8.0.1-essentials_build.zip",
            archive_sha256: "e2aaeaa0fdbc397d4794828086424d4aaa2102cef1fb6874f6ffd29c0b88b673",
            binary_sha256: "5af82a0d4fe2b9eae211b967332ea97edfc51c6b328ca35b827e73eac560dc0d",
            notices: &[
                // GPL v3, the build's license
                Notice {
                    in_archive: "LICENSE",
                    install_as: "LICENSE.txt",
                    sha256: "8ceb4b9ee5adedde47b31e975c1d90c73ad27b6b165a1dcd80c7c545eb65b903",
                },
                // Version, FFmpeg source commit, configuration, linked libraries
                Notice {
                    in_archive: "README.txt",
                    install_as: "BUILD.txt",
                    sha256: "a0e976df3cf1d781264c41db8ee3421978c1278be92ed00edbc96337529670be",
                },
            ],
        }
    } else if target.contains("apple") {
        if target.contains("aarch64") {
            // Apple Silicon (M1/M2/M3)
            FfmpegSource {
                url: "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg80arm.zip",
                archive_sha256: "0d4efcaf6a098430a708e0af694a84792938921fa126162787ae98c6151d7a95",
                binary_sha256: "77d2c853f431318d55ec02676d9b2f185ebfdddb9f7677a251fbe453affe025a",
                // This archive carries no license file; see licenses/ffmpeg/SOURCE.md
                notices: &[],
            }
        } else {
            // Intel Mac
            FfmpegSource {
                url: "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg-8.0.1.zip",
                archive_sha256: "470e482f6e290eac92984ac12b2d67bad425b1e5269fd75fb6a3536c16e824e4",
                binary_sha256: "430d60fbf419dab28daee9b679e7929a31ee9bae53f6e42e8ae26b725584290f",
                // This archive carries no license file; see licenses/ffmpeg/SOURCE.md
                notices: &[],
            }
        }
    } else if target.contains("linux") {
        if target.contains("aarch64") || target.contains("arm") {
            // Linux ARM64 (the archive contains FFmpeg 7.0.2, not 8.0.1)
            FfmpegSource {
                url: "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg-release-arm64-static.tar.xz",
                archive_sha256: "f4149bb2b0784e30e99bdda85471c9b5930d3402014e934a5098b41d0f7201b1",
                binary_sha256: "6bb182d0d75d23028db82e9e4f723ca69b853d055698486e6984ddb2c06fb8ce",
                // This archive carries no license file; see licenses/ffmpeg/SOURCE.md
                notices: &[],
            }
        } else {
            // Linux x86_64 (the archive contains FFmpeg 7.0.2, not 8.0.1)
            FfmpegSource {
                url: "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg-release-amd64-static.tar.xz",
                archive_sha256: "abda8d77ce8309141f83ab8edf0596834087c52467f6badf376a6a2a4c87cf67",
                binary_sha256: "e7e7fb30477f717e6f55f9180a70386c62677ef8a4d4d1a5d948f4098aa3eb99",
                // This archive carries no license file; see licenses/ffmpeg/SOURCE.md
                notices: &[],
            }
        }
    } else {
        return Err(format!("Unsupported target platform: {}", target));
    };

    Ok(source)
}

/// Lowercase hex SHA-256 of a byte slice
fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

/// Lowercase hex SHA-256 of a file, streamed
fn sha256_file(path: &std::path::Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open {:?}: {}", path, e))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .map_err(|e| format!("Failed to read {:?}: {}", path, e))?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Verify FFmpeg binary is functional (runs -version successfully)
fn verify_ffmpeg_binary(path: &std::path::PathBuf) -> bool {
    match std::process::Command::new(path)
        .arg("-version")
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(version_line) = stdout.lines().next() {
                    println!("cargo:warning=✅ FFmpeg verification passed: {}", version_line);
                }
                true
            } else {
                false
            }
        }
        Err(_) => false,
    }
}
