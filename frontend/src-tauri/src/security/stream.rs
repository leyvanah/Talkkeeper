//! Encrypting a file that still has to be seekable.
//!
//! A recording is written once, from beginning to end, and then read in pieces:
//! the player asks for the bytes around wherever the owner dragged the
//! playhead, and the decoder walks the container's index. Encrypting the file
//! as one sealed blob would make both of those read the whole session into
//! memory to get at a few seconds of it, and would lose the property A3 paid
//! for — that a recording cut short by a crash is still playable.
//!
//! So the stream is sealed in fixed frames of [`FRAME_LEN`] plaintext bytes,
//! each its own AES-256-GCM message. Fixed size is what buys random access: a
//! plaintext offset divides into a frame index and an offset inside it, and the
//! frame's place in the file is arithmetic rather than a search. One seek and
//! one 64 KiB decryption answer any read.
//!
//! ## What the framing is asked to prove
//!
//! Per-frame authentication alone stops a frame being altered, but not the file
//! being rearranged, so each frame's AAD carries the header (which holds the
//! per-file nonce base), the frame's own index, and whether it is the last one:
//!
//! * a frame moved within the file fails — its index is wrong;
//! * a frame moved *between* files fails — the nonce base is wrong;
//! * cutting frames off the end is visible, because the frame that said it was
//!   last is gone. That is reported rather than refused: a recording killed
//!   mid-session is exactly a stream with no final frame, and fifty minutes of
//!   a session is worth more than a clean error. [`EncryptedReader::complete`]
//!   says which of the two happened.
//!
//! ## What it deliberately does not do
//!
//! The plaintext length is not stored anywhere. It follows from the file's
//! size, so nothing has to be seeked back and rewritten at the end — which is
//! what would otherwise turn a crash into an unreadable file. Frame size lives
//! in the header so a later version can change it without orphaning what is
//! already on disk.

use std::io::{self, Read, Seek, SeekFrom, Write};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;

use super::envelope::Dek;

/// Marks a file as an encrypted stream. Deliberately not a valid prefix of any
/// container the application writes, so sniffing it cannot misfire.
pub const MAGIC: &[u8; 8] = b"TKENC\x00\x01\n";

/// The only format version so far: AES-256-GCM over fixed plaintext frames.
const VERSION: u8 = 1;

/// Plaintext bytes per frame.
///
/// Sixty-four kilobytes is about 2.7 seconds of the 192 kbit/s AAC this
/// application records, so a seek decrypts a few seconds to reach one, and an
/// hour of audio is some 700 frames — small enough that the per-frame tag costs
/// 0.02% of the file.
pub const FRAME_LEN: usize = 64 * 1024;

/// Bytes a sealed frame occupies: its plaintext plus the GCM tag.
const TAG_LEN: usize = 16;

/// Length of the header at the start of the file.
const HEADER_LEN: usize = 32;

/// Random bytes in the header that make every file's nonces its own.
const NONCE_BASE_LEN: usize = 8;

/// Refuse absurd frame sizes from a header before allocating for them. The
/// header is authenticated, so a tampered size would fail anyway — but only
/// after the allocation it asked for.
const MIN_FRAME_LEN: usize = 1024;
const MAX_FRAME_LEN: usize = 4 * 1024 * 1024;

/// What can go wrong opening an encrypted stream.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("this file is not an encrypted stream")]
    NotEncrypted,
    #[error("the encrypted stream is a version this build does not know: {0}")]
    UnknownVersion(u8),
    #[error("the encrypted stream declares an impossible frame size: {0}")]
    ImpossibleFrameSize(u32),
    #[error("the key does not open this stream, or the stream is damaged")]
    WrongKeyOrDamaged,
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl From<StreamError> for io::Error {
    fn from(error: StreamError) -> Self {
        match error {
            StreamError::Io(inner) => inner,
            other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
        }
    }
}

/// Does this look like an encrypted stream?
///
/// Reading a recording has to work on archives written before B3, so the reader
/// is chosen by what the file actually starts with rather than by its name. An
/// unreadable or too-short file is not encrypted as far as this is concerned;
/// whatever opens it next will produce the real error.
pub fn looks_encrypted(bytes: &[u8]) -> bool {
    bytes.len() >= MAGIC.len() && &bytes[..MAGIC.len()] == MAGIC.as_slice()
}

/// Does the file at this path start with the magic?
pub fn file_looks_encrypted(path: &std::path::Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; MAGIC.len()];
    match file.read_exact(&mut head) {
        Ok(()) => looks_encrypted(&head),
        Err(_) => false,
    }
}

fn cipher_for(key: &Dek) -> Aes256Gcm {
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_ref()))
}

/// The header as it sits on disk, and the pieces of it that matter.
#[derive(Clone)]
struct Header {
    bytes: [u8; HEADER_LEN],
    frame_len: usize,
}

impl Header {
    fn new(frame_len: usize) -> Self {
        let mut bytes = [0u8; HEADER_LEN];
        bytes[..MAGIC.len()].copy_from_slice(MAGIC.as_slice());
        bytes[8] = VERSION;
        bytes[9..13].copy_from_slice(&(frame_len as u32).to_le_bytes());
        rand::thread_rng().fill_bytes(&mut bytes[13..13 + NONCE_BASE_LEN]);
        Self { bytes, frame_len }
    }

    fn parse(bytes: [u8; HEADER_LEN]) -> Result<Self, StreamError> {
        if !looks_encrypted(&bytes) {
            return Err(StreamError::NotEncrypted);
        }
        if bytes[8] != VERSION {
            return Err(StreamError::UnknownVersion(bytes[8]));
        }
        let declared = u32::from_le_bytes([bytes[9], bytes[10], bytes[11], bytes[12]]);
        let frame_len = declared as usize;
        if frame_len < MIN_FRAME_LEN || frame_len > MAX_FRAME_LEN {
            return Err(StreamError::ImpossibleFrameSize(declared));
        }
        Ok(Self { bytes, frame_len })
    }

    /// Nonce for one frame: this file's random base, then the frame's index.
    /// Unique per (file, frame), which is all GCM asks of it.
    fn nonce(&self, frame: u64) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[..NONCE_BASE_LEN].copy_from_slice(&self.bytes[13..13 + NONCE_BASE_LEN]);
        nonce[NONCE_BASE_LEN..].copy_from_slice(&(frame as u32).to_be_bytes());
        nonce
    }

    /// What each frame is bound to: the whole header, the frame's index, and
    /// whether it closes the stream.
    fn aad(&self, frame: u64, last: bool) -> Vec<u8> {
        let mut aad = Vec::with_capacity(HEADER_LEN + 9);
        aad.extend_from_slice(&self.bytes);
        aad.extend_from_slice(&frame.to_le_bytes());
        aad.push(u8::from(last));
        aad
    }

    fn stored_frame_len(&self) -> u64 {
        (self.frame_len + TAG_LEN) as u64
    }
}

/// Writes an encrypted stream, one frame at a time.
///
/// Implements [`Write`], so it can stand in for a file wherever bytes are
/// handed over as they are produced. Note what [`Write::flush`] does *not* do:
/// it pushes whatever is already sealed at the underlying writer and leaves the
/// part-filled frame alone. Sealing short frames on demand would break the
/// fixed framing that random access depends on.
pub struct EncryptedWriter<W: Write> {
    inner: W,
    cipher: Aes256Gcm,
    header: Header,
    /// Plaintext waiting for its frame to fill up.
    pending: Vec<u8>,
    next_frame: u64,
    finished: bool,
}

impl<W: Write> EncryptedWriter<W> {
    /// Start an encrypted stream, writing the header immediately so that even a
    /// recording that dies in its first second leaves a file that is recognised
    /// rather than mistaken for plaintext.
    pub fn create(inner: W, key: &Dek) -> io::Result<Self> {
        Self::create_with_frame_len(inner, key, FRAME_LEN)
    }

    fn create_with_frame_len(mut inner: W, key: &Dek, frame_len: usize) -> io::Result<Self> {
        let header = Header::new(frame_len);
        inner.write_all(&header.bytes)?;
        Ok(Self {
            inner,
            cipher: cipher_for(key),
            header,
            pending: Vec::with_capacity(frame_len),
            next_frame: 0,
            finished: false,
        })
    }

    /// Seal the part-filled frame, marking it as the last, and flush. After
    /// this the file reads back as complete.
    pub fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.seal(true)?;
        self.finished = true;
        self.inner.flush()
    }

    fn seal(&mut self, last: bool) -> io::Result<()> {
        let nonce = self.header.nonce(self.next_frame);
        let aad = self.header.aad(self.next_frame, last);
        let sealed = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &self.pending,
                    aad: &aad,
                },
            )
            // GCM refuses only absurd message sizes, and a frame is 64 KiB.
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "could not encrypt a frame"))?;
        self.inner.write_all(&sealed)?;
        self.pending.clear();
        self.next_frame += 1;
        Ok(())
    }
}

impl<W: Write> Write for EncryptedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.finished {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "the encrypted stream is already closed",
            ));
        }
        let mut written = 0;
        while written < bytes.len() {
            let room = self.header.frame_len - self.pending.len();
            let take = room.min(bytes.len() - written);
            self.pending.extend_from_slice(&bytes[written..written + take]);
            written += take;
            if self.pending.len() == self.header.frame_len {
                self.seal(false)?;
            }
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Reads an encrypted stream as though it were the plaintext file.
///
/// Implements [`Read`] and [`Seek`] over plaintext offsets: everything above
/// this — the decoder, the player's range requests — works in the offsets it
/// already knows, and never learns that the file on disk is longer than the
/// audio in it.
pub struct EncryptedReader<R: Read + Seek> {
    inner: R,
    cipher: Aes256Gcm,
    header: Header,
    /// Frames that authenticate. A torn tail is not one of them.
    frames: u64,
    /// Plaintext length of the final frame.
    last_frame_len: usize,
    plaintext_len: u64,
    position: u64,
    /// The one frame kept decrypted, because reads arrive in small pieces and
    /// re-decrypting 64 KiB for each of them would be the whole cost of reading.
    cached: Option<(u64, Vec<u8>)>,
    complete: bool,
}

impl<R: Read + Seek> EncryptedReader<R> {
    /// Open the stream and work out how long the plaintext is.
    ///
    /// This reads and authenticates the final frame, which is what a wrong key
    /// fails on: opening is the moment to find that out, not halfway through
    /// playback.
    pub fn open(mut inner: R, key: &Dek) -> Result<Self, StreamError> {
        let stored_len = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(0))?;

        let mut header_bytes = [0u8; HEADER_LEN];
        inner.read_exact(&mut header_bytes).map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                StreamError::NotEncrypted
            } else {
                StreamError::Io(error)
            }
        })?;
        let header = Header::parse(header_bytes)?;

        let mut reader = Self {
            inner,
            cipher: cipher_for(key),
            header,
            frames: 0,
            last_frame_len: 0,
            plaintext_len: 0,
            position: 0,
            cached: None,
            complete: true,
        };
        reader.measure(stored_len)?;
        Ok(reader)
    }

    /// Whether the stream ends with the frame that said it was the last one.
    ///
    /// False means the tail is missing: a recording that was killed, or a file
    /// someone cut short. The frames that are here are genuine either way.
    pub fn complete(&self) -> bool {
        self.complete
    }

    /// Length of the plaintext, in bytes.
    pub fn len(&self) -> u64 {
        self.plaintext_len
    }

    pub fn is_empty(&self) -> bool {
        self.plaintext_len == 0
    }

    /// Count the frames and authenticate the last one to learn its length.
    fn measure(&mut self, stored_len: u64) -> Result<(), StreamError> {
        let body = stored_len.saturating_sub(HEADER_LEN as u64);
        let stride = self.header.stored_frame_len();
        let whole = body / stride;
        let tail = (body % stride) as usize;

        // A stream that was never written past its header has nothing in it,
        // and nothing to authenticate either.
        if whole == 0 && tail == 0 {
            self.frames = 0;
            self.last_frame_len = 0;
            self.plaintext_len = 0;
            self.complete = false;
            return Ok(());
        }

        // The tail is either a legitimate short final frame or the remains of a
        // write that did not finish. A short frame cannot be anything but last,
        // so it is tried as one; failing that it is discarded as a torn write
        // and the whole frames before it stand on their own.
        if tail > 0 {
            // `>= TAG_LEN` rather than `>`: a stream whose plaintext divides
            // exactly into frames ends with an *empty* final frame, which is a
            // tag and nothing else. Requiring more than a tag here read every
            // such recording as though it had been killed.
            if tail >= TAG_LEN {
                if let Ok(plain) = self.decrypt_frame_at(whole, tail, true) {
                    self.frames = whole + 1;
                    self.last_frame_len = plain.len();
                    self.plaintext_len = whole * self.header.frame_len as u64 + plain.len() as u64;
                    self.complete = true;
                    self.cached = Some((whole, plain));
                    return Ok(());
                }
            }
            if whole == 0 {
                return Err(StreamError::WrongKeyOrDamaged);
            }
        }

        // No usable tail: the last whole frame is the end of what is here. It is
        // tried as a final frame first (a stream whose plaintext divides exactly
        // into frames still ends with an empty marked frame, so this path is a
        // stream that lost its marker) and then as an ordinary one.
        let last = whole - 1;
        if tail == 0 {
            if let Ok(plain) = self.decrypt_frame_at(last, self.header.frame_len + TAG_LEN, true) {
                self.frames = whole;
                self.last_frame_len = plain.len();
                self.plaintext_len = last * self.header.frame_len as u64 + plain.len() as u64;
                self.complete = true;
                self.cached = Some((last, plain));
                return Ok(());
            }
        }
        let plain = self.decrypt_frame_at(last, self.header.frame_len + TAG_LEN, false)?;
        self.frames = whole;
        self.last_frame_len = plain.len();
        self.plaintext_len = whole * self.header.frame_len as u64;
        self.complete = false;
        self.cached = Some((last, plain));
        Ok(())
    }

    fn frame_offset(&self, frame: u64) -> u64 {
        HEADER_LEN as u64 + frame * self.header.stored_frame_len()
    }

    fn decrypt_frame_at(
        &mut self,
        frame: u64,
        stored_len: usize,
        last: bool,
    ) -> Result<Vec<u8>, StreamError> {
        self.inner.seek(SeekFrom::Start(self.frame_offset(frame)))?;
        let mut sealed = vec![0u8; stored_len];
        self.inner.read_exact(&mut sealed)?;
        let nonce = self.header.nonce(frame);
        let aad = self.header.aad(frame, last);
        self.cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &sealed,
                    aad: &aad,
                },
            )
            .map_err(|_| StreamError::WrongKeyOrDamaged)
    }

    /// The decrypted frame `frame`, from the cache when it is the one already
    /// open.
    fn frame(&mut self, frame: u64) -> io::Result<&[u8]> {
        if self.cached.as_ref().map(|(index, _)| *index) != Some(frame) {
            let last = frame + 1 == self.frames;
            let stored = if last {
                self.last_frame_len + TAG_LEN
            } else {
                self.header.frame_len + TAG_LEN
            };
            let marked_last = last && self.complete;
            let plain = self.decrypt_frame_at(frame, stored, marked_last)?;
            self.cached = Some((frame, plain));
        }
        Ok(self
            .cached
            .as_ref()
            .map(|(_, plain)| plain.as_slice())
            .unwrap_or_default())
    }
}

impl<R: Read + Seek> Read for EncryptedReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.position >= self.plaintext_len {
            return Ok(0);
        }
        let frame = self.position / self.header.frame_len as u64;
        let inside = (self.position % self.header.frame_len as u64) as usize;
        let plain = self.frame(frame)?;
        if inside >= plain.len() {
            return Ok(0);
        }
        let take = (plain.len() - inside).min(out.len());
        out[..take].copy_from_slice(&plain[inside..inside + take]);
        self.position += take as u64;
        Ok(take)
    }
}

impl<R: Read + Seek> Seek for EncryptedReader<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => self.plaintext_len as i64 + offset,
            SeekFrom::Current(offset) => self.position as i64 + offset,
        };
        if target < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot seek before the start of the stream",
            ));
        }
        // Seeking past the end is allowed, as it is on a file; reads there
        // return nothing.
        self.position = (target as u64).min(self.plaintext_len);
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn key() -> Dek {
        super::super::envelope::generate_dek()
    }

    /// Bytes that are obviously themselves, so a misplaced frame shows up as
    /// wrong content rather than as a plausible-looking mistake.
    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|index| (index % 251) as u8).collect()
    }

    fn seal(plain: &[u8], key: &Dek) -> Vec<u8> {
        let mut writer = EncryptedWriter::create(Cursor::new(Vec::new()), key).unwrap();
        writer.write_all(plain).unwrap();
        writer.finish().unwrap();
        writer.inner.into_inner()
    }

    /// The error from an open that must not succeed. Written out because the
    /// reader is generic over a stream that need not be `Debug`, so
    /// `unwrap_err` is not available on it.
    fn open_error(sealed: Vec<u8>, key: &Dek) -> StreamError {
        match EncryptedReader::open(Cursor::new(sealed), key) {
            Err(error) => error,
            Ok(_) => panic!("the stream opened when it should not have"),
        }
    }

    fn read_all(sealed: Vec<u8>, key: &Dek) -> (Vec<u8>, bool) {
        let mut reader = EncryptedReader::open(Cursor::new(sealed), key).unwrap();
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        (out, reader.complete())
    }

    #[test]
    fn what_goes_in_comes_back_out() {
        let key = key();
        for len in [0usize, 1, 100, FRAME_LEN - 1, FRAME_LEN, FRAME_LEN + 1, 5 * FRAME_LEN + 77] {
            let plain = pattern(len);
            let (out, complete) = read_all(seal(&plain, &key), &key);
            assert_eq!(out, plain, "length {len} did not survive the round trip");
            assert!(complete, "length {len} should read back as complete");
        }
    }

    #[test]
    fn the_writer_does_not_care_how_the_bytes_arrive() {
        // FFmpeg hands over whatever a pipe read returned, which is never a
        // neat multiple of anything.
        let key = key();
        let plain = pattern(3 * FRAME_LEN + 1234);
        let mut writer = EncryptedWriter::create(Cursor::new(Vec::new()), &key).unwrap();
        let mut offset = 0;
        for size in [1usize, 7, 4095, FRAME_LEN, 65_537, 13, 200_000] {
            let take = size.min(plain.len() - offset);
            writer.write_all(&plain[offset..offset + take]).unwrap();
            offset += take;
            writer.flush().unwrap();
        }
        writer.write_all(&plain[offset..]).unwrap();
        writer.finish().unwrap();
        let (out, _) = read_all(writer.inner.into_inner(), &key);
        assert_eq!(out, plain);
    }

    #[test]
    fn any_offset_can_be_read_without_reading_what_came_before() {
        let key = key();
        let plain = pattern(4 * FRAME_LEN + 999);
        let sealed = seal(&plain, &key);
        let mut reader = EncryptedReader::open(Cursor::new(sealed), &key).unwrap();

        // Offsets at frame boundaries, across them, and at the very end.
        for offset in [
            0u64,
            1,
            FRAME_LEN as u64 - 1,
            FRAME_LEN as u64,
            FRAME_LEN as u64 + 1,
            2 * FRAME_LEN as u64 + 500,
            4 * FRAME_LEN as u64 + 998,
        ] {
            reader.seek(SeekFrom::Start(offset)).unwrap();
            let mut window = vec![0u8; 300];
            let read = reader.read(&mut window).unwrap();
            assert!(read > 0, "nothing came back at offset {offset}");
            let expected = &plain[offset as usize..offset as usize + read];
            assert_eq!(&window[..read], expected, "wrong bytes at offset {offset}");
        }

        reader.seek(SeekFrom::End(0)).unwrap();
        assert_eq!(reader.read(&mut [0u8; 16]).unwrap(), 0);
        assert_eq!(reader.len(), plain.len() as u64);
    }

    #[test]
    fn a_wrong_key_is_refused_at_open_rather_than_mid_playback() {
        let plain = pattern(2 * FRAME_LEN);
        let sealed = seal(&plain, &key());
        let error = open_error(sealed, &key());
        assert!(matches!(error, StreamError::WrongKeyOrDamaged), "{error:?}");
    }

    #[test]
    fn plaintext_is_not_refused_as_a_damaged_stream() {
        // Archives recorded before B3 have to keep opening, so the reader says
        // "not encrypted" and lets the caller fall back.
        let error = open_error(pattern(9000), &key());
        assert!(matches!(error, StreamError::NotEncrypted), "{error:?}");
        assert!(!looks_encrypted(&pattern(9000)));
    }

    #[test]
    fn a_changed_byte_does_not_go_unnoticed() {
        let key = key();
        let plain = pattern(2 * FRAME_LEN + 10);
        let mut sealed = seal(&plain, &key);
        // Somewhere inside the second frame.
        let target = HEADER_LEN + (FRAME_LEN + TAG_LEN) + 100;
        sealed[target] ^= 0xff;

        let mut reader = EncryptedReader::open(Cursor::new(sealed), &key).unwrap();
        // The first frame is untouched and still readable.
        let mut window = vec![0u8; 64];
        reader.read_exact(&mut window).unwrap();
        assert_eq!(&window[..], &plain[..64]);
        // The damaged one is an error, not silently different audio.
        reader.seek(SeekFrom::Start(FRAME_LEN as u64 + 10)).unwrap();
        let failure = reader.read(&mut window).unwrap_err();
        assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_frame_moved_somewhere_else_in_the_file_fails() {
        let key = key();
        let plain = pattern(3 * FRAME_LEN);
        let sealed = seal(&plain, &key);
        let stride = FRAME_LEN + TAG_LEN;
        let first = HEADER_LEN;
        let second = HEADER_LEN + stride;
        let moved = sealed[first..first + stride].to_vec();
        let mut rearranged = sealed;
        rearranged[second..second + stride].copy_from_slice(&moved);

        let mut reader = EncryptedReader::open(Cursor::new(rearranged), &key).unwrap();
        reader.seek(SeekFrom::Start(FRAME_LEN as u64)).unwrap();
        let failure = reader.read(&mut [0u8; 32]).unwrap_err();
        assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_frame_from_another_recording_does_not_pass_for_one_of_ours() {
        let key = key();
        let ours = seal(&pattern(2 * FRAME_LEN), &key);
        let theirs = seal(&vec![7u8; 2 * FRAME_LEN], &key);
        let stride = FRAME_LEN + TAG_LEN;
        let mut spliced = ours.clone();
        spliced[HEADER_LEN..HEADER_LEN + stride]
            .copy_from_slice(&theirs[HEADER_LEN..HEADER_LEN + stride]);

        let mut reader = EncryptedReader::open(Cursor::new(spliced), &key).unwrap();
        let failure = reader.read(&mut [0u8; 32]).unwrap_err();
        assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_recording_that_was_killed_reads_up_to_where_it_stopped() {
        // What a crash leaves: whole frames, no final frame, and possibly a
        // half-written one on the end.
        let key = key();
        let plain = pattern(3 * FRAME_LEN);
        let mut writer = EncryptedWriter::create(Cursor::new(Vec::new()), &key).unwrap();
        writer.write_all(&plain).unwrap();
        writer.flush().unwrap();
        let mut killed = writer.inner.into_inner();
        killed.extend_from_slice(&pattern(5000)); // a torn fourth frame

        let (out, complete) = read_all(killed, &key);
        assert_eq!(out, plain, "the frames that were written should all be there");
        assert!(!complete, "a killed recording is not a complete stream");
    }

    #[test]
    fn cutting_the_end_off_a_finished_recording_is_visible() {
        let key = key();
        let plain = pattern(3 * FRAME_LEN + 10);
        let mut sealed = seal(&plain, &key);
        // Remove the final, marked frame entirely — the quiet kind of tampering
        // that per-frame authentication alone would not catch.
        sealed.truncate(HEADER_LEN + 3 * (FRAME_LEN + TAG_LEN));

        let (out, complete) = read_all(sealed, &key);
        assert_eq!(out.len(), 3 * FRAME_LEN);
        assert!(!complete, "a stream missing its last frame must not read as complete");
    }

    #[test]
    fn the_session_is_not_lying_around_in_the_file() {
        // The point of the whole exercise: what is on disk must not contain the
        // bytes that went in.
        let key = key();
        let plain = pattern(2 * FRAME_LEN);
        let sealed = seal(&plain, &key);
        let needle = &plain[1000..1064];
        assert!(
            !sealed.windows(needle.len()).any(|window| window == needle),
            "plaintext was found in the encrypted file"
        );
        assert!(looks_encrypted(&sealed));
    }

    #[test]
    fn an_empty_stream_is_still_a_valid_one() {
        let key = key();
        let mut writer = EncryptedWriter::create(Cursor::new(Vec::new()), &key).unwrap();
        writer.finish().unwrap();
        let sealed = writer.inner.into_inner();
        let reader = EncryptedReader::open(Cursor::new(sealed), &key).unwrap();
        assert!(reader.is_empty());
        assert!(reader.complete());
    }

    #[test]
    fn a_stream_that_never_got_past_its_header_is_empty_rather_than_broken() {
        // A recording that died between creating the file and the first frame.
        let key = key();
        let writer = EncryptedWriter::create(Cursor::new(Vec::new()), &key).unwrap();
        let sealed = writer.inner.into_inner();
        let reader = EncryptedReader::open(Cursor::new(sealed), &key).unwrap();
        assert!(reader.is_empty());
        assert!(!reader.complete());
    }

    #[test]
    fn an_impossible_frame_size_is_refused_before_anything_is_allocated() {
        let key = key();
        let mut sealed = seal(&pattern(100), &key);
        sealed[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
        let error = open_error(sealed, &key);
        assert!(matches!(error, StreamError::ImpossibleFrameSize(_)), "{error:?}");
    }

    #[test]
    fn a_stream_from_a_future_version_says_so() {
        let key = key();
        let mut sealed = seal(&pattern(100), &key);
        sealed[8] = VERSION + 1;
        let error = open_error(sealed, &key);
        assert!(matches!(error, StreamError::UnknownVersion(_)), "{error:?}");
    }

    #[test]
    fn each_file_gets_its_own_nonces() {
        let key = key();
        let one = seal(&pattern(FRAME_LEN), &key);
        let two = seal(&pattern(FRAME_LEN), &key);
        assert_ne!(
            one[13..13 + NONCE_BASE_LEN],
            two[13..13 + NONCE_BASE_LEN],
            "two recordings drew the same nonce base"
        );
        assert_ne!(one, two, "the same plaintext sealed to the same bytes twice");
    }
}
