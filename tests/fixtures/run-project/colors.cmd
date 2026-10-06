@echo off
rem Run panel fixture: a red line, a pause, then a line on stderr.
echo [31mred line[0m
ping -n 3 127.0.0.1 >nul
echo second line 1>&2
