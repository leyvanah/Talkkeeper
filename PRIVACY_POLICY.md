# Privacy Policy — Talkkeeper

**Short version:** Talkkeeper records, transcribes and summarises on your
computer. It has no account, no analytics and no telemetry. By default it
does not send what was said anywhere: "This computer only" mode is on, and
it blocks every request carrying meeting content to another machine. The
text of a meeting leaves your computer only if you choose a cloud model or a
server on another machine **and** turn that mode off.

This policy describes the app as it is built from this repository. It covers
the app only, not the services you may choose to connect it to.

## What the app collects about you

Nothing. There is no account, no sign-in, no usage statistics, no crash
reporting that sends itself, and no automatic update check. The authors of
Talkkeeper receive no data from the app.

## What stays on your computer

- **Recordings** (audio files) in the recordings folder chosen in Settings.
- **The archive database**: meeting titles, transcripts, summaries, notes,
  clients and people, speaker names, the speech-recognition vocabulary and
  the settings, including any API keys you enter.
- **Models** you download (speech recognition, speaker separation, the
  built-in summary model).
- **The application log** (`logs/` in the data folder). It is written to hold
  states, counts, durations and identifiers, not what was said; it may hold
  file paths on your computer. It is not encrypted.

## Encryption at rest

Encryption is optional and starts when you set an archive password
(Settings → Protection).

- With a password, recordings are encrypted on disk, and the database values
  that hold meeting content are sealed individually with AES-256-GCM:
  titles, transcripts, summaries, notes, names of clients, people and
  speakers, the recognition vocabulary and API keys. The data key is
  wrapped by a key derived from your password (Argon2id).
- A **recovery code**, if you create one, opens the archive without the
  password. Anyone who has it has the same access as the password.
- **Windows Hello quick unlock**, if you turn it on, keeps a copy of the key
  protected by Windows for this user account.
- The archive locks after the idle time you choose, or when you lock it.
  While it is locked, the app refuses to write meeting content rather than
  store it unencrypted.
- **Without a password, everything above is stored unencrypted.** Values
  written before the password was set are converted when it is set; the
  Protection screen shows anything still unconverted.
- Not covered by encryption: the log file, the settings that say which
  models and providers are used, and anything you take out of the app (see
  below).

## When data leaves your computer

**Meeting content** (audio, transcript text, names) leaves your computer only
in these cases, each of which you choose:

1. **A cloud summary provider** — Claude (Anthropic), OpenAI, Groq or
   OpenRouter. The text to summarise, or your question to the assistant, is
   sent directly to that provider with your own API key. What happens to it
   there is governed by that provider's terms. Before sending, names and
   contact details found in the text are replaced with placeholders and put
   back in the answer (on by default, Settings → Protection); this is a best
   effort, not a guarantee.
2. **Ollama or a custom OpenAI-compatible server on another machine**, if you
   enter its address. On this computer (`localhost`) nothing leaves.
3. **An external speech-recognition service on another machine**, if you
   enter its address. Audio is sent to it.

All three are blocked while **"This computer only"** is on, which is the
default: the request is refused before it is built.

**Other network use** sends nothing from your archive:

- **Model downloads**, when you ask for them, from Hugging Face and GitHub.
  Those sites receive the usual request data (IP address, request headers).
- **Ollama** downloads its models from its own registry when the app asks it
  to install one.
- **Lists of available models** from a cloud provider, when you open its
  settings with your key entered.
- **Links** in the app (source code, releases, drivers) open in your browser.

## What you take out of the app

These leave the encrypted archive by your action and are not protected by it:

- **Export** writes an unencrypted file where you choose.
- **Copying** a transcript or summary puts it on the Windows clipboard. If
  clipboard history or clipboard sync is on in Windows, Windows keeps or
  syncs it.
- **Importing** an audio file encrypts the copy in the archive; the original
  file stays where it was.
- **Windows notifications** are shown and kept by Windows. They contain no
  transcript or summary text.

## Crash reports

After an unexpected exit the app can save a crash report as a ZIP file on
your computer. It sends nothing; sharing the file is up to you. The report
contains the crash type and time, app version, acceleration backend, general
OS information, rounded CPU and memory figures and, when available, a source
file and line inside the app. It contains no audio, transcripts, summaries,
meeting titles, database, settings, credentials, or user, host or device
names.

## Deleting data

Deleting a meeting removes its database records and its recording folder; if
the folder cannot be removed (for example, a file is open in another
program), the app says so.

The uninstaller offers to delete the app's data as well: database, settings,
models and log. With a password set, it first copies the archive's key file
to `Documents\Talkkeeper-key-backup` and says so, because the recordings are
kept outside the app's folder and cannot be opened without that key. The
uninstaller does not delete the recordings folder or that key copy; delete
them yourself to remove everything.

## Changes and contact

Changes to this policy are made in this repository and are visible in its
history.

To report a privacy or security problem, use GitHub's private vulnerability
reporting: open the repository's **Security** tab and choose **Report a
vulnerability**
(<https://github.com/leyvanah/Talkkeeper/security/advisories/new>). Only the
maintainers see the report.
