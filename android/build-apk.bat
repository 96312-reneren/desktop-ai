@echo off
rem ══════════════════════════════════════════════════════════════
rem 桌面AI — 安卓 APK 构建(零 Gradle:aapt2 + d8 + kotlinc + apksigner)
rem
rem 前置条件(见 android/README.md):
rem   1. desktop-ai 已用 cargo ndk 编译出 libdesktop_ai.so(路径见 NDKOUT)
rem   2. llama.cpp 的 4 个 .so(libllama / libggml* / libomp)在 LLAMA_LIB_DIR
rem   3. JDK 17、kotlinc、Android SDK build-tools/platform 已安装
rem
rem 所有路径可用环境变量覆盖(JAVA_HOME / KOTLINC / ANDROID_HOME /
rem NDKOUT / LLAMA_LIB_DIR),默认值只是作者机器的便捷设置。
rem ══════════════════════════════════════════════════════════════
setlocal enabledelayedexpansion

rem ── 工具路径(环境变量可覆盖)──
if not defined JAVA_HOME set "JAVA_HOME=E:\Java\jdk-17.0.20+8"
if not defined KOTLIN_HOME set "KOTLIN_HOME=E:\kotlinc"
if not defined KOTLINC set "KOTLINC=%KOTLIN_HOME%\bin\kotlinc.bat"
if not defined ANDROID_HOME set "ANDROID_HOME=E:\AndroidSDK"
set "BT=%ANDROID_HOME%\build-tools\34.0.0"
set "PLATFORM=%ANDROID_HOME%\platforms\android-34"
set "PATH=%JAVA_HOME%\bin;%PATH%"

rem ── 构建输入(cargo ndk / llama.cpp 产物位置,环境变量可覆盖)──
if not defined NDKOUT set "NDKOUT=C:\Users\TheUn\AppData\Local\Temp\opencode\androidout\jni\arm64-v8a"
if not defined LLAMA_LIB_DIR set "LLAMA_LIB_DIR=C:\Users\TheUn\AppData\Local\Temp\opencode\androidout"

rem ── 版本号:从 Cargo.toml 读取(避免与 Cargo.toml 漂移)──
set "VER="
for /f "tokens=3" %%v in ('findstr /b "version = " "%~dp0..\desktop-ai\Cargo.toml"') do (
  if not defined VER set "VER=%%~v"
)
if not defined VER set "VER=0.0.0"

set "OUT=%~dp0build"
set "SRC=%~dp0app\src\main"
set "KEYSTORE=%USERPROFILE%\.desktopai-android\desktopai.keystore"
set "APK=DesktopAI-v%VER%.apk"

echo == 1. clean (version %VER%) ==
if exist "%OUT%" rmdir /s /q "%OUT%"
mkdir "%OUT%\classes"
mkdir "%OUT%\lib\arm64-v8a"

echo == 2. kotlin compile ==
call "%KOTLINC%" "%SRC%\kotlin\com\desktopai\android\MainActivity.kt" -classpath "%PLATFORM%\android.jar" -d "%OUT%\classes"
if errorlevel 1 exit /b 1

echo == 3. dex ==
python "%~dp0gen-classlist.py" "%OUT%\classes" "%OUT%\classes.list"
call "%BT%\d8.bat" --lib "%PLATFORM%\android.jar" --release --output "%OUT%" --classpath "%OUT%\classes" @%OUT%\classes.list "%KOTLIN_HOME%\lib\kotlin-stdlib.jar"
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
copy /y "%LLAMA_LIB_DIR%\libllama.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA_LIB_DIR%\libggml.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA_LIB_DIR%\libggml-base.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA_LIB_DIR%\libggml-cpu.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA_LIB_DIR%\libomp.so" "%OUT%\lib\arm64-v8a\" >nul
python "%~dp0add-files.py" "%OUT%" "%SRC%"

echo == 6. align+sign ==
rem keystore + random password live outside the repo (see keystore-mgr.py);
rem never hardcode the signing password in this file.
python "%~dp0keystore-mgr.py" ensure
for /f "delims=" %%p in ('python "%~dp0keystore-mgr.py" pass') do set "KSPASS=%%p"
"%BT%\zipalign.exe" -f 4 "%OUT%\base.apk" "%OUT%\aligned.apk"
call "%BT%\apksigner.bat" sign --ks "%KEYSTORE%" --ks-pass pass:%KSPASS% --key-pass pass:%KSPASS% --out "%OUT%\%APK%" "%OUT%\aligned.apk"
if errorlevel 1 exit /b 1

echo == 7. result ==
dir "%OUT%\%APK%"
echo BUILD_OK
