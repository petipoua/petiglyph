param(
    [string]$PetiglyphPath = "",
    [switch]$SkipCliChecks,
    [switch]$SkipClipboardChecks,
    [switch]$NoReadbackVerify
)

$ErrorActionPreference = "Stop"

function Write-Info($msg) { Write-Host "[INFO] $msg" }
function Write-Ok($msg) { Write-Host "[OK] $msg" }
function Fail($msg) {
    Write-Host "[FAIL] $msg" -ForegroundColor Red
    exit 1
}

function Invoke-Petiglyph {
    param([string[]]$CliArgs)

    if ($PetiglyphPath -ne "") {
        $output = & $PetiglyphPath @CliArgs 2>&1
        $exitCode = $LASTEXITCODE
        return [PSCustomObject]@{
            ExitCode = $exitCode
            Output = ($output -join [Environment]::NewLine)
        }
    }

    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    Push-Location $repoRoot
    try {
        $output = & cargo run --quiet -- @CliArgs 2>&1
        $exitCode = $LASTEXITCODE
        return [PSCustomObject]@{
            ExitCode = $exitCode
            Output = ($output -join [Environment]::NewLine)
        }
    } finally {
        Pop-Location
    }
}

function Assert-Success($result, $label) {
    if ($result.ExitCode -ne 0) {
        Write-Host $result.Output
        Fail "$label failed with exit code $($result.ExitCode)"
    }
    Write-Ok $label
}

function Invoke-PetiglyphTuiNoTty {
    if ($PetiglyphPath -ne "") {
        $output = $null | & $PetiglyphPath tui 2>&1
        $exitCode = $LASTEXITCODE
        return [PSCustomObject]@{
            ExitCode = $exitCode
            Output = ($output -join [Environment]::NewLine)
        }
    }

    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
    Push-Location $repoRoot
    try {
        $output = $null | & cargo run --quiet -- tui 2>&1
        $exitCode = $LASTEXITCODE
        return [PSCustomObject]@{
            ExitCode = $exitCode
            Output = ($output -join [Environment]::NewLine)
        }
    } finally {
        Pop-Location
    }
}

if (-not $SkipCliChecks) {
    Write-Info "running petiglyph CLI smoke checks"
    Assert-Success (Invoke-Petiglyph -CliArgs @("--help")) "petiglyph --help"

    $doctor = Invoke-Petiglyph -CliArgs @("doctor", "--json")
    Assert-Success $doctor "petiglyph doctor --json"
    if ($doctor.Output -notmatch '"ok"\s*:\s*true') {
        Write-Host $doctor.Output
        Fail "petiglyph doctor --json did not report ok=true"
    }
    Write-Ok "doctor payload contains ok=true"

    $tui = Invoke-PetiglyphTuiNoTty
    if ($tui.ExitCode -eq 0) {
        Fail "petiglyph tui should fail without a TTY"
    }
    if ($tui.Output -notmatch "(?i)requires a terminal") {
        Write-Host $tui.Output
        Fail "non-TTY TUI failure message did not include terminal-required guidance"
    }
    Write-Ok "petiglyph tui non-TTY guard"
}

if (-not $SkipClipboardChecks) {
    $payload = "petiglyph-clipboard-smoke-$([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())-$PID"
    $providers = @("powershell", "clip.exe")

    $selected = $null
    Write-Info ("running clipboard provider chain: " + ($providers -join ", "))

    foreach ($provider in $providers) {
        try {
            switch ($provider) {
                "powershell" {
                    $payload | powershell -NoProfile -Command "Set-Clipboard -Value ([Console]::In.ReadToEnd())" | Out-Null
                }
                "clip.exe" {
                    $payload | clip.exe
                }
                default {
                    throw "unsupported provider: $provider"
                }
            }
            $selected = $provider
            Write-Ok "copied payload with $selected"
            break
        } catch {
            Write-Host "[WARN] provider failed: $provider ($($_.Exception.Message))"
        }
    }

    if ($null -eq $selected) {
        Fail "no clipboard provider succeeded"
    }

    if (-not $NoReadbackVerify) {
        try {
            $readback = Get-Clipboard -Raw
            if ($readback -ne $payload) {
                Fail "clipboard readback mismatch for $selected"
            }
            Write-Ok "clipboard readback matched payload"
        } catch {
            Write-Host "[WARN] readback skipped ($($_.Exception.Message))"
        }
    }
} else {
    Write-Ok "clipboard checks skipped"
}

Write-Ok "clipboard smoke test finished"
