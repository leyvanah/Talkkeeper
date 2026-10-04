// ============================================================================
// FFmpeg Binary Bundling
// ============================================================================
// Download and bundle FFmpeg binaries at build-time to eliminate runtime download delays
//
// The bundled FFmpeg ships inside the installer and processes decrypted audio,
// so every archive and every extracted binary is pinned by SHA-256. Nothing is
// extracted or executed before its hash matches; a mismatch fails the build.

use sha2::{Digest, Sha256};

/// A pinned FFmpeg download for one target.
struct FfmpegSource {
    url: &'static str,
    /// SHA-256 of the archive at `url`, as published for the release asset.
    archive_sha256: &'static str,
    /// SHA-256 of the ffmpeg executable inside the archive.
    binary_sha256: &'static str,
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
                if verify_ffmpeg_binary(&binary_path) {
                    println!("cargo:warning=✅ FFmpeg binary already cached and verified: {}", binary_name);
                    return;
                }
                println!("cargo:warning=⚠️  Cached FFmpeg binary failed to run, re-downloading...");
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

    println!("cargo:warning=📥 FFmpeg binary not found, downloading for {}", target);

    // Create binaries directory if it doesn't exist
    if !binaries_dir.exists() {
        std::fs::create_dir_all(&binaries_dir)
            .expect("Failed to create binaries directory");
    }

    // Download and extract
    match download_and_extract_ffmpeg(&target, &source, &binary_path) {
        Ok(()) => {
            println!("cargo:warning=✅ FFmpeg binary downloaded successfully: {}", binary_name);

            // Hash already checked in extract_ffmpeg_from_archive; now it is safe to run
            if !verify_ffmpeg_binary(&binary_path) {
                panic!("⚠️  Downloaded FFmpeg binary verification failed!");
            }
        }
        Err(e) => {
            panic!("⚠️  Failed to download FFmpeg: {}", e);
        }
    }
}

/// Download FFmpeg from platform-specific URL and extract to target location
fn download_and_extract_ffmpeg(
    target: &str,
    source: &FfmpegSource,
    output_path: &std::path::PathBuf,
) -> Result<(), String> {
    use std::io::Write;

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

    // Download to temp file
    let temp_dir = std::env::temp_dir();
    let archive_filename = url.split('/').next_back().unwrap_or("ffmpeg-archive");
    let archive_path = temp_dir.join(format!("ffmpeg-build-{}-{}", target, archive_filename));

    {
        let content = response.bytes()
            .map_err(|e| format!("Failed to read response: {}", e))?;

        // Verify before the archive touches the disk or gets extracted
        let actual = sha256_hex(&content);
        if actual != source.archive_sha256 {
            return Err(format!(
                "FFmpeg archive SHA-256 mismatch for {}: expected {}, got {}. Refusing to extract.",
                url, source.archive_sha256, actual
            ));
        }
        println!("cargo:warning=🔒 FFmpeg archive SHA-256 matches pinned value: {}", actual);

        let mut file = std::fs::File::create(&archive_path)
            .map_err(|e| format!("Failed to create temp file: {}", e))?;

        file.write_all(&content)
            .map_err(|e| format!("Failed to write archive: {}", e))?;
    }

    println!("cargo:warning=📦 Downloaded to: {:?}", archive_path);
    println!("cargo:warning=📂 Extracting FFmpeg binary...");

    // Extract binary (platform-specific)
    extract_ffmpeg_from_archive(&archive_path, target, source, output_path)?;

    // Cleanup archive
    let _ = std::fs::remove_file(&archive_path);

    println!("cargo:warning=✨ Extraction complete");

    Ok(())
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
        }
    } else if target.contains("apple") {
        if target.contains("aarch64") {
            // Apple Silicon (M1/M2/M3)
            FfmpegSource {
                url: "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg80arm.zip",
                archive_sha256: "0d4efcaf6a098430a708e0af694a84792938921fa126162787ae98c6151d7a95",
                binary_sha256: "77d2c853f431318d55ec02676d9b2f185ebfdddb9f7677a251fbe453affe025a",
            }
        } else {
            // Intel Mac
            FfmpegSource {
                url: "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg-8.0.1.zip",
                archive_sha256: "470e482f6e290eac92984ac12b2d67bad425b1e5269fd75fb6a3536c16e824e4",
                binary_sha256: "430d60fbf419dab28daee9b679e7929a31ee9bae53f6e42e8ae26b725584290f",
            }
        }
    } else if target.contains("linux") {
        if target.contains("aarch64") || target.contains("arm") {
            // Linux ARM64 (the archive contains FFmpeg 7.0.2, not 8.0.1)
            FfmpegSource {
                url: "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg-release-arm64-static.tar.xz",
                archive_sha256: "f4149bb2b0784e30e99bdda85471c9b5930d3402014e934a5098b41d0f7201b1",
                binary_sha256: "6bb182d0d75d23028db82e9e4f723ca69b853d055698486e6984ddb2c06fb8ce",
            }
        } else {
            // Linux x86_64 (the archive contains FFmpeg 7.0.2, not 8.0.1)
            FfmpegSource {
                url: "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg-release-amd64-static.tar.xz",
                archive_sha256: "abda8d77ce8309141f83ab8edf0596834087c52467f6badf376a6a2a4c87cf67",
                binary_sha256: "e7e7fb30477f717e6f55f9180a70386c62677ef8a4d4d1a5d948f4098aa3eb99",
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

/// Extract FFmpeg binary from downloaded archive (handles ZIP and TAR.XZ)
fn extract_ffmpeg_from_archive(
    archive_path: &std::path::Path,
    target: &str,
    source: &FfmpegSource,
    output_path: &std::path::PathBuf,
) -> Result<(), String> {
    let extract_dir = std::env::temp_dir().join(format!("ffmpeg-extract-{}", target));

    // Clean old extraction directory
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    // Determine archive format from extension
    let archive_str = archive_path.to_string_lossy();

    if archive_str.ends_with(".zip") {
        extract_zip(archive_path, &extract_dir)?;
    } else if archive_str.ends_with(".tar.xz") || archive_str.ends_with(".txz") {
        extract_tar_xz(archive_path, &extract_dir)?;
    } else {
        return Err(format!("Unsupported archive format: {}", archive_str));
    }

    // Find extracted FFmpeg binary (platform-specific locations)
    let ffmpeg_binary = find_ffmpeg_in_extracted_dir(&extract_dir, target)?;

    println!("cargo:warning=📋 Found FFmpeg at: {:?}", ffmpeg_binary);

    // Copy to target location
    std::fs::copy(&ffmpeg_binary, output_path)
        .map_err(|e| format!("Failed to copy binary to binaries/: {}", e))?;
    let _ = std::fs::remove_dir_all(&extract_dir);

    // Verify the copy itself, not the temp file: this is the file that gets run
    // and bundled, and the temp dir may be shared (/tmp on Unix)
    let actual = sha256_file(output_path)?;
    if actual != source.binary_sha256 {
        let _ = std::fs::remove_file(output_path);
        return Err(format!(
            "Extracted FFmpeg binary SHA-256 mismatch: expected {}, got {}",
            source.binary_sha256, actual
        ));
    }
    println!("cargo:warning=🔒 FFmpeg binary SHA-256 matches pinned value: {}", actual);

    // Set executable permissions on Unix systems
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(output_path)
            .map_err(|e| format!("Failed to get metadata: {}", e))?
            .permissions();
        perms.set_mode(0o755); // rwxr-xr-x
        std::fs::set_permissions(output_path, perms)
            .map_err(|e| format!("Failed to set executable permissions: {}", e))?;
        println!("cargo:warning=🔐 Set executable permissions");
    }

    Ok(())
}

/// Extract ZIP archive (Windows, macOS)
fn extract_zip(
    archive_path: &std::path::Path,
    extract_dir: &std::path::Path,
) -> Result<(), String> {
    

    let file = std::fs::File::open(archive_path)
        .map_err(|e| format!("Failed to open ZIP: {}", e))?;

    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Failed to read ZIP archive: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry {}: {}", i, e))?;

        // Use enclosed_name() to prevent Zip Slip path traversal attacks
        let outpath = match file.enclosed_name() {
            Some(name) => extract_dir.join(name),
            None => {
                // Skip entries with path traversal sequences (e.g., "../")
                println!("cargo:warning=⚠️  Skipping suspicious ZIP entry: {}", file.name());
                continue;
            }
        };

        if file.is_dir() {
            // Directory
            std::fs::create_dir_all(&outpath)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        } else {
            // File
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent directory: {}", e))?;
            }

            let mut outfile = std::fs::File::create(&outpath)
                .map_err(|e| format!("Failed to create output file: {}", e))?;

            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to extract file: {}", e))?;
        }

        // Set Unix permissions if available
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = file.unix_mode() {
                std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode))
                    .ok();
            }
        }
    }

    Ok(())
}

/// Extract TAR.XZ archive (Linux)
fn extract_tar_xz(
    archive_path: &std::path::Path,
    extract_dir: &std::path::Path,
) -> Result<(), String> {
    let file = std::fs::File::open(archive_path)
        .map_err(|e| format!("Failed to open TAR.XZ: {}", e))?;

    // Decompress XZ
    let decompressor = xz2::read::XzDecoder::new(file);

    // Extract TAR
    let mut archive = tar::Archive::new(decompressor);
    archive.unpack(extract_dir)
        .map_err(|e| format!("Failed to extract TAR: {}", e))?;

    Ok(())
}

/// Find FFmpeg binary in extracted directory (handles nested structures)
fn find_ffmpeg_in_extracted_dir(
    extract_dir: &std::path::Path,
    target: &str,
) -> Result<std::path::PathBuf, String> {
    let executable_name = if target.contains("windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };

    // Search patterns (in priority order)
    let search_patterns = [
        extract_dir.join(executable_name),                    // Flat: ffmpeg
        extract_dir.join("bin").join(executable_name),        // Nested: bin/ffmpeg
    ];

    // Try direct paths first
    for pattern in &search_patterns {
        if pattern.exists() && pattern.is_file() {
            return Ok(pattern.clone());
        }
    }

    // Recursive search for nested directories (e.g., ffmpeg-6.0-full_build/bin/ffmpeg.exe)
    for entry in std::fs::read_dir(extract_dir)
        .map_err(|e| format!("Failed to read extract dir: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();

        if path.is_dir() {
            // Check bin/ subdirectory
            let bin_path = path.join("bin").join(executable_name);
            if bin_path.exists() && bin_path.is_file() {
                return Ok(bin_path);
            }

            // Check root of subdirectory
            let root_path = path.join(executable_name);
            if root_path.exists() && root_path.is_file() {
                return Ok(root_path);
            }
        }
    }

    Err(format!("FFmpeg binary '{}' not found in extracted archive", executable_name))
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
