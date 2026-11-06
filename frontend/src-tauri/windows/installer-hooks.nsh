; Talkkeeper — NSIS installer dependency hooks
; Tauri copies bundle.resources flat under $INSTDIR (e.g. $INSTDIR\runtime-deps\).
; The NSIS stub is 32-bit: use Sysnative / disable WOW64 redirection for 64-bit tools.

!include "LogicLib.nsh"
!include "x64.nsh"

Var MeetilyBackend
Var MeetilyCudaState
Var MeetilyCudaNotice
!define MEETILY_MIN_NVIDIA_DRIVER "580.00"

!macro Meetily_CheckVcredist
  StrCpy $R9 0
  SetRegView 64
  ReadRegDword $0 HKLM "SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" "Installed"
  ${If} $0 == 1
    StrCpy $R9 1
  ${Else}
    ReadRegDword $0 HKLM "SOFTWARE\WOW6432Node\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" "Installed"
    ${If} $0 == 1
      StrCpy $R9 1
    ${EndIf}
  ${EndIf}
  ${If} $R9 == 0
    ReadRegDword $0 HKLM "SOFTWARE\Microsoft\VisualStudio\2022\VC\Runtimes\x64" "Installed"
    ${If} $0 == 1
      StrCpy $R9 1
    ${EndIf}
  ${EndIf}
  SetRegView lastused
!macroend

!macro Meetily_InstallVcredist
  ; Bundled next to other runtime-deps (NOT under resources\)
  StrCpy $R0 "$INSTDIR\runtime-deps\vc_redist.x64.exe"
  ${If} ${FileExists} "$R0"
    DetailPrint "      Running VC++ redistributable setup…"
    nsExec::ExecToLog '"$R0" /install /quiet /norestart'
    Pop $0
    DetailPrint "      VC++ installer exit code: $0"
    ${If} $0 != 0
    ${AndIf} $0 != 1638
    ${AndIf} $0 != 3010
      MessageBox MB_ICONSTOP|MB_OK "Microsoft Visual C++ Runtime installation failed (code $0). Talkkeeper setup cannot continue."
      Abort
    ${EndIf}
  ${Else}
    MessageBox MB_ICONSTOP|MB_OK "The Microsoft Visual C++ Runtime package is missing. Talkkeeper setup cannot continue."
    Abort
  ${EndIf}
!macroend

!macro Meetily_InstallCudaRuntime
  ; Tauri resource path: $INSTDIR\runtime-deps\*.dll
  StrCpy $R1 "$INSTDIR\runtime-deps"
  ${If} ${FileExists} "$R1\cudart64_13.dll"
    DetailPrint "      Copying CUDA / DirectML libraries next to Talkkeeper…"
    CopyFiles /SILENT "$R1\cudart64_13.dll" "$INSTDIR\"
    ${If} ${FileExists} "$R1\cublas64_13.dll"
      CopyFiles /SILENT "$R1\cublas64_13.dll" "$INSTDIR\"
    ${EndIf}
    ${If} ${FileExists} "$R1\cublasLt64_13.dll"
      CopyFiles /SILENT "$R1\cublasLt64_13.dll" "$INSTDIR\"
    ${EndIf}
    ${If} ${FileExists} "$R1\DirectML.dll"
      CopyFiles /SILENT "$R1\DirectML.dll" "$INSTDIR\"
    ${EndIf}
    DetailPrint "      GPU runtime libraries installed."
  ${ElseIf} ${FileExists} "$INSTDIR\resources\runtime-deps\cudart64_13.dll"
    ; Fallback if resource layout ever nests under resources\
    StrCpy $R1 "$INSTDIR\resources\runtime-deps"
    DetailPrint "      Copying CUDA / DirectML libraries (nested resources path)…"
    CopyFiles /SILENT "$R1\cudart64_13.dll" "$INSTDIR\"
    CopyFiles /SILENT "$R1\cublas64_13.dll" "$INSTDIR\"
    CopyFiles /SILENT "$R1\cublasLt64_13.dll" "$INSTDIR\"
    ${If} ${FileExists} "$R1\DirectML.dll"
      CopyFiles /SILENT "$R1\DirectML.dll" "$INSTDIR\"
    ${EndIf}
    DetailPrint "      GPU runtime libraries installed."
  ${Else}
    DetailPrint "      WARNING: cudart64_13.dll not in bundle — GPU build may fail to start."
  ${EndIf}
!macroend

!macro Meetily_InstallCommonRuntime
  StrCpy $R1 "$INSTDIR\runtime-deps"
  ${If} ${FileExists} "$R1\DirectML.dll"
    CopyFiles /SILENT "$R1\DirectML.dll" "$INSTDIR\"
    DetailPrint "      DirectML / ONNX runtime installed."
  ${Else}
    DetailPrint "      WARNING: DirectML.dll missing from bundle."
  ${EndIf}
!macroend

; Sets $R8=1 if an NVIDIA driver / nvidia-smi is present (WOW64-safe)
!macro Meetily_CheckNvidia
  StrCpy $R8 0
  ${DisableX64FSRedirection}
  ${If} ${FileExists} "$WINDIR\System32\nvidia-smi.exe"
    StrCpy $R8 1
  ${ElseIf} ${FileExists} "$WINDIR\Sysnative\nvidia-smi.exe"
    StrCpy $R8 1
  ${ElseIf} ${FileExists} "$PROGRAMFILES64\NVIDIA Corporation\NVSMI\nvidia-smi.exe"
    StrCpy $R8 1
  ${ElseIf} ${FileExists} "$PROGRAMFILES\NVIDIA Corporation\NVSMI\nvidia-smi.exe"
    StrCpy $R8 1
  ${EndIf}
  ${EnableX64FSRedirection}
  ; Registry fallback (driver package)
  ${If} $R8 == 0
    SetRegView 64
    EnumRegKey $0 HKLM "SOFTWARE\NVIDIA Corporation\GPU" 0
    ${If} $0 != ""
      StrCpy $R8 1
    ${EndIf}
    ReadRegStr $0 HKLM "SOFTWARE\NVIDIA Corporation\Global\NVTweak" "NvCplDaemon"
    ${If} $0 != ""
      StrCpy $R8 1
    ${EndIf}
    SetRegView lastused
  ${EndIf}
!macroend

; Sets $R9=1 when Windows has an NVIDIA display adapter, including fresh
; machines where only Microsoft Basic Display Adapter is installed and
; nvidia-smi is not available yet.
!macro Meetily_CheckNvidiaHardware
  StrCpy $R9 0
  SetRegView 64
  StrCpy $0 0
  ${Do}
    EnumRegKey $1 HKLM "SYSTEM\CurrentControlSet\Enum\PCI" $0
    ${If} $1 == ""
      ${ExitDo}
    ${EndIf}
    StrCpy $2 $1 8
    ${If} $2 == "VEN_10DE"
      StrCpy $3 0
      ${Do}
        EnumRegKey $4 HKLM "SYSTEM\CurrentControlSet\Enum\PCI\$1" $3
        ${If} $4 == ""
          ${ExitDo}
        ${EndIf}
        ReadRegStr $5 HKLM "SYSTEM\CurrentControlSet\Enum\PCI\$1\$4" "ClassGUID"
        ${If} $5 == "{4d36e968-e325-11ce-bfc1-08002be10318}"
          StrCpy $R9 1
          ${ExitDo}
        ${EndIf}
        IntOp $3 $3 + 1
      ${LoopUntil} $R9 == 1
    ${EndIf}
    IntOp $0 $0 + 1
  ${LoopUntil} $R9 == 1
  SetRegView lastused
!macroend

; CUDA 13 requires a modern driver and this universal build targets Turing+
; (compute capability 7.5 and newer). Sets $R8=1 when both checks pass and
; records a reason in $MeetilyCudaState when detected NVIDIA hardware cannot.
!macro Meetily_CheckCudaCapable
  StrCpy $R8 0
  StrCpy $R7 ""
  StrCpy $MeetilyCudaState "no-nvidia"
  !insertmacro Meetily_CheckNvidiaHardware
  ${DisableX64FSRedirection}
  ${If} ${FileExists} "$WINDIR\System32\nvidia-smi.exe"
    StrCpy $R7 "$WINDIR\System32\nvidia-smi.exe"
  ${ElseIf} ${FileExists} "$WINDIR\Sysnative\nvidia-smi.exe"
    StrCpy $R7 "$WINDIR\Sysnative\nvidia-smi.exe"
  ${ElseIf} ${FileExists} "$PROGRAMFILES64\NVIDIA Corporation\NVSMI\nvidia-smi.exe"
    StrCpy $R7 "$PROGRAMFILES64\NVIDIA Corporation\NVSMI\nvidia-smi.exe"
  ${EndIf}

  ${If} $R7 != ""
    ; A working nvidia-smi path itself proves an NVIDIA driver is present even
    ; if Windows' PCI class metadata was unavailable to this process.
    StrCpy $R9 1
    StrCpy $MeetilyCudaState "query-failed"
    nsExec::ExecToStack /TIMEOUT=5000 '"$R7" --id=0 --query-gpu=driver_version --format=csv,noheader,nounits'
    Pop $0
    Pop $1
    ${If} $0 == 0
      ${VersionCompare} "$1" "${MEETILY_MIN_NVIDIA_DRIVER}" $2
      ${If} $2 != 2
        nsExec::ExecToStack /TIMEOUT=5000 '"$R7" --id=0 --query-gpu=compute_cap --format=csv,noheader,nounits'
        Pop $0
        Pop $1
        ${If} $0 == 0
          ${VersionCompare} "$1" "7.5" $2
          ${If} $2 != 2
            StrCpy $R8 1
            StrCpy $MeetilyCudaState "ready"
          ${Else}
            StrCpy $MeetilyCudaState "unsupported-gpu"
          ${EndIf}
        ${EndIf}
      ${Else}
        StrCpy $MeetilyCudaState "old-driver"
      ${EndIf}
    ${EndIf}
  ${ElseIf} $R9 == 1
    StrCpy $MeetilyCudaState "no-driver"
  ${EndIf}
  ${EnableX64FSRedirection}
!macroend

; Sets $R6=1 when the 64-bit Vulkan loader can reach a usable physical device.
; The native x64 probe follows the loader's modern DCH discovery and rejects
; stale loaders with no physical device. The legacy registry scan is retained
; only as a compatibility fallback for packages that predate the probe.
!macro Meetily_CheckVulkan
  StrCpy $R6 0
  ${DisableX64FSRedirection}
  ${If} ${FileExists} "$WINDIR\System32\vulkan-1.dll"
    StrCpy $R6 1
  ${ElseIf} ${FileExists} "$WINDIR\Sysnative\vulkan-1.dll"
    StrCpy $R6 1
  ${EndIf}
  ${If} $R6 == 1
    StrCpy $R6 0
    StrCpy $R4 "$INSTDIR\installer-variants\meetily-vulkan-probe.exe"
    ${If} ${FileExists} "$R4"
      nsExec::ExecToStack /TIMEOUT=15000 '"$R4"'
      Pop $R3
      Pop $R2
      ${If} $R3 == 0
        StrCpy $R6 1
      ${Else}
        DetailPrint "      Vulkan probe found no usable device (code $R3)."
      ${EndIf}
    ${Else}
      DetailPrint "      Vulkan capability probe is missing; checking legacy drivers."
      StrCpy $R5 0
      SetRegView 64
      ${Do}
        EnumRegValue $R4 HKLM "SOFTWARE\Khronos\Vulkan\Drivers" $R5
        ${If} $R4 == ""
          ${ExitDo}
        ${EndIf}
        ReadRegDword $R3 HKLM "SOFTWARE\Khronos\Vulkan\Drivers" "$R4"
        ${If} $R3 == 0
        ${AndIf} ${FileExists} "$R4"
          StrCpy $R6 1
        ${EndIf}
        IntOp $R5 $R5 + 1
      ${LoopUntil} $R6 == 1
      SetRegView lastused
    ${EndIf}
  ${EndIf}
  ${EnableX64FSRedirection}
!macroend

; Pick and install the canonical meetily.exe. Optional command-line override:
;   /BACKEND=cuda | /BACKEND=vulkan | /BACKEND=cpu
!macro Meetily_SelectBackend
  StrCpy $MeetilyBackend ""
  StrCpy $MeetilyCudaState ""
  StrCpy $MeetilyCudaNotice ""
  ClearErrors
  ${GetOptions} $CMDLINE "/BACKEND=" $MeetilyBackend

  ; A standard Tauri bundle has no complete variant set. Keep its compiled
  ; executable instead of accidentally consuming stale local variant files.
  ${IfNot} ${FileExists} "$INSTDIR\installer-variants\universal.marker"
    StrCpy $MeetilyBackend "bundled"
    DetailPrint "      Standard bundle detected — keeping bundled executable."
  ${ElseIf} $MeetilyBackend == "cuda"
    !insertmacro Meetily_CheckCudaCapable
    ${If} $R8 != 1
      DetailPrint "      CUDA override unavailable — falling back to CPU."
      StrCpy $MeetilyBackend "cpu"
    ${EndIf}
  ${ElseIf} $MeetilyBackend == "vulkan"
    !insertmacro Meetily_CheckVulkan
    ${If} $R6 != 1
      DetailPrint "      Vulkan override unavailable — falling back to CPU."
      StrCpy $MeetilyBackend "cpu"
    ${EndIf}
  ${ElseIf} $MeetilyBackend == ""
    !insertmacro Meetily_CheckCudaCapable
    ${If} $R8 == 1
      StrCpy $MeetilyBackend "cuda"
    ${Else}
      !insertmacro Meetily_CheckVulkan
      ${If} $R6 == 1
        StrCpy $MeetilyBackend "vulkan"
      ${Else}
        StrCpy $MeetilyBackend "cpu"
      ${EndIf}
    ${EndIf}
  ${ElseIf} $MeetilyBackend != "cpu"
    DetailPrint "      Unknown backend override — falling back to CPU."
    StrCpy $MeetilyBackend "cpu"
  ${EndIf}

  ; Invalid, unavailable, or missing variants always degrade safely to CPU.
  ${If} $MeetilyBackend == "bundled"
    StrCpy $R5 ""
  ${ElseIf} $MeetilyBackend == "cuda"
    StrCpy $R5 "$INSTDIR\installer-variants\meetily-cuda.exe"
  ${ElseIf} $MeetilyBackend == "vulkan"
    StrCpy $R5 "$INSTDIR\installer-variants\meetily-vulkan.exe"
  ${Else}
    StrCpy $MeetilyBackend "cpu"
    StrCpy $R5 "$INSTDIR\installer-variants\meetily-cpu.exe"
  ${EndIf}
  ${If} $MeetilyBackend != "bundled"
  ${AndIfNot} ${FileExists} "$R5"
    DetailPrint "      Requested backend binary missing — falling back to CPU."
    StrCpy $MeetilyBackend "cpu"
    StrCpy $R5 "$INSTDIR\installer-variants\meetily-cpu.exe"
  ${EndIf}

  ${If} $MeetilyBackend != "bundled"
    DetailPrint "      Selected transcription backend: $MeetilyBackend"
    CopyFiles /SILENT "$R5" "$INSTDIR\meetily.selected.exe"
    ${If} ${FileExists} "$INSTDIR\meetily.selected.exe"
      Delete "$INSTDIR\${MAINBINARYNAME}.exe"
      Rename "$INSTDIR\meetily.selected.exe" "$INSTDIR\${MAINBINARYNAME}.exe"
    ${Else}
      DetailPrint "      Backend replacement failed — keeping bundled executable."
      StrCpy $MeetilyBackend "bundled"
    ${EndIf}
  ${EndIf}

  StrCpy $R3 "$MeetilyBackend"
  ${If} $MeetilyBackend == "vulkan"
    StrCpy $R3 "Vulkan"
  ${ElseIf} $MeetilyBackend == "cpu"
    StrCpy $R3 "CPU"
  ${ElseIf} $MeetilyBackend == "bundled"
    StrCpy $R3 "the bundled backend"
  ${EndIf}
  ${If} $MeetilyCudaState == "no-driver"
    StrCpy $MeetilyCudaNotice "NVIDIA graphics were detected, but a compatible driver was not found. Install NVIDIA driver ${MEETILY_MIN_NVIDIA_DRIVER} or newer, then rerun setup to enable CUDA. Talkkeeper selected $R3 for now."
  ${ElseIf} $MeetilyCudaState == "old-driver"
    StrCpy $MeetilyCudaNotice "The installed NVIDIA driver is too old for Talkkeeper CUDA. Update to NVIDIA driver ${MEETILY_MIN_NVIDIA_DRIVER} or newer, then rerun setup. Talkkeeper selected $R3 for now."
  ${ElseIf} $MeetilyCudaState == "query-failed"
    StrCpy $MeetilyCudaNotice "NVIDIA graphics were detected, but setup could not verify CUDA support. Update or reinstall the NVIDIA driver, then rerun setup. Talkkeeper selected $R3 for now."
  ${EndIf}
  ${If} $MeetilyCudaNotice != ""
    DetailPrint "      NOTICE: $MeetilyCudaNotice"
  ${EndIf}

  CreateDirectory "$INSTDIR\data"
  FileOpen $R4 "$INSTDIR\data\selected-backend.txt" w
  FileWrite $R4 "$MeetilyBackend$\r$\n"
  FileClose $R4
  ${If} $MeetilyProgressToken != ""
    WriteRegStr HKCU "Software\meetily\InstallerProgress\$MeetilyProgressToken" "Backend" "$MeetilyBackend"
    WriteRegStr HKCU "Software\meetily\InstallerProgress\$MeetilyProgressToken" "CudaNotice" "$MeetilyCudaNotice"
  ${EndIf}

  RMDir /r "$INSTDIR\installer-variants"
!macroend

; A line in $INSTDIR\data\install.log for every run that gets as far as copying
; files.
;
; Written because the opposite — the absence of a line — is the answer to a
; question that cost a morning: the owner ran the installer, nothing was
; replaced, and the only way to establish that was comparing file sizes by
; hand. An installer that leaves no trace cannot be diagnosed at all.
!macro Meetily_Stamp Stage
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $9
  CreateDirectory "$INSTDIR\data"
  ; day month year day-of-week hour minute second
  ${GetTime} "" "L" $0 $1 $2 $3 $4 $5 $6
  ClearErrors
  FileOpen $9 "$INSTDIR\data\install.log" a
  ${IfNot} ${Errors}
    FileSeek $9 0 END
    FileWrite $9 "$2-$1-$0 $4:$5:$6  ${Stage}  v${VERSION}  $INSTDIR$\r$\n"
    FileClose $9
  ${EndIf}
  Pop $9
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  SetDetailsPrint both
  !insertmacro Meetily_Stamp "files: begin"
  DetailPrint "────────────────────────────────────────"
  DetailPrint " Talkkeeper"
  ${If} $UpdateMode == 1
    DetailPrint " Updating app files…"
  ${Else}
    DetailPrint " Installing app files…"
  ${EndIf}
  DetailPrint "────────────────────────────────────────"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  SetDetailsPrint both
  DetailPrint ""
  DetailPrint "────────────────────────────────────────"
  DetailPrint " Runtime setup"
  DetailPrint "────────────────────────────────────────"

  DetailPrint "[1/4] Visual C++ 2015–2022 (x64)…"
  !insertmacro MeetilyReportProgress 80
  !insertmacro Meetily_CheckVcredist
  ${If} $R9 == 0
    DetailPrint "      Not found — installing quietly…"
    !insertmacro Meetily_InstallVcredist
  ${Else}
    DetailPrint "      Already installed — skipped."
  ${EndIf}

  DetailPrint "[2/4] Common local AI runtime…"
  !insertmacro MeetilyReportProgress 85
  !insertmacro Meetily_InstallCommonRuntime

  DetailPrint "[3/4] Selecting CUDA, Vulkan, or CPU…"
  !insertmacro MeetilyReportProgress 90
  !insertmacro Meetily_SelectBackend

  DetailPrint "[4/4] Backend runtime…"
  !insertmacro MeetilyReportProgress 94
  ${If} $MeetilyBackend == "cuda"
    !insertmacro Meetily_InstallCudaRuntime
    DetailPrint "      NVIDIA CUDA enabled."
  ${ElseIf} $MeetilyBackend == "bundled"
    ; Harmless for CPU/Vulkan bundles and required for legacy CUDA bundles.
    !insertmacro Meetily_InstallCudaRuntime
    DetailPrint "      Bundled backend runtime installed."
  ${ElseIf} $MeetilyBackend == "vulkan"
    DetailPrint "      Vulkan enabled (AMD / Intel / compatible NVIDIA)."
  ${Else}
    DetailPrint "      CPU mode enabled — maximum compatibility."
  ${EndIf}

  DetailPrint ""
  DetailPrint "Runtime setup finished."
  !insertmacro Meetily_Stamp "install: finished"
  !insertmacro MeetilyReportProgress 98
  DetailPrint "────────────────────────────────────────"
!macroend

; Waits until Talkkeeper is no longer running, and never closes it itself.
;
; A forced kill ends a recording without stopping it, so the meeting is not
; saved and the tail of the audio can be lost. Instead the owner is asked to
; stop the recording and quit from the tray, then to retry; Cancel leaves
; everything as it was. A silent run cannot ask, so it gives up with an error
; and changes nothing. This replaces both Tauri's CheckIfAppIsRunning, which
; kills on OK (and without asking in silent and passive runs), and the
; `taskkill /F` this file used to run before uninstalling.
;
; Inside a section, Abort stops the install or uninstall; inside a page's leave
; function it keeps the owner on that page.
!macro Talkkeeper_WaitForAppToClose
  !define TkWaitID ${__LINE__}
  tk_wait_check_${TkWaitID}:
  !if "${INSTALLMODE}" == "currentUser"
    nsis_tauri_utils::FindProcessCurrentUser "${MAINBINARYNAME}.exe"
  !else
    nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
  !endif
  Pop $R0
  ${If} $R0 = 0
    IfSilent 0 tk_wait_ask_${TkWaitID}
      System::Call 'kernel32::AttachConsole(i -1)i.r0'
      ${If} $0 <> 0
        System::Call 'kernel32::GetStdHandle(i -11)i.r0'
        System::call 'kernel32::SetConsoleTextAttribute(i r0, i 0x0004)' ; red
        FileWrite $0 "$(tkAppRunningSilent)$\n"
      ${EndIf}
      Abort
    tk_wait_ask_${TkWaitID}:
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(tkAppRunning)" IDRETRY tk_wait_check_${TkWaitID}
      Abort
  ${EndIf}
  !undef TkWaitID
!macroend

; Where the recordings live, found before anything is deleted, so that the
; uninstaller can say what it leaves behind.
;
; Recordings are the one thing kept outside $INSTDIR (see
; audio/recording_preferences.rs): `<Music>\meetily-recordings` unless the owner
; picked another folder, which the app keeps as `save_folder` in its
; recording_preferences.json under $APPDATA\<identifier> — the very folder the
; "delete app data" box wipes, so it has to be read first. The uninstaller
; deletes none of it; it only names the folder and says whether it is sealed.
Var TkRecordingsDir     ; the folder, or the default one if the setting is unreadable
Var TkRecordingsLeft    ; 1 when that folder holds anything
Var TkRecordingsSealed  ; 1 when a keystore exists, so recordings made under it are encrypted

; Body of un.Talkkeeper_FindRecordings, defined in installer.nsi where
; ${BUNDLEID} is known.
;
; The store is UTF-8 and the path may hold non-Latin letters (a Cyrillic user
; name is common), which FileRead would mangle through the ANSI code page. So
; the file is read raw, `"save_folder"` is found in the bytes, and only that
; stretch is converted from UTF-8 — one line, well under NSIS_MAX_STRLEN, even
; when the whole file is not.
!macro Talkkeeper_FindRecordingsBody
  Push $R0
  Push $R1
  Push $R2
  Push $R3
  Push $R4
  Push $R5
  Push $R6
  Push $R7
  Push $R8
  Push $R9
  StrCpy $TkRecordingsDir ""
  StrCpy $TkRecordingsLeft 0
  StrCpy $TkRecordingsSealed 0
  ${If} ${FileExists} "$INSTDIR\data\keystore.json"
    StrCpy $TkRecordingsSealed 1
  ${EndIf}

  StrCpy $R9 "$APPDATA\${BUNDLEID}\recording_preferences.json"
  ${If} ${FileExists} "$R9"
    ; GENERIC_READ, share read/write/delete, OPEN_EXISTING
    System::Call 'kernel32::CreateFileW(w R9, i 0x80000000, i 7, p 0, i 3, i 0, p 0) p.R8'
    ${If} $R8 <> -1
      System::Call 'kernel32::GetFileSize(p R8, p 0) i.R7'
      ${If} $R7 > 0
      ${AndIf} $R7 < 1048576
        IntOp $R6 $R7 + 1
        System::Alloc $R6 ; zero-filled, so the bytes end in a NUL for StrStrA
        Pop $R6
        System::Call 'kernel32::ReadFile(p R8, p R6, i R7, *i .R5, p 0) i.R4'
        ${If} $R4 <> 0
          System::Call `shlwapi::StrStrA(p R6, m '"save_folder"') p.R4`
          ${If} $R4 <> 0
            IntOp $R3 $R6 + $R5
            IntOp $R3 $R3 - $R4
            ${If} $R3 > 1000
              StrCpy $R3 1000
            ${EndIf}
            System::Call 'kernel32::MultiByteToWideChar(i 65001, i 0, p R4, i R3, w .R2, i 1023) i.R1'
            StrCpy $R2 $R2 $R1
            ; $R2 = "save_folder": "C:\\Users\\…\\meetily-recordings", …
            StrLen $R0 $R2
            StrCpy $R1 13 ; past "save_folder"
            StrCpy $R5 "" ; the path, unescaped
            StrCpy $R7 0  ; 0 before the opening quote, 1 inside, 2 done, 3 gave up
            ${Do}
              ${If} $R1 >= $R0
                StrCpy $R7 3
                ${ExitDo}
              ${EndIf}
              StrCpy $R3 $R2 1 $R1
              IntOp $R1 $R1 + 1
              ${If} $R7 = 0
                ${If} $R3 == '"'
                  StrCpy $R7 1
                ${EndIf}
              ${ElseIf} $R3 == '"'
                StrCpy $R7 2
                ${ExitDo}
              ${ElseIf} $R3 == "\"
                ; serde_json escapes only \ and " in a Windows path; anything
                ; else is not a path this parser should guess at
                StrCpy $R3 $R2 1 $R1
                IntOp $R1 $R1 + 1
                ${If} $R3 == "\"
                ${OrIf} $R3 == '"'
                ${OrIf} $R3 == "/"
                  StrCpy $R5 "$R5$R3"
                ${Else}
                  StrCpy $R7 3
                  ${ExitDo}
                ${EndIf}
              ${Else}
                StrCpy $R5 "$R5$R3"
              ${EndIf}
            ${Loop}
            ${If} $R7 = 2
            ${AndIf} $R5 != ""
              StrCpy $TkRecordingsDir $R5
            ${EndIf}
          ${EndIf}
        ${EndIf}
        System::Free $R6
      ${EndIf}
      System::Call 'kernel32::CloseHandle(p R8)'
    ${EndIf}
  ${EndIf}

  ${If} $TkRecordingsDir == ""
    ; The app's default: the Music folder, else Documents
    ${If} $MUSIC != ""
      StrCpy $TkRecordingsDir "$MUSIC\meetily-recordings"
    ${Else}
      StrCpy $TkRecordingsDir "$DOCUMENTS\meetily-recordings"
    ${EndIf}
  ${EndIf}

  ClearErrors
  FindFirst $R0 $R1 "$TkRecordingsDir\*"
  ${IfNot} ${Errors}
    ${Do}
      ${If} $R1 == ""
        ${ExitDo}
      ${EndIf}
      ${If} $R1 != "."
      ${AndIf} $R1 != ".."
        StrCpy $TkRecordingsLeft 1
        ${ExitDo}
      ${EndIf}
      FindNext $R0 $R1
    ${Loop}
    FindClose $R0
  ${EndIf}

  Pop $R9
  Pop $R8
  Pop $R7
  Pop $R6
  Pop $R5
  Pop $R4
  Pop $R3
  Pop $R2
  Pop $R1
  Pop $R0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Call un.Talkkeeper_FindRecordings
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Say what stays on disk: the box never deleted recordings, and without a
  ; password they are plain audio anyone with the computer can play.
  ${If} $UpdateMode <> 1
  ${AndIf} $TkRecordingsLeft = 1
    DetailPrint "Recordings left in $TkRecordingsDir"
    ${If} $TkRecordingsSealed = 1
      StrCpy $R0 "encrypted where made under a password"
    ${Else}
      StrCpy $R0 "not encrypted"
    ${EndIf}
    ; A silent or passive run has no one to tell; the log stays when the data
    ; folder does, which it always does without the confirm page.
    ${If} ${FileExists} "$INSTDIR\data\*.*"
      !insertmacro Meetily_Stamp "uninstall: recordings left in $TkRecordingsDir ($R0)"
    ${EndIf}
    ${IfNot} ${Silent}
    ${AndIf} $PassiveMode <> 1
      ${If} $TkRecordingsSealed = 1
        MessageBox MB_ICONINFORMATION|MB_OK "$(tkRecordingsLeftSealed)"
      ${Else}
        MessageBox MB_ICONEXCLAMATION|MB_OK "$(tkRecordingsLeftOpen)"
      ${EndIf}
    ${EndIf}
  ${EndIf}
  DetailPrint "Talkkeeper was removed from this user profile."
!macroend
