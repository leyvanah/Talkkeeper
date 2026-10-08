# FFmpeg in Talkkeeper

Talkkeeper runs `ffmpeg.exe`, installed next to the app, to read and write
audio. It is an unmodified third-party program, not part of Talkkeeper's own
code, and is licensed separately from it.

| | |
|---|---|
| Program | FFmpeg 8.0.1, "essentials" static build for 64-bit Windows by gyan.dev |
| License | GNU General Public License, version 3 — [`LICENSE.txt`](LICENSE.txt) |
| Build description | [`BUILD.txt`](BUILD.txt): version, the FFmpeg source commit, the build configuration and every library linked in, with versions |
| Obtained from | <https://github.com/GyanD/codexffmpeg/releases/tag/8.0.1>, file `ffmpeg-8.0.1-essentials_build.zip` (SHA-256 `e2aaeaa0fdbc397d4794828086424d4aaa2102cef1fb6874f6ffd29c0b88b673`) |

`LICENSE.txt` and `BUILD.txt` are the `LICENSE` and `README.txt` of that
archive, taken out of it at build time and checked against pinned SHA-256
values (`frontend/src-tauri/build/ffmpeg.rs`).

## Source code

- FFmpeg at the commit this build was made from:
  <https://github.com/FFmpeg/FFmpeg/commit/894da5ca7d>; the 8.0.1 release
  archive: <https://ffmpeg.org/releases/ffmpeg-8.0.1.tar.xz>.
- The external libraries linked into the build are listed in `BUILD.txt` with
  their versions; each is available from its project.
- How gyan.dev builds it: <https://www.gyan.dev/ffmpeg/builds/>.

Builds for macOS and Linux use FFmpeg archives from
<https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/tag/0.0.1>,
which carry no license file of their own; FFmpeg's license terms are at
<https://ffmpeg.org/legal.html>.
