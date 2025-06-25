@echo off
setlocal enabledelayedexpansion

:: Prompt for folder
set /p folder="Enter the path to the folder containing SWF files: "
set /p scale="Enter the scale to export at (usually 2 for Yukon): " 
set /p width="Enter the width of the stage (usually 150 for clothing items): " 

:: Max number of parallel jobs
set maxJobs=2
set jobCount=0

:: Loop through SWF files
for %%f in ("%folder%\*.swf") do (
    echo Processing: %%~nxf
    
    :: Start a new background process (the /b flag keeps it in the same window)
    start "" /b cmd /c (
		:: Copy the SWF file to current working directory
		copy "%%f" "%%~nxf"
        ruffle_desktop.exe --dummy-external-interface --no-gui --export-swf="%%~nxf" --stage-width=%width% --stage-scale=%scale% export-script.swf
        del "%%~nxf"
    )

    :: Increment job counter
    set /a jobCount+=1

    :: If we've reached the max, wait for all jobs to finish
    if !jobCount! geq %maxJobs% (
        call :waitForJobs
        set jobCount=0
    )
)

:: Final wait to make sure all jobs finish
call :waitForJobs

echo Done processing all SWF files.
pause
exit /b

:waitForJobs
:: Wait for all background jobs to finish
:check
timeout /t 1 >nul
:: Count running ruffle_desktop.exe processes
tasklist /fi "imagename eq ruffle_desktop.exe" | find /i "ruffle_desktop.exe" >nul
if not errorlevel 1 goto check
exit /b
