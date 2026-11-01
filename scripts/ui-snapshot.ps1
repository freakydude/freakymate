<#
  Renders the pages of the Slint UI headless as PNG against a freshly started fake Moonraker.
  Usage (repo root): powershell -File scripts\ui-snapshot.ps1 [-OutDir <folder>] [-Profile voron] [-Size 800x480]
                     [-NoFit] [-Scenes pages,network,calibrate,printing,shutdown,idle,login|all] [-Port 7126]
  -Port: port of its own fake; 7126 by default, so a fake on Moonraker's 7125 (the debug setup) is left alone.
  -NoFit: the window of the given size without scaling the layout to 800x480 (as a desktop window; at least 800x480).
          Without it the layout is fitted to the size like the app does in fullscreen.
  Without -Scenes all scenes except the slow 'idle' and 'login' run; with 'login' the fake wants a login (user test,
  password secret) and the other chosen scenes run logged in. Exit code 0 on success; the PNG paths are printed.
#>
param(
    [string]$OutDir = (Join-Path $env:TEMP 'freakymate-ui-snapshots'),
    [ValidateSet('basic', 'voron', 'happy-hare', 'ercf')][string]$Profile = 'voron',
    [string]$Size = '800x480',
    [string]$Scenes = '',
    [switch]$NoFit,
    [int]$Port = 7126
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
    cargo build -q -p fake-moonraker -p ui-snapshot
    if ($LASTEXITCODE -ne 0) { throw 'build failed' }

    # Stale instances from an earlier run on this port would hold it; instances on other ports are left alone.
    Get-CimInstance Win32_Process -Filter "Name='fake-moonraker.exe' OR Name='ui-snapshot.exe'" |
        Where-Object { $_.CommandLine -match "(--port $Port|127\.0\.0\.1:$Port)(\s|`"|$)" } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
    $fakeArguments = @('--port', $Port, '--time-scale', '20', '--profile', $Profile)
    if (",$Scenes," -like '*,login,*') { $fakeArguments += @('--auth', 'test:secret') }
    $fake = Start-Process -FilePath 'target\debug\fake-moonraker.exe' -ArgumentList $fakeArguments -PassThru -WindowStyle Hidden
    try {
        Start-Sleep -Milliseconds 800
        $arguments = @('--moonraker', "http://127.0.0.1:$Port", '--out', $OutDir, '--size', $Size)
        if ($Scenes) { $arguments += @('--scenes', $Scenes) }
        if ($NoFit) { $arguments += '--no-fit' }
        & 'target\debug\ui-snapshot.exe' @arguments
        $code = $LASTEXITCODE
    }
    finally {
        Stop-Process -Id $fake.Id -Force -ErrorAction SilentlyContinue
    }
    exit $code
}
finally {
    Pop-Location
}
