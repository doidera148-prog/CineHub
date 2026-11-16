# Prepara o libmpv para o build de Windows do CineHub (feature `mpv`).
#
# - Baixa o pacote de desenvolvimento do mpv (release mais recente que o contém).
# - Copia mpv-2.dll para src-tauri/resources/ (o Tauri instala a DLL junto do app).
# - Procura mpv.lib (import library MSVC). Se o pacote não tiver, gera um a partir das
#   exportações da DLL, usando dumpbin/lib do Visual Studio.
# - Define RUSTFLAGS para o linker achar o mpv.lib nas próximas etapas do job.
#
# Uso (no diretório src-tauri): pwsh -File ../.github/scripts/prepare-libmpv.ps1
param([string]$Root = (Get-Location).Path)
$ErrorActionPreference = 'Stop'

# Usa a release mais recente que tenha o pacote mpv-dev x86_64 (o nome do arquivo inclui a data
# e o hash do commit, então ele é lido da API em vez de escrito à mão).
$headers = @{ 'Accept' = 'application/vnd.github+json' }
if ($env:GH_TOKEN) { $headers['Authorization'] = "Bearer $env:GH_TOKEN" }
$releases = Invoke-RestMethod -Headers $headers -Uri 'https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases?per_page=10'
$asset = $null
foreach ($release in $releases) {
    $asset = $release.assets | Where-Object { $_.name -match '^mpv-dev-x86_64-\d{8}-git-[0-9a-f]+\.7z$' } | Select-Object -First 1
    if ($asset) { Write-Host "release usada: $($release.tag_name)"; break }
}
if (-not $asset) { throw 'nenhum pacote mpv-dev x86_64 nas releases recentes' }
$Archive = $asset.name

$work = Join-Path $env:RUNNER_TEMP 'libmpv'
$extract = Join-Path $work 'x'
New-Item -ItemType Directory -Force $work | Out-Null
$archivePath = Join-Path $work $Archive
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $archivePath
Write-Host "baixado: $Archive"

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
