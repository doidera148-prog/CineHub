# Prepara o libmpv para o build de Windows do CineHub (feature `mpv`).
#
# - Baixa o pacote de desenvolvimento do mpv (versão fixada abaixo).
# - Copia mpv-2.dll para src-tauri/resources/ (o Tauri instala a DLL junto do app).
# - Procura mpv.lib (import library MSVC). Se o pacote não tiver, gera um a partir das
#   exportações da DLL, usando dumpbin/lib do Visual Studio.
# - Define RUSTFLAGS para o linker achar o mpv.lib nas próximas etapas do job.
#
# Uso (no diretório src-tauri): pwsh -File ../.github/scripts/prepare-libmpv.ps1
param([string]$Root = (Get-Location).Path)
$ErrorActionPreference = 'Stop'

$Tag = '20260921'
$Archive = "mpv-dev-x86_64-$Tag-git-e76a35ec95.7z"
$Url = "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/$Tag/$Archive"

$work = Join-Path $env:RUNNER_TEMP 'libmpv'
$extract = Join-Path $work 'x'
New-Item -ItemType Directory -Force $work | Out-Null
$archivePath = Join-Path $work $Archive
Invoke-WebRequest -Uri $Url -OutFile $archivePath

$7z = 'C:\Program Files\7-Zip\7z.exe'
& $7z x $archivePath "-o$extract" -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw "falha ao extrair $Archive" }

$dll = Get-ChildItem $extract -Recurse -Filter 'mpv-2.dll' | Select-Object -First 1
if (-not $dll) { throw 'mpv-2.dll não encontrada no pacote do mpv' }

$libDir = Join-Path $work 'lib'
New-Item -ItemType Directory -Force $libDir | Out-Null
$libOut = Join-Path $libDir 'mpv.lib'

$lib = Get-ChildItem $extract -Recurse -Filter 'mpv.lib' | Select-Object -First 1
if ($lib) {
    Copy-Item $lib.FullName $libOut -Force
} else {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $vsPath = & $vswhere -latest -products * -property installationPath
    $dumpbin = Get-ChildItem (Join-Path $vsPath 'VC\Tools\MSVC') -Recurse -Filter 'dumpbin.exe' |
        Where-Object { $_.FullName -match 'Hostx64\\x64' } | Select-Object -First 1
    if (-not $dumpbin) { throw 'dumpbin.exe não encontrado' }
    $libExe = Join-Path (Split-Path $dumpbin.FullName) 'lib.exe'

    $names = & $dumpbin.FullName /exports $dll.FullName |
        ForEach-Object { if ($_ -match '^\s+\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]+\s+(\S+)\s*$') { $Matches[1] } }
    if (-not $names) { throw 'nenhuma exportação encontrada em mpv-2.dll' }

    $def = Join-Path $libDir 'mpv.def'
    Set-Content -Path $def -Value (@('LIBRARY mpv-2.dll', 'EXPORTS') + $names) -Encoding ascii
    & $libExe "/def:$def" /machine:x64 "/out:$libOut"
    if ($LASTEXITCODE -ne 0) { throw 'falha ao gerar mpv.lib' }
}

$resources = Join-Path $Root 'resources'
New-Item -ItemType Directory -Force $resources | Out-Null
Copy-Item $dll.FullName (Join-Path $resources 'mpv-2.dll') -Force

"RUSTFLAGS=-L native=$libDir" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
Write-Host "libmpv pronto: $($dll.Name) e mpv.lib em $libDir"
