# Speech for the audio bench, synthesised locally.
#
# The bench needs speech whose words are known exactly, so a transcription can
# be scored against them instead of judged by ear. Windows' own synthesiser is
# used rather than a recording: it is offline, it is on every machine this
# project targets, and the text is whatever we pass it.
#
# Output is 48 kHz mono 16-bit, which is the rate the capture pipeline runs at,
# so nothing is resampled on the way in and the bench measures the pipeline
# rather than a converter.
#
# Files land under target\audio-bench\ and are never committed: the repository
# is public and audio files are blocked by the pre-commit hook on purpose.

param(
    [string]$OutputDir = "$PSScriptRoot\..\..\..\target\audio-bench"
)

Add-Type -AssemblyName System.Speech

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

# Deliberately dull, technical sentences: this repository is public, and test
# material that reads like a real conversation says more than it should.
$scripts = @{
    'speaker-ru' = @(
        'Первый пункт повестки — сроки поставки оборудования.',
        'Смета пересчитана с учётом новой ставки аренды.',
        'Отчёт за квартал будет готов к пятнице.',
        'Остаток бюджета переносим на следующий период.',
        'Протокол разошлю всем участникам сегодня вечером.'
    )
    'farend-ru' = @(
        'Напоминаю, что склад работает до шести часов.',
        'Договор подписан обеими сторонами во вторник.',
        'Доставка занимает от трёх до пяти рабочих дней.',
        'Счёт выставлен, оплата ожидается до конца месяца.'
    )
}

$format = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(
    48000,
    [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen,
    [System.Speech.AudioFormat.AudioChannel]::Mono
)

foreach ($name in $scripts.Keys) {
    $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
    try {
        $synth.SelectVoice('Microsoft Irina Desktop')
        # The two speakers have to be distinguishable, so the far end is given a
        # different rate. One installed Russian voice means pitch cannot differ.
        if ($name -eq 'farend-ru') { $synth.Rate = -2 } else { $synth.Rate = 0 }

        $wav = Join-Path $OutputDir "$name.wav"
        $txt = Join-Path $OutputDir "$name.txt"

        $synth.SetOutputToWaveFile($wav, $format)
        foreach ($line in $scripts[$name]) {
            $synth.Speak($line)
            # A beat between sentences, so a lost word shows as a lost word
            # rather than as two sentences running together.
            $synth.Speak([System.Speech.Synthesis.PromptBuilder]::new())
            Start-Sleep -Milliseconds 1
        }
        $synth.SetOutputToNull()

        $scripts[$name] -join ' ' | Set-Content -Path $txt -Encoding utf8
        $size = (Get-Item $wav).Length
        Write-Output "$name.wav  $([math]::Round($size / 1024)) KB  $($scripts[$name].Count) sentences"
    }
    finally {
        $synth.Dispose()
    }
}

Write-Output "Written to $OutputDir"
