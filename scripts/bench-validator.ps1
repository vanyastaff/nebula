param(
    [ValidateSet("quick", "full", "baseline", "compare")]
    [string]$Mode = "quick",
    [string]$Baseline = "main",
    [string[]]$Benches = @("string_validators", "combinators", "error_construction", "cache")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$workspaceRoot = Split-Path -Parent $PSScriptRoot
Push-Location $workspaceRoot

try {
    Write-Host "Running nebula-validator benches" -ForegroundColor Cyan
    Write-Host "Mode: $Mode | Baseline: $Baseline | Benches: $($Benches -join ', ')" -ForegroundColor Gray

    foreach ($bench in $Benches) {
        $args = @("bench", "-p", "nebula-validator", "--bench", $bench, "--")

        switch ($Mode) {
            "quick" {
                # PR profile: fast feedback, reduced sample size
                $args += @("--quick")
            }
            "full" {
                # Release profile: full statistical analysis
                # Uses criterion defaults (100 samples, 5s warmup)
            }
            "baseline" {
                $args += @("--save-baseline", $Baseline)
            }
            "compare" {
                $args += @("--baseline", $Baseline)
            }
        }

        Write-Host "`n==> cargo $($args -join ' ')" -ForegroundColor Yellow
        cargo @args
    }

    Write-Host "`nDone. Criterion outputs are in target/criterion/." -ForegroundColor Green
    Write-Host "HTML reports: target/criterion/<group>/report/index.html" -ForegroundColor Gray
}
finally {
    Pop-Location
}
