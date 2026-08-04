@echo off
setlocal enabledelayedexpansion

set "JAVA_HOME=E:\Java\jdk-17.0.20+8"
set "PATH=%JAVA_HOME%\bin;%PATH%"
set "JAVA=%JAVA_HOME%\bin"
set "KOTLINC=E:\kotlinc\bin\kotlinc.bat"
set "BT=E:\AndroidSDK\build-tools\34.0.0"
set "PLATFORM=E:\AndroidSDK\platforms\android-34"
set "OUT=%~dp0build"
set "SRC=%~dp0app\src\main"
set "NDKOUT=C:\Users\TheUn\AppData\Local\Temp\opencode\androidout\jni\arm64-v8a"
set "LLAMA=C:\Users\TheUn\AppData\Local\Temp\opencode\androidout"

echo == 1. clean ==
set "KEYSTORE=%USERPROFILE%\.desktopai-android\desktopai.keystore"
if exist "%OUT%" rmdir /s /q "%OUT%"
mkdir "%OUT%\classes"
mkdir "%OUT%\lib\arm64-v8a"

echo == 2. kotlin compile ==
call "%KOTLINC%" "%SRC%\kotlin\com\desktopai\android\MainActivity.kt" -classpath "%PLATFORM%\android.jar" -d "%OUT%\classes"
if errorlevel 1 exit /b 1

echo == 3. dex ==
python "%~dp0gen-classlist.py" "%OUT%\classes" "%OUT%\classes.list"
call "%BT%\d8.bat" --lib "%PLATFORM%\android.jar" --release --output "%OUT%" --classpath "%OUT%\classes" @%OUT%\classes.list "E:\kotlinc\lib\kotlin-stdlib.jar"
if not exist "%OUT%\classes.dex" ( echo DEX_FAILED & exit /b 1 )

echo == 4. aapt2 ==
set "RESFLAT="
if exist "%SRC%\res\xml" (
  mkdir "%OUT%\res" 2>nul
  for /r "%SRC%\res\xml" %%f in (*.xml) do (
    "%BT%\aapt2.exe" compile --legacy -o "%OUT%\res\%%~nf.zip" "%%f"
    set "RESFLAT=!RESFLAT! "%OUT%\res\%%~nf.zip""
  )
)
"%BT%\aapt2.exe" link -o "%OUT%\base.apk" --manifest "%SRC%\AndroidManifest.xml" -I "%PLATFORM%\android.jar" --min-sdk-version 24 --target-sdk-version 34 !RESFLAT!
if errorlevel 1 exit /b 1

echo == 5. add dex+libs+assets ==
copy /y "%NDKOUT%\libdesktop_ai.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA%\libllama.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA%\libggml.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA%\libggml-base.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA%\libggml-cpu.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA%\libomp.so" "%OUT%\lib\arm64-v8a\" >nul
python "%~dp0add-files.py" "%OUT%" "%SRC%"

echo == 6. align+sign ==
rem keystore + random password live outside the repo/build dirs (see
rem keystore-mgr.py); never hardcode the signing password in this file.
python "%~dp0keystore-mgr.py" ensure
for /f "delims=" %%p in ('python "%~dp0keystore-mgr.py" pass') do set "KSPASS=%%p"
"%BT%\zipalign.exe" -f 4 "%OUT%\base.apk" "%OUT%\aligned.apk"
call "%BT%\apksigner.bat" sign --ks "%KEYSTORE%" --ks-pass pass:%KSPASS% --key-pass pass:%KSPASS% --out "%OUT%\DesktopAI-v6.1.6.apk" "%OUT%\aligned.apk"
if errorlevel 1 exit /b 1

echo == 7. result ==
dir "%OUT%\DesktopAI-v6.1.6.apk"
echo BUILD_OK
