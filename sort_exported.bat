@echo off
setlocal enabledelayedexpansion

:: Input and output directories
set inputDir=exported_frames
set outputDir=sorted_frames

:: Loop through each SWF-named folder
for /d %%A in (%inputDir%\*) do (
    set "swfName=%%~nA"

    :: Loop through each resolution folder inside the SWF folder
    for /d %%B in ("%%A\@*") do (
        set "resolution=%%~nxB"

        :: Create the target folder if it doesn't exist
        if not exist "%outputDir%\!resolution!" mkdir "%outputDir%\!resolution!"

        :: Copy and rename 1_1.png to the target folder
        copy "%%B\1_1.png" "%outputDir%\!resolution!\!swfName!.png"
    )
)

echo Done sorting frames.
pause
