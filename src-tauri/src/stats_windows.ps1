# home-cc machine stats, Windows, full version. Needs an account that may use CIM/WMI over
# SSH, which by default means an administrator. If any query is refused the script stops with
# no "@@end" line and home-cc runs stats_windows_nowmi.ps1 instead.
# Sent as "powershell.exe -EncodedCommand <base64>". Comment lines and blank lines are
# removed and line endings are made the same before encoding, so comments here cost
# nothing; keep each comment on a line of its own. The encoded command must stay well
# under cmd.exe's limit of 8191 characters (a unit test checks).
# Numbers are cast to whole numbers so a decimal-comma language setting cannot change
# them, and the result is written as raw UTF-8 bytes so the console code page does not matter.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$os = Get-CimInstance -ClassName Win32_OperatingSystem
$cs = Get-CimInstance -ClassName Win32_ComputerSystem -Property NumberOfLogicalProcessors
$cpu = $null
try {
  $sum = 0
  $cnt = 0
  foreach ($p in @(Get-CimInstance -ClassName Win32_Processor -Property LoadPercentage)) {
    if ($null -ne $p.LoadPercentage) { $sum += $p.LoadPercentage; $cnt++ }
  }
  if ($cnt -gt 0) { $cpu = $sum / $cnt }
} catch { }
$n = "`n"
$t = "`t"
$o = '@@hcc-stats 1' + $n
$o += 'os=' + ([string]$os.Caption).Trim() + $n
$o += 'os_id=windows' + $n
$o += 'os_version=' + $os.Version + $n
$o += 'kernel=' + $os.BuildNumber + $n
$o += 'arch=' + $env:PROCESSOR_ARCHITECTURE + $n
$o += 'host=' + $os.CSName + $n
$o += 'uptime_s=' + [long]((Get-Date) - $os.LastBootUpTime).TotalSeconds + $n
$o += 'ncpu=' + $cs.NumberOfLogicalProcessors + $n
if ($null -ne $cpu) { $o += 'cpu_pct=' + [int]$cpu + $n }
$o += 'mem_kb=' + $os.TotalVisibleMemorySize + ' ' + $os.FreePhysicalMemory + $n
$o += 'swap_kb=' + $os.SizeStoredInPagingFiles + ' ' + $os.FreeSpaceInPagingFiles + $n
foreach ($d in @(Get-CimInstance -ClassName Win32_LogicalDisk -Filter 'DriveType=3')) {
  if ($d.Size -gt 0) {
    $label = ([string]$d.VolumeName) -replace '[\t\r\n]', ' '
    $o += 'disk=' + [long]($d.Size / 1024) + $t + [long](($d.Size - $d.FreeSpace) / 1024) + $t + [long]($d.FreeSpace / 1024) + $t + $d.FileSystem + $t + $label + $t + $d.DeviceID + $n
  }
}
$o += '@@end' + $n
$b = [System.Text.Encoding]::UTF8.GetBytes($o)
$s = [System.Console]::OpenStandardOutput()
$s.Write($b, 0, $b.Length)
$s.Flush()
