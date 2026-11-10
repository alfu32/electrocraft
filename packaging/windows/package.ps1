# MSIX with the reserved Microsoft Store identity, from the portable zip.
# Signed (and test-installed) when ELECTROCRAFT_PFX_B64 / ELECTROCRAFT_PFX_PASSWORD
# are set; otherwise unsigned, which Partner Center accepts (the Store signs).
param(
  [Parameter(Mandatory=$true)][ValidateSet('x64','arm64')][string]$Arch,
  [Parameter(Mandatory=$true)][string]$Version
)
$ErrorActionPreference = 'Stop'
$Target = if ($Arch -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Dist = Join-Path $Root 'dist'
$Package = "electrocraft-$Version-$Target"
$Archive = Join-Path $Dist "$Package.zip"
$Stage = Join-Path $Root "target\msix-$Arch"
if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
$Unzip = Join-Path $Root "target\msix-unzip-$Arch"
if (Test-Path $Unzip) { Remove-Item $Unzip -Recurse -Force }
Expand-Archive $Archive -DestinationPath $Unzip
Move-Item (Join-Path $Unzip $Package) $Stage
if (-not (Test-Path (Join-Path $Stage 'electrocraft.exe'))) { throw 'electrocraft.exe missing from the portable payload' }

# Tile and Store logos from the app icon.
$AssetsDir = Join-Path $Stage 'Assets'
New-Item $AssetsDir -ItemType Directory -Force | Out-Null
Add-Type -AssemblyName System.Drawing
$Icon = [System.Drawing.Image]::FromFile((Join-Path $Root 'packaging\electrocraft.png'))
foreach ($spec in @(@('StoreLogo',50),@('Square44x44Logo',44),@('Square150x150Logo',150))) {
  $bitmap = [System.Drawing.Bitmap]::new($spec[1],$spec[1])
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $graphics.Clear([System.Drawing.Color]::Transparent)
  $graphics.DrawImage($Icon,0,0,$spec[1],$spec[1])
  $bitmap.Save((Join-Path $AssetsDir "$($spec[0]).png"),[System.Drawing.Imaging.ImageFormat]::Png)
  $graphics.Dispose(); $bitmap.Dispose()
}
$Icon.Dispose()

python (Join-Path $Root 'scripts\release\msix.py') --version $Version --arch $Arch --output (Join-Path $Stage 'AppxManifest.xml')
if ($LASTEXITCODE -ne 0) { throw 'MSIX manifest generation failed' }

$Kit = 'C:\Program Files (x86)\Windows Kits\10\bin'
$MakeAppx = Get-ChildItem $Kit -Recurse -Filter makeappx.exe | Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $MakeAppx) { throw 'Windows SDK makeappx.exe not found' }
$Msix = Join-Path $Dist "electrocraft-$Version-windows-$Arch.msix"
& $MakeAppx.FullName pack /d $Stage /p $Msix /o
if ($LASTEXITCODE -ne 0 -or -not (Test-Path $Msix)) { throw 'makeappx pack failed (manifest or payload invalid)' }

$Identity = Get-Content (Join-Path $PSScriptRoot 'store-identity.json') -Raw | ConvertFrom-Json
$Artifacts = @($Msix)
if (-not [string]::IsNullOrWhiteSpace($env:ELECTROCRAFT_PFX_B64) -and -not [string]::IsNullOrWhiteSpace($env:ELECTROCRAFT_PFX_PASSWORD)) {
  $Pfx = Join-Path $env:RUNNER_TEMP ("electrocraft-sign-" + [guid]::NewGuid().ToString('N') + '.pfx')
  try {
    [IO.File]::WriteAllBytes($Pfx, [Convert]::FromBase64String($env:ELECTROCRAFT_PFX_B64))
    $Certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($Pfx, $env:ELECTROCRAFT_PFX_PASSWORD, [Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet)
    if (-not $Certificate.HasPrivateKey -or $Certificate.Subject -cne $Identity.publisher) { throw 'Signing certificate does not match the reserved Store publisher' }
    $SignTool = Get-ChildItem $Kit -Recurse -Filter signtool.exe | Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending | Select-Object -First 1
    & $SignTool.FullName sign /fd SHA256 /a /f $Pfx /p $env:ELECTROCRAFT_PFX_PASSWORD /tr http://timestamp.digicert.com /td SHA256 $Msix
    if ($LASTEXITCODE -ne 0) { throw 'MSIX signing failed' }
    $Cer = Join-Path $Dist "electrocraft-$Version-windows-$Arch.cer"
    [IO.File]::WriteAllBytes($Cer, $Certificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Cert))
    $Artifacts += $Cer
    # Install, check identity and launch, remove.
    $Store = [Security.Cryptography.X509Certificates.X509Store]::new('TrustedPeople','LocalMachine')
    $Store.Open('ReadWrite'); $Store.Add($Certificate)
    try {
      Add-AppxPackage -Path $Msix -ErrorAction Stop
      $Installed = Get-AppxPackage -Name $Identity.identityName
      if (-not $Installed -or $Installed.PackageFamilyName -cne $Identity.packageFamilyName) { throw 'Installed MSIX identity mismatch' }
      $Out = & (Join-Path $Installed.InstallLocation 'electrocraft.exe') --version
      if ($Out -ne "electrocraft $Version") { throw "Installed MSIX reports '$Out'" }
      Remove-AppxPackage -Package $Installed.PackageFullName
    } finally { $Store.Remove($Certificate); $Store.Close() }
    Write-Host "Signed and test-installed $Msix"
  } finally { if (Test-Path $Pfx) { Remove-Item $Pfx -Force } }
} else {
  Write-Host "No ELECTROCRAFT_PFX_* secrets: $Msix is unsigned (for Partner Center upload; sideloading needs a signed package)"
}
foreach ($a in $Artifacts) {
  $digest = (Get-FileHash $a -Algorithm SHA256).Hash.ToLowerInvariant()
  "$digest  $(Split-Path $a -Leaf)" | Set-Content "$a.sha256"
}
