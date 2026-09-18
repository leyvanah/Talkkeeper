# Talkkeeper

<p align="center">
  <img src="frontend/src-tauri/icon-source.png" alt="Talkkeeper logo" width="200" />
</p>

<p align="center"><b>Private recording, transcription and notes for conversations that must never leave your computer.</b></p>

> **Коротко по-русски.** Talkkeeper записывает разговор, расшифровывает его и
> помогает с заметками — целиком на вашем компьютере. Записи и текст зашифрованы
> на диске, телеметрии нет, облако подключается только если вы сами этого
> захотите. Интерфейс по умолчанию русский; распознавание русской речи — GigaAM.
> Сборка для Windows 10/11.

Talkkeeper is developed by [leyvanah](https://github.com/leyvanah). It started
life as a fork of an open-source meeting recorder and has since been reshaped
around one idea: **the recording of a conversation is the most private thing on
the machine, and the software should behave accordingly.**

## Philosophy

- **Local by default, local by design.** Recording, speech recognition,
  speaker separation and summaries all run on your own computer. A cloud model is
  something you connect deliberately, with your own key — never a fallback.
- **Encrypted at rest.** With a password set, every recording and every line of
  text in the database is sealed. A copied disk, a stray backup or a synced folder
  gives nobody the conversation.
- **No telemetry, no silent network.** Analytics are switched off in the code,
  update checks are off unless you turn them on, and nothing is sent anywhere
  without you asking for it.
- **The person has the last word.** A transcript you corrected by hand stays
  corrected: recognising the recording again fills in around your edits instead
  of overwriting them.
- **Nothing is lost quietly.** Deleting app data warns you in plain words before
  it can make recordings unreadable, and the key can be exported whenever you
  want a copy.
- **Open source.** MIT licensed; read it, build it, change it.

## What it does

**Recording**
- Microphone and system audio are captured as separate tracks on a common clock,
  so two people talking at once are both kept.
- Echo handling for calls without headphones: Windows' own echo cancellation, and
  a detector that tells your voice apart from the other side's coming out of the
  speakers.
- Encoded while recording, so stopping is instant; a working 16 kHz track is kept
  for fast re-recognition.
- A floating mini bar, and optional detection of meeting apps.

**Transcription**
- Local engines: Whisper, **GigaAM** (strong on Russian) and Parakeet.
- Word-level timings: the word being heard is highlighted during playback.
- Speaker separation on the recorded tracks, with renaming.
- **Two views:** a chat-style transcript, and a table on a time ruler where the
  two sides of the conversation run in parallel columns and overlaps are visible
  at a glance.
- Correct or remove any line in place; re-recognition keeps your corrections.

**Notes and summaries**
- Summaries with local models (a bundled llama.cpp helper using Vulkan on the GPU,
  or the CPU), Ollama, or any OpenAI-compatible endpoint you choose.
- Custom templates, exports to PDF, DOCX, Markdown, text and JSON.

**Library and interface**
- A library organised by client, with recordings filed under each.
- Global search that works on the encrypted archive.
- Russian and English interface, light, dark and AMOLED themes.
- Resizable panels that collapse by dragging, a layout that is remembered, and a
  window frame drawn by the app itself.

## Install

Talkkeeper is built for **Windows 10/11 x64**. Builds of this fork will be
published on this repository's [Releases](https://github.com/leyvanah/Talkkeeper/releases)
page; until then, build from source (below).

The installer is unsigned, so SmartScreen may show **Unknown publisher**.
Updating over an existing installation keeps your data and your keys.

## Your data

| Data | Where |
| --- | --- |
| Database, models, templates, keystore | the app's data folder beside the installation |
| Recordings | `Music\meetily-recordings` by default; change it in **Settings → General** |
| Recording folders | named by an opaque identifier, so a folder listing says nothing about who or what |

## Password and keys

**Settings → Security** puts a password in front of the archive.

- The password never becomes the key the data is encrypted with. It derives a
  key-encryption key through **argon2id** (19 MiB, 2 passes), which wraps a
  separate random data key. Changing the password rewraps 32 bytes; it does not
  rewrite the archive.
- A **recovery code** wraps the same data key in a second envelope. It is
  offered by default and shown exactly once. Without it, a forgotten password
  means nobody can open the archive, including you.
- **Locking wipes the key from memory** rather than covering the window. The
  archive locks when idle — never during a recording — and a recording cannot be
  started while it is locked.
- Guesses are throttled: three are free, then the wait doubles up to five
  minutes, and the count survives closing the app.
- The keys live in `keystore.json` in the app's data folder. It holds no secret
  in the clear; copying it gains an attacker only the right to guess.

**Keeping the key.** **Settings → Security → Save a copy of the key** writes the
keystore wherever you choose. Keep that copy *apart from the recordings*:
together they allow offline guessing of the password, which is exactly why the
app does not keep one beside them for you. When the uninstaller is asked to
delete the app data, it asks separately — naming what will become unreadable —
saves a copy of the key to `Documents\Talkkeeper-key-backup`, and refuses to wipe
anything if that copy could not be written.

### Quick unlock (Windows Hello, optional, off by default)

A Windows Hello prompt can be added as a faster way in. It needs the password to
switch on, and one switch removes it again.

It is a Hello prompt plus a key that **DPAPI** ties to this Windows account —
deliberately not a TPM-bound credential, which would require attaching the
machine to a Microsoft account or a domain. The difference matters:

- **The face check is enforced by the app, not by the encryption.** Code running
  under this Windows account can read the DPAPI blob without the prompt.
- A disk taken on its own is still useless, and so is the keystore on another
  machine or account. A key backup never includes this envelope.
- The password and recovery code are unaffected.

### Recordings on disk

Every track is sealed with **AES-256-GCM** under the archive's data key, in fixed
64 KiB frames, which keeps a recording seekable and lets one cut short by a crash
play up to where it stopped. Each frame is bound to its place in its own file, so
frames cannot be reordered or moved between recordings.

- Setting a password encrypts the recordings that already exist; removing it
  decrypts them first and deletes the key only once every one opens without it.
- No file is rewritten in place: a converted file is verified against the
  original's SHA-256 before it replaces it.
- **Settings → Security** counts what is encrypted and what is not, and offers to
  convert the rest.

### The database

Meeting titles, every line of transcript, word timings, speaker names, summaries,
and the names and notes of clients are sealed value by value with the same key,
with a fresh nonce for each write. The file stays an ordinary SQLite database, so
no patched SQLite is shipped.

- Setting or removing the password converts the database in one transaction; a
  copy is written beside it once, before the first conversion.
- Search runs in the application, because SQL cannot read a sealed column.
- The log files do not print titles, transcript text or names.

**What stays visible:** how many meetings there are, when each was recorded, how
long it ran, which client it is filed under, and how many lines it has. What was
said is not.

**Boundaries.** The key sits in the memory of the running app: cold-boot attacks,
swap files, crash dumps and an operating system you have already unlocked are
outside what this protects against. It defends the disk, not the machine while
you are using it.

## Build

<details>
<summary>Build instructions (Windows)</summary>

Requirements: Rust (stable MSVC), Node.js 22, pnpm, Visual Studio 2022 Build
Tools with C++, CMake and Git. The first build compiles whisper.cpp and ONNX
Runtime bindings and can take half an hour.

```powershell
cd frontend
pnpm install
pnpm run tauri:dev:cpu     # development
pnpm run tauri:build:cpu   # NSIS installer
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md) for implementation details.

</details>

## Credits and license

Talkkeeper is developed by [leyvanah](https://github.com/leyvanah).

It grew out of [Meetily - Actually Free](https://github.com/TylerBuza/Meetily-ActuallyFree)
by Tyler Buza, itself based on [Meetily](https://github.com/Zackriya-Solutions/meetily)
by Zackriya Solutions. Thanks to both projects.

MIT licensed — see [`LICENSE.md`](LICENSE.md). The original copyright notices are
retained.
