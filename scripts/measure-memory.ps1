# Sums memory for an app's whole process tree (main process + all children,
# e.g. Electron/WebView2 renderer, GPU and utility processes).
# "Private WS" matches Task Manager's "Memory" column.
param([string[]]$Names = @('Discord', 'rustcord'))

$perf = @{}
Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | ForEach-Object { $perf[[int]$_.IDProcess] = $_ }
$all = Get-CimInstance Win32_Process
$children = @{}
foreach ($p in $all) {
    $ppid = [int]$p.ParentProcessId
    if (-not $children.ContainsKey($ppid)) { $children[$ppid] = @() }
    $children[$ppid] += $p
}

function Get-Tree([int]$id) {
    $id
    foreach ($c in $children[$id]) { Get-Tree ([int]$c.ProcessId) }
}

foreach ($name in $Names) {
    $roots = $all | Where-Object { $_.Name -eq "$name.exe" }
    $rootIds = $roots | ForEach-Object { [int]$_.ProcessId }
    # Only top-level instances of this exe (children of the same exe are part of the tree).
    $tops = $roots | Where-Object { $rootIds -notcontains [int]$_.ParentProcessId }
    if (-not $tops) { Write-Output ("{0,-10} not running" -f $name); continue }
    $ids = $tops | ForEach-Object { Get-Tree ([int]$_.ProcessId) } | Sort-Object -Unique
    $priv = 0; $ws = 0
    foreach ($id in $ids) {
        if ($perf.ContainsKey($id)) { $priv += [int64]$perf[$id].WorkingSetPrivate; $ws += [int64]$perf[$id].WorkingSet }
    }
    Write-Output ("{0,-10} processes={1,2}  privateWS={2,6:N0} MB  workingSet={3,6:N0} MB" -f $name, $ids.Count, ($priv / 1MB), ($ws / 1MB))
}
