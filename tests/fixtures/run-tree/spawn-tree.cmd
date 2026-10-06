@echo off
rem Process tree fixture: starts a long-lived grandchild, prints its pid, waits.
powershell -NoProfile -NonInteractive -Command "$p = Start-Process -FilePath ping.exe -ArgumentList '-n 300 127.0.0.1' -WindowStyle Hidden -PassThru; Write-Output ('grandchild ' + $p.Id); $p.WaitForExit()"
