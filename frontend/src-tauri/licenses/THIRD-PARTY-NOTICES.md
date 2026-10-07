# Third-party components in the Talkkeeper installer

Talkkeeper itself is MIT-licensed (`LICENSE.md` in the source repository).
The installer also carries the components below, each under its own license.
The installed copy of this file is in `licenses\` in the program folder.

| Component | Where | License |
|---|---|---|
| FFmpeg 8.0.1 (gyan.dev essentials build) | `ffmpeg.exe` | GPL-3.0 — see `licenses\ffmpeg\` |
| llama.cpp / ggml | `llama-helper.exe`, `llama-helper-cpu.exe` | MIT |
| whisper.cpp / ggml | compiled into the app | MIT |
| ONNX Runtime | compiled into the app | MIT |
| pyannote segmentation-3.0 | `resources\diarization\segmentation-3.0-fp16.onnx` | MIT |
| WeSpeaker ResNet34 (VoxCeleb, large-margin fine-tuned) | `resources\diarization\wespeaker-resnet34-LM.onnx` | CC BY 4.0 |
| VBx x-vector LDA transform | `resources\diarization\xvec_transform.npz` | CC BY 4.0 |
| Microsoft Visual C++ Redistributable | `runtime-deps\vc_redist.x64.exe` | Microsoft Software License Terms (redistributable, unmodified) |
| Microsoft DirectML | `runtime-deps\DirectML.dll` | Microsoft Software License Terms (redistributable, unmodified) |
| NVIDIA CUDA runtime and cuBLAS (CUDA build only) | `runtime-deps\cudart64_13.dll`, `cublas64_13.dll`, `cublasLt64_13.dll` | NVIDIA CUDA Toolkit EULA, redistributable components (unmodified) |

The app and the helper are also built from Rust crates, and the window from
npm packages, under permissive licenses (MIT, Apache-2.0, BSD and similar);
their notices are in the respective packages' sources.

Speech-recognition and summary models that you download from inside the app
(Whisper, Parakeet, GigaAM, the built-in summary models) are not in the
installer; each is under the license published with it on Hugging Face.

---

## FFmpeg

FFmpeg is free software under the GNU General Public License, version 3. The
full license, the description of this exact build and where to get its
source code are in `licenses\ffmpeg\` (`LICENSE.txt`, `BUILD.txt`,
`SOURCE.md`). Talkkeeper runs the unmodified `ffmpeg.exe` as a separate
program.

## llama.cpp, whisper.cpp, ggml

<https://github.com/ggml-org/llama.cpp>, <https://github.com/ggml-org/whisper.cpp>

```
MIT License

Copyright (c) 2023-2026 The ggml authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## ONNX Runtime

<https://github.com/microsoft/onnxruntime> — MIT License,
Copyright (c) Microsoft Corporation. The permission notice is the MIT text
above.

## pyannote segmentation-3.0

<https://huggingface.co/pyannote/segmentation-3.0> — MIT License,
Copyright (c) CNRS (pyannote.audio, Hervé Bredin et al.). The permission
notice is the MIT text above. Redistributed converted to ONNX with 16-bit
weights.

## WeSpeaker ResNet34 and the VBx transform

- Speaker-embedding model: WeSpeaker ResNet34 trained on VoxCeleb with
  large-margin fine-tuning, by the WeNet community
  (<https://github.com/wenet-e2e/wespeaker>), as published by pyannote at
  <https://huggingface.co/pyannote/wespeaker-voxceleb-resnet34-LM>.
- x-vector LDA transform: from VBx (Brno University of Technology,
  <https://github.com/BUTSpeechFIT/VBx>), as published by pyannote at
  <https://huggingface.co/pyannote/speaker-diarization-community-1>
  (`plda/xvec_transform.npz`).

Both are licensed under the Creative Commons Attribution 4.0 International
License (<https://creativecommons.org/licenses/by/4.0/>). The embedding model
is redistributed exported to ONNX; the transform is unmodified.
