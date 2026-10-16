$ErrorActionPreference = "Stop"
$traceRoot = Join-Path (Get-Location) "python-network-traces"
New-Item -ItemType Directory -Force $traceRoot | Out-Null
$curl = Join-Path $env:SystemRoot "System32/curl.exe"
& $curl --version | Out-File (Join-Path $traceRoot "curl-version.txt")
$metadata = Get-Content crates/uv-python-managed/download-metadata.json -Raw | ConvertFrom-Json
$entry = $metadata.'cpython-3.13.16+freethreaded-windows-x86_64-none'
$urls = @{
    github = $entry.url
    mirror = $entry.url.Replace("https://github.com/astral-sh/python-build-standalone/", "https://releases.astral.sh/github/python-build-standalone/")
}
$machine = @{
    provider = $env:DIAGNOSTIC_PROVIDER
    hostname = [System.Net.Dns]::GetHostName()
    os = [System.Environment]::OSVersion.VersionString
    cwd = (Get-Location).Path
    commit = $env:GITHUB_SHA
    curl = $curl
}
$machine | ConvertTo-Json | Set-Content (Join-Path $traceRoot "machine.json")
$records = @()
foreach ($iteration in 1..3) {
    $endpoints = if ($iteration % 2) { @("mirror", "github") } else { @("github", "mirror") }
    foreach ($endpoint in $endpoints) {
        $casePath = Join-Path $traceRoot "$iteration-$endpoint"
        New-Item -ItemType Directory -Force $casePath | Out-Null
        $archivePath = Join-Path $casePath "archive.tar.gz"
        $headersPath = Join-Path $casePath "headers.raw"
        $record = @{
            iteration = $iteration
            endpoint = $endpoint
            started_at = [DateTime]::UtcNow.ToString("o")
        }
        $record | ConvertTo-Json -Compress | Write-Host
        $metrics = & $curl --http1.1 --location --fail --silent --show-error --max-time 90 --output $archivePath --dump-header $headersPath --write-out "%{http_code},%{remote_ip},%{http_version},%{size_download},%{time_connect},%{time_appconnect},%{time_starttransfer},%{time_total},%{speed_download}" $urls[$endpoint] 2> (Join-Path $casePath "stderr.log")
        $record.exit_code = $LASTEXITCODE
        $fields = ($metrics -join "").Split(",")
        $record.http_code = [int]$fields[0]
        $record.remote_ip = $fields[1]
        $record.http_version = $fields[2]
        $record.size_download = [long]$fields[3]
        $names = @("time_connect", "time_appconnect", "time_starttransfer", "time_total", "speed_download")
        for ($index = 0; $index -lt $names.Count; $index++) {
            $record[$names[$index]] = [double]::Parse($fields[$index + 4], [System.Globalization.CultureInfo]::InvariantCulture)
        }
        if (Test-Path $headersPath) {
            Get-Content $headersPath | Where-Object { $_ -match "^(HTTP/|content-length:|content-encoding:|server:|age:|cf-ray:|cf-cache-status:|x-cache:|x-served-by:)" } | Set-Content (Join-Path $casePath "headers.log")
            Remove-Item $headersPath
        }
        if ($record.exit_code -eq 0) {
            $record.sha256_matches_metadata = (Get-FileHash $archivePath -Algorithm SHA256).Hash -eq $entry.sha256
        }
        $records += $record
        $records | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $traceRoot "results.json")
        $record | ConvertTo-Json -Compress | Write-Host
    }
}
