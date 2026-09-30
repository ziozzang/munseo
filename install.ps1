# HWP2PDF Windows 설치 스크립트 (PowerShell)
#   irm https://raw.githubusercontent.com/ziozzang/munseo/main/install.ps1 | iex
#
# 최신 릴리스의 exe 를 받아 SHA256SUMS 로 검증한 뒤 %LOCALAPPDATA%\Programs\HWP2PDF 에 두고
# 시작 메뉴 바로가기를 만든다. 관리자 권한이 필요 없고, 앱의 자동 업데이트도 이 폴더에 쓴다.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'   # Windows PowerShell 5.1 에서 진행 표시가 다운로드를 크게 늦춘다
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repo = 'ziozzang/munseo'
$dir = Join-Path $env:LOCALAPPDATA 'Programs\HWP2PDF'
$headers = @{ 'User-Agent' = 'hwp2pdf-install'; 'Accept' = 'application/vnd.github+json' }

$rel = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers $headers
$name = "HWP2PDF_$($rel.tag_name.TrimStart('v'))_windows_x86_64.exe"
$asset = $rel.assets | Where-Object { $_.name -eq $name }
$sums = $rel.assets | Where-Object { $_.name -eq 'SHA256SUMS' }
if (-not $asset -or -not $sums) { throw "릴리스에 $name 또는 SHA256SUMS 가 없습니다." }

New-Item -ItemType Directory -Force $dir | Out-Null
$exe = Join-Path $dir 'hwp2pdf.exe'
$tmp = "$exe.download"
$sumFile = Join-Path $dir 'SHA256SUMS.download'

Write-Host "HWP2PDF $($rel.tag_name) 받는 중…"
Invoke-WebRequest $asset.browser_download_url -OutFile $tmp -UseBasicParsing
Invoke-WebRequest $sums.browser_download_url -OutFile $sumFile -UseBasicParsing
$expected = Get-Content $sumFile | ForEach-Object {
    $f = $_ -split '\s+'
    if ($f.Count -ge 2 -and $f[1].TrimStart('*') -eq $name) { $f[0] }
} | Select-Object -First 1
Remove-Item $sumFile
$actual = (Get-FileHash $tmp -Algorithm SHA256).Hash
if (-not $expected -or $actual -ne $expected) { Remove-Item $tmp; throw "체크섬이 맞지 않습니다 ($actual)." }

Move-Item -Force $tmp $exe
Unblock-File $exe

$shell = New-Object -ComObject WScript.Shell
$lnk = $shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Programs')) 'HWP2PDF.lnk'))
$lnk.TargetPath = $exe
$lnk.WorkingDirectory = $dir
$lnk.Save()

Write-Host "✓ $exe 설치됨 — 시작 메뉴에서 HWP2PDF 를 실행하세요. 이후 업데이트는 앱이 알아서 합니다."
