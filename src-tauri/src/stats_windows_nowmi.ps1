# home-cc machine stats, Windows, reduced version for accounts that are refused CIM/WMI
# (standard accounts in an SSH session). Uses only .NET and the registry; a value whose
# query is refused is left out. Same rules as stats_windows.ps1: comment lines are removed
# before sending, keep each on a line of its own.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$n = "`n"
$t = "`t"
$o = '@@hcc-stats 1' + $n
$v = [Environment]::OSVersion.Version
$name = 'Windows'
try {
  $r = Get-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
  if ($r.ProductName) { $name = [string]$r.ProductName }
  if ($v.Build -ge 22000) { $name = $name -replace 'Windows 10', 'Windows 11' }
} catch { }
$o += 'os=' + $name.Trim() + $n
$o += 'os_id=windows' + $n
$o += 'os_version=' + $v.Major + '.' + $v.Minor + '.' + $v.Build + $n
$o += 'kernel=' + $v.Build + $n
$o += 'arch=' + $env:PROCESSOR_ARCHITECTURE + $n
$o += 'host=' + [Environment]::MachineName + $n
$o += 'uptime_s=' + [long]([Diagnostics.Stopwatch]::GetTimestamp() / [Diagnostics.Stopwatch]::Frequency) + $n
$o += 'ncpu=' + [Environment]::ProcessorCount + $n
try {
  $pc = New-Object System.Diagnostics.PerformanceCounter('Processor', '% Processor Time', '_Total')
  [void]$pc.NextValue()
  Start-Sleep -Milliseconds 500
  $o += 'cpu_pct=' + [int]$pc.NextValue() + $n
} catch { }
try {
  Add-Type -AssemblyName Microsoft.VisualBasic
  $ci = New-Object Microsoft.VisualBasic.Devices.ComputerInfo
  $o += 'mem_kb=' + [long]($ci.TotalPhysicalMemory / 1024) + ' ' + [long]($ci.AvailablePhysicalMemory / 1024) + $n
} catch { }
foreach ($d in [System.IO.DriveInfo]::GetDrives()) {
  try {
    if ($d.DriveType -eq [System.IO.DriveType]::Fixed -and $d.IsReady -and $d.TotalSize -gt 0) {
      $label = ([string]$d.VolumeLabel) -replace '[\t\r\n]', ' '
      $o += 'disk=' + [long]($d.TotalSize / 1024) + $t + [long](($d.TotalSize - $d.TotalFreeSpace) / 1024) + $t + [long]($d.AvailableFreeSpace / 1024) + $t + $d.DriveFormat + $t + $label + $t + $d.Name.TrimEnd('\') + $n
    }
  } catch { }
}
$o += '@@end' + $n
$b = [System.Text.Encoding]::UTF8.GetBytes($o)
$s = [System.Console]::OpenStandardOutput()
$s.Write($b, 0, $b.Length)
$s.Flush()
