//! Handing a stored recording to the player.
//!
//! A webview cannot open a file by path, so the recording has to be served to
//! it. The way this application did that was to read the whole file in Rust
//! and return the bytes from a command — which crosses the IPC boundary as a
//! JSON array of numbers, one decimal number per byte. An hour of audio is
//! about 40 MB on disk and something like 150 MB of text by the time it
//! arrives, decoded into a 700 MB buffer to be played at all.
//!
//! Here the webview fetches the file over a URL of its own instead, and gets
//! back exactly the bytes asked for. Playback starts at the first block rather
//! than the last, seeking costs one small request, and nothing is held in
//! memory that is not being listened to.
//!
//! Two things this is also the right place for, later:
//!
//! * **Decryption (B3).** Everything that reads a recording for the player
//!   passes through [`respond`], so the file can stay encrypted on disk and be
//!   decrypted per range here.
//! * **Access control.** A URL is reachable from any page the webview loads,
//!   so the handler serves audio files under the recording folders and
//!   nothing else — not the database, not the models, not `../../` anywhere.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use log::{debug, warn};
use tauri::http::{header, Request, Response, StatusCode};
use tauri::{AppHandle, Runtime};

use super::constants::AUDIO_EXTENSIONS;

/// The scheme the webview fetches recordings over. On Windows the URL is
/// `http://recording.localhost/<percent-encoded path>`, elsewhere
/// `recording://localhost/<percent-encoded path>`; the frontend builds it with
/// Tauri's `convertFileSrc(path, "recording")` rather than by hand.
pub const SCHEME: &str = "recording";

/// Most that goes back in one answer. The player streams: it asks for what it
/// is about to need, and a whole recording in one response is both a wait
/// before the first sound and a copy of the session held in memory.
const MAX_RANGE_BYTES: u64 = 1024 * 1024;

/// The file the player should open for a meeting, if its recording is still
/// on disk. The frontend turns this into a URL with `convertFileSrc`.
///
/// Meetings recorded by the application have `audio.mp4`; imported ones keep
/// whatever they were imported as, and a meeting whose folder was moved or
/// emptied has nothing to play, which is `None` rather than an error.
#[tauri::command]
pub async fn meeting_playback_file<R: Runtime>(
    app: AppHandle<R>,
    meeting_folder: String,
) -> Result<Option<String>, String> {
    let folder = PathBuf::from(meeting_folder.trim());
    if folder.as_os_str().is_empty() || !folder.is_dir() {
        return Ok(None);
    }
    let folder = folder
        .canonicalize()
        .map_err(|error| format!("Could not resolve the meeting folder: {error}"))?;

    let roots = super::recording_preferences::recording_roots(&app).await;
    if !roots.iter().any(|root| folder.starts_with(root)) {
        return Err("That folder is not a recording folder".to_string());
    }

    Ok(super::retranscription::find_audio_file(&folder)
        .ok()
        .map(|path| path.to_string_lossy().into_owned()))
}

/// Answer one request from the player.
pub fn respond<R: Runtime>(app: &AppHandle<R>, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let Some(path) = requested_path(request.uri().path()) else {
        return refuse(StatusCode::BAD_REQUEST, "Not a readable path");
    };

    let path = match allowed_recording(app, &path) {
        Ok(path) => path,
        Err(reason) => {
            warn!("Refused to serve {}: {reason}", path.display());
            return refuse(StatusCode::FORBIDDEN, reason);
        }
    };

    match serve(&path, request.headers().get(header::RANGE)) {
        Ok(response) => response,
        Err(error) => {
            warn!("Could not serve {}: {error}", path.display());
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "Could not read the recording")
        }
    }
}

/// The file a request is asking for. The whole path arrives as one
/// percent-encoded segment, so a meeting titled in any language survives it.
fn requested_path(uri_path: &str) -> Option<PathBuf> {
    let encoded = uri_path.strip_prefix('/').unwrap_or(uri_path);
    if encoded.is_empty() {
        return None;
    }
    let decoded = percent_decode(encoded)?;
    Some(PathBuf::from(decoded))
}

/// Decode `%XX` sequences into bytes, then read those bytes as UTF-8. Written
/// out rather than pulled in because it is eight lines and one dependency.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let digits = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
                decoded.push(u8::from_str_radix(digits, 16).ok()?);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

/// The canonical path, if this is an audio file inside a recording folder.
fn allowed_recording<R: Runtime>(
    app: &AppHandle<R>,
    requested: &Path,
) -> Result<PathBuf, &'static str> {
    let path = requested.canonicalize().map_err(|_| "No such file")?;
    if !path.is_file() {
        return Err("Not a file");
    }
    if !is_audio(&path) {
        return Err("Not an audio file");
    }

    let roots = tauri::async_runtime::block_on(super::recording_preferences::recording_roots(app));
    if roots.iter().any(|root| path.starts_with(root)) {
        Ok(path)
    } else {
        Err("Outside the recording folders")
    }
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str()))
}

/// What the player should be told the file is, so it picks a decoder without
/// sniffing the whole thing first.
fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp4") | Some("m4a") => "audio/mp4",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("ogg") | Some("oga") => "audio/ogg",
        Some("webm") => "audio/webm",
        Some("aac") => "audio/aac",
        _ => "application/octet-stream",
    }
}

fn serve(path: &Path, range: Option<&header::HeaderValue>) -> std::io::Result<Response<Vec<u8>>> {
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();

    let asked_for_range = range.is_some();
    let asked = range
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_range(value, length));

    // A range that cannot be answered has to be refused as one. Answering it
    // with the file from the start is how a player ends up decoding the middle
    // of a recording as if it were the beginning.
    if asked_for_range && asked.is_none() {
        return Ok(unsatisfiable(length));
    }

    let (start, end) = asked.unwrap_or((0, length.saturating_sub(1)));
    // Hand back a block at a time rather than the whole recording. The player
    // asks for `bytes=0-` and would otherwise be sent an hour of audio to hold
    // before the first second of it plays.
    let end = end.min(start + MAX_RANGE_BYTES - 1);
    let wanted = end.saturating_sub(start) + 1;

    let mut body = vec![0u8; wanted as usize];
    file.seek(SeekFrom::Start(start))?;
    // A recording being written to is a length ago by the time it is read, so
    // take what is there rather than insisting on the whole range.
    let read = read_as_much_as_possible(&mut file, &mut body)?;
    body.truncate(read);

    debug!(
        "Serving {} bytes of {} ({}..={} of {length})",
        body.len(),
        path.display(),
        start,
        end
    );

    let builder = base_response()
        .header(header::CONTENT_TYPE, content_type(path))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, body.len());

    let response = if asked.is_some() {
        builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header(
                header::CONTENT_RANGE,
                format!("bytes {start}-{}/{length}", start + read as u64 - 1),
            )
            .body(body)
    } else {
        builder.status(StatusCode::OK).body(body)
    };

    Ok(response.unwrap_or_else(|_| refuse(StatusCode::INTERNAL_SERVER_ERROR, "Malformed response")))
}

fn read_as_much_as_possible(file: &mut std::fs::File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

/// `bytes=start-end`, as a media player asks for it. Only the single-range
/// form exists in practice, and it is the only one worth answering.
fn parse_range(value: &str, length: u64) -> Option<(u64, u64)> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || length == 0 {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let last = length - 1;

    let (start, end) = match (start.trim(), end.trim()) {
        // "bytes=-500": the last 500 bytes.
        ("", tail) => {
            let tail: u64 = tail.parse().ok()?;
            (length.saturating_sub(tail.max(1)), last)
        }
        (head, "") => (head.parse().ok()?, last),
        (head, tail) => (head.parse().ok()?, tail.parse().ok()?),
    };

    if start > last {
        return None;
    }
    Some((start, end.min(last)))
}

/// The headers every answer carries.
///
/// The page and the recording are different origins (`tauri.localhost` and
/// `recording.localhost`), so the webview needs to be told the response may be
/// used, and that the player may read the range it was given back. Tauri's own
/// protocols answer the same way.
fn base_response() -> tauri::http::response::Builder {
    Response::builder()
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "content-range")
        // Recordings are private; a copy in the webview cache would be one
        // more place holding a session, outside anything that protects them.
        .header(header::CACHE_CONTROL, "no-store")
}

fn unsatisfiable(length: u64) -> Response<Vec<u8>> {
    base_response()
        .status(StatusCode::RANGE_NOT_SATISFIABLE)
        .header(header::CONTENT_RANGE, format!("bytes */{length}"))
        .body(Vec::new())
        .expect("a refusal is always a valid response")
}

fn refuse(status: StatusCode, reason: &str) -> Response<Vec<u8>> {
    base_response()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(reason.as_bytes().to_vec())
        .expect("a refusal is always a valid response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_survives_being_carried_in_a_url() {
        // What `convertFileSrc` produces: the whole path as one segment.
        let encoded = "/D%3A%2Frecordings%2FMeeting%202026%2Faudio.mp4";
        assert_eq!(
            requested_path(encoded),
            Some(PathBuf::from("D:/recordings/Meeting 2026/audio.mp4"))
        );
    }

    #[test]
    fn a_meeting_named_in_any_language_survives_it_too() {
        // Percent-encoded UTF-8, which is what a non-Latin title becomes.
        let encoded = "/%D0%92%D1%81%D1%82%D1%80%D0%B5%D1%87%D0%B0%2Faudio.mp4";
        assert_eq!(
            requested_path(encoded),
            Some(PathBuf::from("Встреча/audio.mp4"))
        );
    }

    #[test]
    fn nothing_is_asked_for_by_an_empty_path() {
        assert_eq!(requested_path("/"), None);
        assert_eq!(requested_path(""), None);
    }

    #[test]
    fn only_audio_is_served() {
        assert!(is_audio(Path::new("/recordings/meeting/audio.mp4")));
        assert!(is_audio(Path::new("/recordings/meeting/.work/mic.FLAC")));
        assert!(!is_audio(Path::new("/data/meeting_minutes.sqlite")));
        assert!(!is_audio(Path::new("/recordings/meeting/transcripts.json")));
    }

    #[test]
    fn the_player_is_told_what_it_is_playing() {
        assert_eq!(content_type(Path::new("a/audio.mp4")), "audio/mp4");
        assert_eq!(content_type(Path::new("a/mic.flac")), "audio/flac");
        assert_eq!(content_type(Path::new("a/x.unknown")), "application/octet-stream");
    }

    #[test]
    fn a_range_is_read_the_way_a_player_writes_it() {
        assert_eq!(parse_range("bytes=0-499", 1000), Some((0, 499)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
        // Past the end is clamped, not refused: the file is what it is.
        assert_eq!(parse_range("bytes=900-5000", 1000), Some((900, 999)));
        // The tail form, used to read a container's index.
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
    }

    #[test]
    fn a_range_that_cannot_be_answered_is_not_invented() {
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 1000), None);
        assert_eq!(parse_range("bytes=abc-", 1000), None);
        assert_eq!(parse_range("bytes=0-0", 0), None);
    }

    #[test]
    fn a_range_request_is_answered_with_that_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        std::fs::write(&path, (0u8..=255).collect::<Vec<u8>>()).unwrap();

        let range = header::HeaderValue::from_static("bytes=10-19");
        let response = serve(&path, Some(&range)).unwrap();

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.body(), &(10u8..=19).collect::<Vec<u8>>());
        assert_eq!(
            response.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes 10-19/256"
        );
        assert_eq!(response.headers().get(header::ACCEPT_RANGES).unwrap(), "bytes");
    }

    #[test]
    fn a_player_is_given_a_block_at_a_time_not_the_whole_recording() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        let long = vec![7u8; (MAX_RANGE_BYTES * 2) as usize];
        std::fs::write(&path, &long).unwrap();

        // What a player opens with: everything from the start.
        let range = header::HeaderValue::from_static("bytes=0-");
        let response = serve(&path, Some(&range)).unwrap();

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.body().len() as u64, MAX_RANGE_BYTES);
        // The total is still declared, so the player knows what it is playing
        // and can ask for the rest.
        assert_eq!(
            response.headers().get(header::CONTENT_RANGE).unwrap(),
            format!("bytes 0-{}/{}", MAX_RANGE_BYTES - 1, long.len()).as_str()
        );
    }

    #[test]
    fn a_range_beyond_the_recording_is_refused_as_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        std::fs::write(&path, b"short").unwrap();

        let range = header::HeaderValue::from_static("bytes=500-600");
        let response = serve(&path, Some(&range)).unwrap();

        // Answering with the start of the file instead would have the player
        // decode the beginning as if it were the part it asked for.
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(response.headers().get(header::CONTENT_RANGE).unwrap(), "bytes */5");
        assert!(response.body().is_empty());
    }

    #[test]
    fn every_answer_may_be_used_by_the_page_that_asked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        std::fs::write(&path, b"a recording").unwrap();

        // The page and the recording are different origins, so without this
        // the webview drops the response and the player has nothing to decode.
        for response in [
            serve(&path, None).unwrap(),
            refuse(StatusCode::FORBIDDEN, "no"),
            unsatisfiable(11),
        ] {
            assert_eq!(
                response
                    .headers()
                    .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .unwrap(),
                "*"
            );
        }
    }

    #[test]
    fn without_a_range_the_whole_recording_is_answered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        std::fs::write(&path, b"a whole recording").unwrap();

        let response = serve(&path, None).unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body(), b"a whole recording");
        // Nothing about a session should end up in the webview cache.
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
    }
}
