param(
    [ValidateSet("single", "baseline", "compare")]
    [string]$Mode = "single",
    [string]$Baseline = "main",
    [string[]]$Benches = @("manager", "rate_limiter", "circuit_breaker")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$workspaceRoot = Split-Path -Parent $PSScriptRoot
Push-Location $workspaceRoot

try {
    Write-Host "Running nebula-resilience benches" -ForegroundColor Cyan
    Write-Host "Mode: $Mode | Baseline: $Baseline | Benches: $($Benches -join ', ')" -ForegroundColor Gray

    foreach ($bench in $Benches) {
        $args = @("bench", "-p", "nebula-resilience", "--bench", $bench, "--")

        switch ($Mode) {
            "baseline" {
                $args += @("--save-baseline", $Baseline)
            }
            "compare" {
                $args += @("--baseline", $Baseline)
            }
            default {
                # single run without baseline compare
            }
        }

        Write-Host "`n==> cargo $($args -join ' ')" -ForegroundColor Yellow
        cargo @args
    }

    Write-Host "`nDone. Criterion outputs are in target/criterion/." -ForegroundColor Green
}
finally {
    Pop-Location
}
