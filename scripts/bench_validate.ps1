# scripts\bench_validate.ps1
# Real bench acceptance run on the machine that has the bench: routes every
# built-in and agent-designed board through the CLI and checks result.ok.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\bench_validate.ps1 [-Exe target\debug\amproute-studio.exe]

param([string]$Exe = 'target\debug\amproute-studio.exe')
$exe = (Resolve-Path $Exe).Path

$boards = @(
    'lm386-mini', 'tda2030-compact', 'cmoy-headphone', 'classic-ltp', 'twin-pair-200',
    'pam8403-mini', 'lm358-preamp', 'tda2822-bridge', 'ne555-tone', 'classa-single',
    'jfet-buffer', 'discrete-opamp', 'lm1875-mono', 'ta2024-tripath', 'hybrid-output'
)

$fail = $false
foreach ($id in $boards) {
    $out = Join-Path $env:TEMP "ampbench_$id"
    New-Item -ItemType Directory -Force -Path $out | Out-Null
    $json = Join-Path $out 'result.json'
    try {
        $res = & $exe --cli --board $id -o $out --json 2>$null
        $ok = ($res | Out-String | ConvertFrom-Json).ok -eq $true
    } catch {
        $ok = $false
    }
    if ($ok) {
        Write-Host "OK   $id"
    } else {
        Write-Host "FAIL $id"
        $fail = $true
    }
}

if ($fail) {
    Write-Host "Bench acceptance: FAILED"
    exit 1
}
Write-Host "Bench acceptance: all boards ok"
Write-Host "Next: bench meter — gain, THD (1 kHz), junction temperature of the boards."
