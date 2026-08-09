$ErrorActionPreference = "Stop"

$probeRoot = Join-Path ([IO.Path]::GetTempPath()) ("shellx-cut-updater-handoff-" + [guid]::NewGuid().ToString("N"))
$installDir = Join-Path $probeRoot "installed"
$decoyDir = Join-Path $probeRoot "other"
$ownedProcess = $null
$decoyProcess = $null

try {
    New-Item -ItemType Directory -Path $installDir, $decoyDir | Out-Null
    $systemPing = Join-Path $env:WINDIR "System32\ping.exe"
    $ownedExe = Join-Path $installDir "cutd.exe"
    $decoyExe = Join-Path $decoyDir "cutd.exe"
    Copy-Item -LiteralPath $systemPing -Destination $ownedExe
    Copy-Item -LiteralPath $systemPing -Destination $decoyExe

    $ownedProcess = Start-Process -FilePath $ownedExe -ArgumentList "-t", "127.0.0.1" -WindowStyle Hidden -PassThru
    $decoyProcess = Start-Process -FilePath $decoyExe -ArgumentList "-t", "127.0.0.1" -WindowStyle Hidden -PassThru

    $target = [IO.Path]::GetFullPath($ownedExe)
    $owned = @()
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        $owned = @(
            Get-CimInstance Win32_Process -Filter "Name='cutd.exe'" -ErrorAction Stop |
                Where-Object {
                    $_.ExecutablePath -and
                    [IO.Path]::GetFullPath($_.ExecutablePath).Equals(
                        $target,
                        [StringComparison]::OrdinalIgnoreCase
                    )
                }
        )
        if ($owned.Count -eq 1) {
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if ($owned.Count -ne 1) {
        throw "Expected one process at the install path, found $($owned.Count)."
    }

    foreach ($process in $owned) {
        Stop-Process -Id $process.ProcessId -Force -ErrorAction Stop
    }
    Start-Sleep -Milliseconds 500

    $remaining = @(
        Get-CimInstance Win32_Process -Filter "Name='cutd.exe'" -ErrorAction Stop |
            Where-Object {
                $_.ExecutablePath -and
                [IO.Path]::GetFullPath($_.ExecutablePath).Equals(
                    $target,
                    [StringComparison]::OrdinalIgnoreCase
                )
            }
    )
    if ($remaining.Count -ne 0) {
        throw "The install-path process remained alive after the updater handoff."
    }
    if ($null -eq (Get-Process -Id $decoyProcess.Id -ErrorAction SilentlyContinue)) {
        throw "The updater handoff stopped an identically named process outside the install path."
    }

    Write-Output "PASS windows-updater-process-handoff"
}
finally {
    foreach ($process in @($ownedProcess, $decoyProcess)) {
        if ($null -ne $process -and $null -ne (Get-Process -Id $process.Id -ErrorAction SilentlyContinue)) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
    }
    if (Test-Path -LiteralPath $probeRoot) {
        Remove-Item -LiteralPath $probeRoot -Recurse -Force
    }
}
