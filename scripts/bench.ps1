# Run from an ADMIN PowerShell: powershell -ExecutionPolicy Bypass -File scripts\bench.ps1 [term]
# Compares our rs.exe / Task manager.exe with Windows Search / Taskmgr.exe. Changes nothing on the system.
param([string]$term = 'notepad')
$root = Split-Path $PSScriptRoot
$rs = Join-Path $root 'payload\rs.exe'
$tm = Join-Path $root 'payload\Task manager.exe'
function Mem($exe, $name) {
    $p = Start-Process $exe -PassThru; Start-Sleep 5; $p.Refresh()
    $o = "{0}: WS {1} MB, private {2} MB, CPU {3:N1}s" -f $name, [math]::Round($p.WorkingSet64/1MB,1), [math]::Round($p.PrivateMemorySize64/1MB,1), $p.TotalProcessorTime.TotalSeconds
    Stop-Process $p -Force; $o
}
"== Search for '$term' =="
1..3 | ForEach-Object {
    $t = Measure-Command { $n = (& $rs $term | Measure-Object -Line).Lines }
    "rs.exe run $_ : {0:N0} ms, {1} lines" -f $t.TotalMilliseconds, $n
}
try {
    $c = New-Object -ComObject ADODB.Connection
    $c.Open("Provider=Search.CollatorDSO;Extended Properties='Application=Windows';")
    1..3 | ForEach-Object {
        $t = Measure-Command { $r = $c.Execute("SELECT System.ItemPath FROM SystemIndex WHERE System.FileName LIKE '%$term%'"); $k = 0; while (-not $r.EOF) { $k++; $r.MoveNext() } }
        "Windows Search run $_ : {0:N0} ms, {1} rows" -f $t.TotalMilliseconds, $k
    }
} catch { "Windows Search: unavailable ($($_.Exception.Message))" }
"== Task manager =="
Mem $tm 'ours'
Mem 'C:\Windows\System32\Taskmgr.exe' 'Windows Taskmgr'
