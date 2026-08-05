[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ProbePath,

    [Parameter(Mandatory = $true)]
    [string]$OutputRoot,

    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$SourceCommit,

    [string]$FfmpegPath = "",
    [string]$FfplayPath = "",
    [string]$FfprobePath = "",
    [ValidateRange(1000, 30000)]
    [int]$DurationMs = 6000,
    [ValidateRange(2, 30)]
    [int]$ToneDurationSeconds = 8,
    [ValidateRange(20, 20000)]
    [int]$ToneFrequencyHz = 997
)

$ErrorActionPreference = "Stop"

function Resolve-NativeTool {
    param(
        [string]$ConfiguredPath,
        [string]$CommandName
    )

    if ($ConfiguredPath) {
        return (Resolve-Path -LiteralPath $ConfiguredPath).Path
    }
    $command = Get-Command $CommandName -ErrorAction Stop
    return $command.Source
}

$resolvedProbe = (Resolve-Path -LiteralPath $ProbePath).Path
$ffmpeg = Resolve-NativeTool -ConfiguredPath $FfmpegPath -CommandName "ffmpeg.exe"
$ffplay = Resolve-NativeTool -ConfiguredPath $FfplayPath -CommandName "ffplay.exe"
$ffprobe = Resolve-NativeTool -ConfiguredPath $FfprobePath -CommandName "ffprobe.exe"
$resolvedOutput = [System.IO.Path]::GetFullPath($OutputRoot)
if (Test-Path -LiteralPath $resolvedOutput) {
    throw "OutputRoot already exists: $resolvedOutput"
}

New-Item -ItemType Directory -Path $resolvedOutput | Out-Null
$probe = Join-Path $resolvedOutput "windows_loopback_probe.exe"
$tone = Join-Path $resolvedOutput "qualification-tone.wav"
$capture = Join-Path $resolvedOutput "system-loopback.wav"
$stdout = Join-Path $resolvedOutput "probe.stdout.log"
$stderr = Join-Path $resolvedOutput "probe.stderr.log"
$receiptPath = Join-Path $resolvedOutput "receipt.json"
Copy-Item -LiteralPath $resolvedProbe -Destination $probe

& $ffmpeg -hide_banner -loglevel error -f lavfi `
    -i "sine=frequency=${ToneFrequencyHz}:duration=${ToneDurationSeconds}:sample_rate=48000" `
    -af "pan=stereo|c0=c0|c1=c0" -c:a pcm_s16le -n $tone
if ($LASTEXITCODE -ne 0) {
    throw "tone generation failed with exit $LASTEXITCODE"
}

$env:SHELLX_CAPTURE_TRACE = "1"
$probeProcess = $null
$toneProcess = $null
try {
    $probeStart = New-Object System.Diagnostics.ProcessStartInfo
    $probeStart.FileName = $probe
    $probeStart.Arguments = "`"$capture`" $DurationMs"
    $probeStart.UseShellExecute = $false
    $probeStart.CreateNoWindow = $true
    $probeStart.RedirectStandardOutput = $true
    $probeStart.RedirectStandardError = $true
    $probeProcess = New-Object System.Diagnostics.Process
    $probeProcess.StartInfo = $probeStart
    if (-not $probeProcess.Start()) {
        throw "loopback probe did not start"
    }

    Start-Sleep -Milliseconds 1000
    $toneStart = New-Object System.Diagnostics.ProcessStartInfo
    $toneStart.FileName = $ffplay
    $toneStart.Arguments = "-nodisp -autoexit -loglevel error `"$tone`""
    $toneStart.UseShellExecute = $false
    $toneStart.CreateNoWindow = $true
    $toneProcess = New-Object System.Diagnostics.Process
    $toneProcess.StartInfo = $toneStart
    if (-not $toneProcess.Start()) {
        throw "qualification tone playback did not start"
    }

    if (-not $probeProcess.WaitForExit($DurationMs + 15000)) {
        throw "loopback probe timed out"
    }
    $probeProcess.WaitForExit()
    $probeExit = $probeProcess.ExitCode
    $probeStdout = $probeProcess.StandardOutput.ReadToEnd()
    $probeStderr = $probeProcess.StandardError.ReadToEnd()
}
finally {
    if ($null -ne $toneProcess -and -not $toneProcess.HasExited) {
        $toneProcess.Kill()
        $toneProcess.WaitForExit()
    }
    if ($null -ne $probeProcess -and -not $probeProcess.HasExited) {
        $probeProcess.Kill()
        $probeProcess.WaitForExit()
    }
}

$probeStdout | Set-Content -LiteralPath $stdout -Encoding utf8
$probeStderr | Set-Content -LiteralPath $stderr -Encoding utf8
if ($probeExit -ne 0) {
    throw "loopback probe failed with exit $probeExit`n$probeStderr"
}
if (-not (Test-Path -LiteralPath $capture)) {
    throw "loopback probe produced no capture file"
}

$probeData = (& $ffprobe -v error -show_streams -show_format -of json $capture | Out-String) |
    ConvertFrom-Json
if ($LASTEXITCODE -ne 0) {
    throw "ffprobe failed with exit $LASTEXITCODE"
}

$previousErrorPreference = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    $volumeLog = (& $ffmpeg -hide_banner -nostats -i $capture `
        -af volumedetect -f null NUL 2>&1 | Out-String)
    $volumeExit = $LASTEXITCODE
}
finally {
    $ErrorActionPreference = $previousErrorPreference
}
if ($volumeExit -ne 0) {
    throw "volume analysis failed with exit $volumeExit"
}

$meanMatch = [regex]::Match($volumeLog, "mean_volume:\s*(-?[0-9.]+)\s*dB")
$maxMatch = [regex]::Match($volumeLog, "max_volume:\s*(-?[0-9.]+)\s*dB")
$stream = $probeData.streams[0]
$duration = [double]$probeData.format.duration
$meanDb = if ($meanMatch.Success) { [double]$meanMatch.Groups[1].Value } else { -999.0 }
$maxDb = if ($maxMatch.Success) { [double]$maxMatch.Groups[1].Value } else { -999.0 }
$expectedSeconds = $DurationMs / 1000.0
$checks = [ordered]@{
    probeExit = ($probeExit -eq 0)
    codecPcmS16le = ($stream.codec_name -eq "pcm_s16le")
    sampleRate48k = ([int]$stream.sample_rate -eq 48000)
    stereo = ([int]$stream.channels -eq 2)
    duration = ($duration -ge ($expectedSeconds - 0.5) -and $duration -le ($expectedSeconds + 0.5))
    audible = ($maxDb -gt -60.0 -and $meanDb -gt -70.0)
}
$ok = -not ($checks.Values -contains $false)
$receipt = [ordered]@{
    schema = "shellx-cut/windows-process-loopback-proof@1"
    recordedAt = (Get-Date).ToUniversalTime().ToString("o")
    ok = $ok
    version = $Version
    sourceCommit = $SourceCommit
    runRoot = $resolvedOutput
    probe = [ordered]@{
        exitCode = $probeExit
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $probe).Hash.ToLowerInvariant()
        stdout = $probeStdout.Trim()
        stderr = $probeStderr.Trim()
    }
    tone = [ordered]@{
        frequencyHz = $ToneFrequencyHz
        durationSeconds = $ToneDurationSeconds
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $tone).Hash.ToLowerInvariant()
    }
    capture = [ordered]@{
        path = $capture
        bytes = (Get-Item -LiteralPath $capture).Length
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $capture).Hash.ToLowerInvariant()
        codec = $stream.codec_name
        sampleRate = [int]$stream.sample_rate
        channels = [int]$stream.channels
        durationSeconds = $duration
        meanVolumeDb = $meanDb
        maxVolumeDb = $maxDb
    }
    checks = $checks
}
$receiptJson = $receipt | ConvertTo-Json -Depth 8
$receiptJson | Set-Content -LiteralPath $receiptPath -Encoding utf8
$receipt | ConvertTo-Json -Depth 8 -Compress

if (-not $ok) {
    throw "loopback receipt checks failed; see $receiptPath"
}
