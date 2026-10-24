//! A font with Cyrillic for the PDF export.
//!
//! The PDF writer in the window only knows the standard PDF fonts, which have
//! no Cyrillic: Russian text came out as garbage, spaced out letter by letter.
//! The export needs a TrueType font to embed. Rather than ship one, it borrows
//! a font every system already has — read from the system's font folder, never
//! fetched from anywhere.

use base64::Engine as _;
use serde::Serialize;
use std::path::PathBuf;

/// A regular face and, when there is one, its bold, as base64 TrueType.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfFont {
    pub regular: String,
    pub bold: Option<String>,
}

/// Regular and bold files to try, best first.
fn candidates() -> Vec<(PathBuf, PathBuf)> {
    let mut pairs = Vec::new();
    if cfg!(target_os = "windows") {
        let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let fonts = windir.join("Fonts");
        pairs.push((fonts.join("arial.ttf"), fonts.join("arialbd.ttf")));
        pairs.push((fonts.join("segoeui.ttf"), fonts.join("segoeuib.ttf")));
    } else if cfg!(target_os = "macos") {
        let supplemental = PathBuf::from("/System/Library/Fonts/Supplemental");
        pairs.push((supplemental.join("Arial.ttf"), supplemental.join("Arial Bold.ttf")));
    } else {
        let dejavu = PathBuf::from("/usr/share/fonts/truetype/dejavu");
        pairs.push((dejavu.join("DejaVuSans.ttf"), dejavu.join("DejaVuSans-Bold.ttf")));
        let liberation = PathBuf::from("/usr/share/fonts/truetype/liberation");
        pairs.push((
            liberation.join("LiberationSans-Regular.ttf"),
            liberation.join("LiberationSans-Bold.ttf"),
        ));
    }
    pairs
}

fn read_font(pairs: &[(PathBuf, PathBuf)]) -> Option<PdfFont> {
    let encode = |bytes: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(bytes);
    for (regular, bold) in pairs {
        let Ok(regular_bytes) = std::fs::read(regular) else {
            continue;
        };
        let name = regular.file_name().and_then(|name| name.to_str()).unwrap_or("?");
        log::info!("PDF export font: {}", name);
        return Some(PdfFont {
            regular: encode(regular_bytes),
            bold: std::fs::read(bold).ok().map(encode),
        });
    }
    None
}

#[tauri::command]
pub async fn pdf_export_font() -> Result<PdfFont, String> {
    tauri::async_runtime::spawn_blocking(|| read_font(&candidates()))
        .await
        .map_err(|error| format!("Failed to read the font: {error}"))?
        .ok_or_else(|| "noFont".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_first_pair_whose_regular_face_exists() {
        let dir = tempfile::tempdir().unwrap();
        let missing = (dir.path().join("a.ttf"), dir.path().join("a-bold.ttf"));
        let regular = dir.path().join("b.ttf");
        std::fs::write(&regular, b"regular").unwrap();
        let present = (regular, dir.path().join("b-bold.ttf"));

        let font = read_font(&[missing, present]).unwrap();
        assert_eq!(font.regular, base64::engine::general_purpose::STANDARD.encode(b"regular"));
        // No bold file: the export makes do with the regular face.
        assert_eq!(font.bold, None);
    }

    #[test]
    fn says_so_when_no_font_is_found() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_font(&[(dir.path().join("x.ttf"), dir.path().join("y.ttf"))]).is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn this_system_has_a_font_with_cyrillic() {
        assert!(read_font(&candidates()).is_some());
    }
}
