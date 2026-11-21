# Prepara o libmpv fixado e validado por SHA-256 para o build Windows do CineHub.
#
# - Baixa um pacote de desenvolvimento identificado por data e commit.
# - Verifica o SHA-256 antes de extrair qualquer arquivo.
# - Copia mpv-2.dll para src-tauri/resources/ (o Tauri instala a DLL junto do app).
# - Procura mpv.lib (import library MSVC). Se o pacote não tiver, gera um a partir das
#   exportações da DLL, usando dumpbin/lib do Visual Studio.
# - Define RUSTFLAGS para o linker achar o mpv.lib nas próximas etapas do job.
#
# Uso (no diretório src-tauri): pwsh -File ../.github/scripts/prepare-libmpv.ps1
param(
    [string]$Root = (Get-Location).Path,
    [string]$Archive = 'mpv-dev-x86_64-20261108-git-55d3feb45c.7z',
    [string]$ExpectedSha256 = '7c147cb7ceaba717414e782ef4398de728f679db6e86daaa8aeb175670cb7994'
)
$ErrorActionPreference = 'Stop'

$work = Join-Path $env:RUNNER_TEMP 'libmpv'
$extract = Join-Path $work 'x'
New-Item -ItemType Directory -Force $work | Out-Null
$archivePath = Join-Path $work $Archive
$downloadUrl = "https://sourceforge.net/projects/mpv-player-windows/files/libmpv/$Archive/download"
Invoke-WebRequest -Uri $downloadUrl -OutFile $archivePath
$actualSha256 = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -ne $ExpectedSha256.ToLowerInvariant()) {
    throw "SHA-256 inválido para $Archive`: esperado $ExpectedSha256; obtido $actualSha256."
}
Write-Host "baixado e validado: $Archive (SHA-256 $actualSha256)"

$7z = 'C:\Program Files\7-Zip\7z.exe'
& $7z x $archivePath "-o$extract" -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw "falha ao extrair $Archive" }

# O nome da DLL varia entre pacotes (mpv-2.dll ou libmpv-2.dll). Se não achar, lista o pacote no log.
$dll = Get-ChildItem $extract -Recurse -Filter '*.dll' |
    Where-Object { $_.Name -match '^(lib)?mpv(-\d)?\.dll$' } | Select-Object -First 1
if (-not $dll) {
    Write-Host 'Conteúdo do pacote mpv-dev:'
    Get-ChildItem $extract -Recurse -File | ForEach-Object { Write-Host "  $($_.FullName.Substring($extract.Length))" }
    throw 'DLL do libmpv não encontrada no pacote'
}
Write-Host "DLL encontrada: $($dll.Name)"

$libDir = Join-Path $work 'lib'
New-Item -ItemType Directory -Force $libDir | Out-Null
$libOut = Join-Path $libDir 'mpv.lib'

$lib = Get-ChildItem $extract -Recurse -Filter '*.lib' | Where-Object { $_.Name -match '^(lib)?mpv\.lib$' } | Select-Object -First 1
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
