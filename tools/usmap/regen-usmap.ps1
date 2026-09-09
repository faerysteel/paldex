<#
.SYNOPSIS
Build the patched mappings dumper, inject it into Palworld, and write a .usmap.

.DESCRIPTION
Run this on the Windows machine that has Palworld installed. It clones the
dumper, applies palworld-ue51.patch (sitting next to this script), builds it
with MSBuild, launches the game, injects the DLL, and copies out the mappings
file the dumper produces.

Everything is auto-detected where it can be: the game through Steam's registry
entry and library folders, MSBuild through vswhere. Override any of it with the
parameters below.

Requirements are the game itself, really. Palworld has to actually render, so
this needs a real GPU and an interactive desktop session. A headless box or an
RDP session with no GPU will produce a dump with too thin an object graph, or
none at all. Any ordinary Windows PC or laptop that plays the game is fine.

You also need git, and MSVC build tools with the C++ workload.

.PARAMETER OutFile
Where to write the finished Mappings.usmap. Defaults to the current directory.

.PARAMETER GameDir
Palworld's Win64 binaries directory. Auto-detected from Steam if omitted.

.PARAMETER SrcDir
Working directory for the dumper checkout. Defaults to a folder under TEMP.

.PARAMETER MSBuild
Full path to MSBuild.exe. Auto-detected with vswhere if omitted.

.PARAMETER SkipBuild
Reuse the DLL already built in SrcDir instead of rebuilding.

.PARAMETER KeepGame
Leave Palworld running when the dump is done.

.EXAMPLE
.\regen-usmap.ps1 -OutFile C:\Users\me\Mappings.usmap
#>
[CmdletBinding()]
param(
    [string] $OutFile = (Join-Path (Get-Location) 'Mappings.usmap'),
    [string] $GameDir,
    [string] $SrcDir  = (Join-Path $env:TEMP 'UnrealMappingsDumper'),
    [string] $MSBuild,
    [switch] $SkipBuild,
    [switch] $KeepGame
)

$ErrorActionPreference = 'Stop'

$DumperRepo = 'https://github.com/gameknife/UnrealMappingsDumper.git'
$DumperRef  = '3bf7e24'
$PatchFile  = Join-Path $PSScriptRoot 'palworld-ue51.patch'

function Say  { param($m) Write-Host "`n==> $m" -ForegroundColor Cyan }
function Note { param($m) Write-Host "    $m" }
function Die  { param($m) Write-Host "FATAL: $m" -ForegroundColor Red; exit 1 }

# ------------------------------------------------------------------ discovery

# Palworld can live in any Steam library, not just the default one under
# Steam's own root, so the library list has to come out of libraryfolders.vdf
# rather than being assumed.
function Find-GameDir {
    $tail = 'steamapps\common\Palworld\Pal\Binaries\Win64'
    $steam = (Get-ItemProperty -Path 'HKCU:\Software\Valve\Steam' -Name SteamPath -ErrorAction SilentlyContinue).SteamPath
    if (-not $steam) {
        $steam = (Get-ItemProperty -Path 'HKLM:\SOFTWARE\WOW6432Node\Valve\Steam' -Name InstallPath -ErrorAction SilentlyContinue).InstallPath
    }
    if (-not $steam) { return $null }

    $roots = @($steam)
    $vdf = Join-Path $steam 'steamapps\libraryfolders.vdf'
    if (Test-Path $vdf) {
        Select-String -Path $vdf -Pattern '"path"\s+"([^"]+)"' -AllMatches | ForEach-Object {
            $_.Matches | ForEach-Object { $roots += $_.Groups[1].Value -replace '\\\\', '\' }
        }
    }
    foreach ($r in $roots) {
        $candidate = Join-Path $r $tail
        if (Test-Path (Join-Path $candidate 'Palworld-Win64-Shipping.exe')) { return $candidate }
    }
    return $null
}

function Find-MSBuild {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { return $null }
    & $vswhere -latest -products * -requires Microsoft.Component.MSBuild `
        -find 'MSBuild\**\Bin\amd64\MSBuild.exe' 2>$null | Select-Object -First 1
}

Say 'Preflight'

if (-not (Test-Path $PatchFile)) { Die "missing patch next to this script: $PatchFile" }
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Die 'git is not on PATH' }

if (-not $GameDir) {
    $GameDir = Find-GameDir
    if (-not $GameDir) {
        Die 'could not find Palworld via Steam. Pass -GameDir <...\Pal\Binaries\Win64>'
    }
}
if (-not (Test-Path (Join-Path $GameDir 'Palworld-Win64-Shipping.exe'))) {
    Die "no Palworld-Win64-Shipping.exe in $GameDir"
}
Note "game       $GameDir"

if (-not $SkipBuild) {
    if (-not $MSBuild) {
        $MSBuild = Find-MSBuild
        if (-not $MSBuild) { Die 'could not find MSBuild via vswhere. Pass -MSBuild <path>' }
    }
    if (-not (Test-Path $MSBuild)) { Die "no MSBuild at $MSBuild" }
    Note "msbuild    $MSBuild"
}

$DllPath = Join-Path $SrcDir 'x64\UE4SS.dll'
Note "source     $SrcDir"
Note "output     $OutFile"

# ---------------------------------------------------------------- build

if (-not $SkipBuild) {
    Say "Preparing dumper source"

    if (-not (Test-Path (Join-Path $SrcDir '.git'))) {
        New-Item -ItemType Directory -Force -Path (Split-Path $SrcDir -Parent) | Out-Null
        git clone --quiet --recurse-submodules $DumperRepo $SrcDir
        if ($LASTEXITCODE -ne 0) { Die 'git clone failed' }
    }
    Push-Location $SrcDir
    try {
        git checkout --quiet $DumperRef
        # Discard any previous patch application so re-runs are repeatable.
        git checkout -- .
        # Dependencies/Memcury is a submodule and scanning.cpp includes its
        # header directly, so a checkout without it builds as far as C1083.
        # Done here as well as at clone time, since the ref moves the pointer.
        git submodule update --init --recursive --quiet
        if ($LASTEXITCODE -ne 0) { Die 'git submodule update failed' }
        Note "at $(git rev-parse --short HEAD)"

        # git apply verifies context, so a partial application fails loudly
        # instead of silently half-patching the way a regex edit would.
        git apply --whitespace=nowarn $PatchFile
        if ($LASTEXITCODE -ne 0) { Die 'git apply failed' }
        Note 'patch applied'
        git diff --stat | ForEach-Object { Note $_ }
    } finally {
        Pop-Location
    }

    Say 'Building (MSBuild, Release x64)'
    $buildLog = Join-Path $SrcDir 'build.log'

    # A crashed game leaves CrashReportClient holding the old DLL mapped, which
    # blocks the link step with "user-mapped section open".
    Stop-Process -Name CrashReportClient -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2

    & $MSBuild (Join-Path $SrcDir 'UnrealMappingsDumper.sln') `
        /t:Rebuild /p:Configuration=Release /p:Platform=x64 /v:minimal /nologo `
        *> $buildLog

    if ($LASTEXITCODE -ne 0) {
        Select-String -Path $buildLog -Pattern ': error' |
            Select-Object -First 5 |
            ForEach-Object { Note $_.Line.Trim() }
        Die "build failed, full log at $buildLog"
    }
    if (-not (Test-Path $DllPath)) { Die 'build produced no DLL' }
    Note "built $((Get-Item $DllPath).Length) bytes"
} else {
    Say 'Skipping build (-SkipBuild)'
    if (-not (Test-Path $DllPath)) { Die "no DLL at $DllPath, drop -SkipBuild" }
}

# ---------------------------------------------------------------- launch

$UsmapOut = Join-Path $GameDir 'Mappings.usmap'
$DumperLog = Join-Path (Split-Path $DllPath -Parent) 'dumper.log'
Remove-Item $UsmapOut, $DumperLog -Force -ErrorAction SilentlyContinue

$proc = Get-Process Palworld-Win64-Shipping -ErrorAction SilentlyContinue | Select-Object -First 1

if ($proc) {
    Say "Using the Palworld already running (pid $($proc.Id))"
} else {
    Say 'Launching Palworld'
    # Start the shipping exe directly rather than through Steam: Steam's
    # DirectX 11/12 chooser prompt blocks an unattended launch, and going
    # direct skips it.
    Start-Process -FilePath (Join-Path $GameDir 'Palworld-Win64-Shipping.exe') -WorkingDirectory $GameDir

    # The dump needs a fully initialised object graph, so wait for the game to
    # stop growing rather than for a fixed interval. Three consecutive polls
    # within 60 MB of each other means it has settled.
    $t0 = Get-Date; $last = 0; $stable = 0
    while (((Get-Date) - $t0).TotalSeconds -lt 240) {
        Start-Sleep -Seconds 10
        $proc = Get-Process Palworld-Win64-Shipping -ErrorAction SilentlyContinue | Select-Object -First 1
        if (-not $proc) { continue }
        $mb = [int]($proc.WorkingSet64 / 1MB)
        if ([Math]::Abs($mb - $last) -lt 60) { $stable++ } else { $stable = 0 }
        $last = $mb
        if ($stable -ge 3 -and ((Get-Date) - $t0).TotalSeconds -ge 60) { break }
    }
    $proc = Get-Process Palworld-Win64-Shipping -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $proc) { Die 'the game failed to start' }
    Note "settled pid=$($proc.Id) mem=${last}MB"
}

# ---------------------------------------------------------------- inject

Say 'Injecting the dumper'

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class Inj {
  [DllImport("kernel32", SetLastError=true)] public static extern IntPtr OpenProcess(uint a, bool b, int p);
  [DllImport("kernel32", SetLastError=true)] public static extern IntPtr VirtualAllocEx(IntPtr h, IntPtr a, uint s, uint t, uint p);
  [DllImport("kernel32", SetLastError=true)] public static extern bool WriteProcessMemory(IntPtr h, IntPtr a, byte[] b, uint s, out UIntPtr w);
  [DllImport("kernel32", CharSet=CharSet.Unicode, SetLastError=true)] public static extern IntPtr GetModuleHandleW(string n);
  [DllImport("kernel32", CharSet=CharSet.Ansi,    SetLastError=true)] public static extern IntPtr GetProcAddress(IntPtr h, string n);
  [DllImport("kernel32", SetLastError=true)] public static extern IntPtr CreateRemoteThread(IntPtr h, IntPtr a, uint s, IntPtr f, IntPtr p, uint c, IntPtr t);
  [DllImport("kernel32", SetLastError=true)] public static extern uint WaitForSingleObject(IntPtr h, uint ms);
  [DllImport("kernel32", SetLastError=true)] public static extern bool GetExitCodeThread(IntPtr h, out uint c);
}
"@

function InjectDie {
    param($m)
    Die "$m (win32=$([Runtime.InteropServices.Marshal]::GetLastWin32Error()))"
}

# GetModuleHandleW MUST be CharSet.Unicode. DllImport defaults to Ansi, so the
# W function gets a mangled string and returns NULL, GetProcAddress then returns
# NULL, and CreateRemoteThread cheerfully starts a thread at address 0 which
# kills the game instantly with a fault at IP 0x0. Validate the resolved
# address, not just the thread handle.
$k32 = [Inj]::GetModuleHandleW('kernel32.dll')
if ($k32 -eq [IntPtr]::Zero) { InjectDie 'GetModuleHandleW' }
$ll = [Inj]::GetProcAddress($k32, 'LoadLibraryW')
if ($ll -eq [IntPtr]::Zero) { InjectDie 'GetProcAddress' }

$h = [Inj]::OpenProcess(0x1F0FFF, $false, $proc.Id)
if ($h -eq [IntPtr]::Zero) { InjectDie 'OpenProcess' }

$bytes = [Text.Encoding]::Unicode.GetBytes($DllPath + "`0")
$mem = [Inj]::VirtualAllocEx($h, [IntPtr]::Zero, [uint32]$bytes.Length, 0x3000, 0x04)
if ($mem -eq [IntPtr]::Zero) { InjectDie 'VirtualAllocEx' }

$w = [UIntPtr]::Zero
if (-not [Inj]::WriteProcessMemory($h, $mem, $bytes, [uint32]$bytes.Length, [ref]$w)) {
    InjectDie 'WriteProcessMemory'
}
$th = [Inj]::CreateRemoteThread($h, [IntPtr]::Zero, 0, $ll, $mem, 0, [IntPtr]::Zero)
if ($th -eq [IntPtr]::Zero) { InjectDie 'CreateRemoteThread' }

[void][Inj]::WaitForSingleObject($th, 30000)
$code = 0
[void][Inj]::GetExitCodeThread($th, [ref]$code)
if ($code -eq 0) { InjectDie 'LoadLibraryW returned NULL' }

$proc.Refresh()
if (-not ($proc.Modules | Where-Object { $_.ModuleName -match 'UE4SS|MappingsDumper' })) {
    Die 'the DLL loaded but never appeared in the module list'
}
Note 'injected'

# ---------------------------------------------------------------- collect

Say 'Waiting for the dump'
# The dumper sleeps 10s on load, then walks ~10k structs before writing.
Start-Sleep -Seconds 120

if (-not (Test-Path $UsmapOut)) {
    Die "no usmap produced. The dumper's own log is at $DumperLog once the game exits"
}
Note "$((Get-Item $UsmapOut).Length) bytes at $UsmapOut"

New-Item -ItemType Directory -Force -Path (Split-Path $OutFile -Parent) | Out-Null
Copy-Item $UsmapOut $OutFile -Force

if (-not $KeepGame) {
    Say 'Stopping the game'
    Stop-Process -Name Palworld-Win64-Shipping, Palworld -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 5
    Stop-Process -Name CrashReportClient -Force -ErrorAction SilentlyContinue

    # The game holds dumper.log open for as long as it lives, so these counts
    # only become readable once it exits. A log you cannot read yet means the
    # game survived the dump, which is the good outcome.
    if (Test-Path $DumperLog) {
        $li = Get-Content $DumperLog -ErrorAction SilentlyContinue
        $bogus = @($li | Where-Object { $_ -like '*BOGUS*' }).Count
        Note ("BOGUS={0}  STRUCTS={1}  ENUMS={2}" -f `
            $bogus,
            @($li | Where-Object { $_ -like 'Struct: *' }).Count,
            @($li | Where-Object { $_ -like 'Enum: *' }).Count)
        if ($bogus -gt 0) {
            Write-Host "    WARNING: $bogus bogus-enum reports, enum data may be incomplete." -ForegroundColor Yellow
        }
    }
}

Say 'Done'
Note "wrote $OutFile"
Note "validate it with: python3 verify_usmap.py `"$OutFile`""
