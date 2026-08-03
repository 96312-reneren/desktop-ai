@echo off
rem ============================================================
rem 桌面AI Android APK 打包脚本 (纯命令行, 无需 Gradle)
rem 前置: desktop-ai 已用 cargo ndk 编译出 libdesktop_ai.so,
rem        ANDROID_HOME / ANDROID_NDK_HOME 已设置
rem ============================================================
setlocal enabledelayedexpansion

set "JAVA_HOME=E:\Java\jdk-17.0.20+8"
set "PATH=%JAVA_HOME%\bin;%PATH%"

set "SDK=%ANDROID_HOME%"
set "BT=%SDK%\build-tools\34.0.0"
set "PLATFORM=%SDK%\platforms\android-34"
set "OUT=%~dp0build"
set "SRC=%~dp0app\src\main"
set "JNI=%OUT%\jniLibs"
set "NDKOUT=C:\Users\TheUn\AppData\Local\Temp\opencode\androidout\jni\arm64-v8a"
set "LLAMA=C:\Users\TheUn\AppData\Local\Temp\opencode\androidout"

echo == 1. clean ==
if exist "%OUT%" rmdir /s /q "%OUT%"
mkdir "%OUT%\lib\arm64-v8a"

echo == 2. compile manifest ==
"%BT%\aapt2.exe" compile --dir "%SRC%\res" -o "%OUT%\res.zip" 2>nul
if not exist "%OUT%\res.zip" echo (no resources - ok)
"%BT%\aapt2.exe" link -o "%OUT%\base.apk" --manifest "%SRC%\AndroidManifest.xml" -I "%PLATFORM%\android.jar" --min-sdk-version 24 --target-sdk-version 34
if errorlevel 1 exit /b 1

echo == 3. add native libs ==
copy /y "%NDKOUT%\libdesktop_ai.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA%\libllama.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA%\libggml.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA%\libggml-base.so" "%OUT%\lib\arm64-v8a\" >nul
copy /y "%LLAMA%\libggml-cpu.so" "%OUT%\lib\arm64-v8a\" >nul

python "%~dp0add-libs.py" "%OUT%"

echo == 4. align + sign ==
if not exist "%OUT%\desktopai.keystore" (
  "E:\Java\jdk-17.0.20+8\bin\keytool.exe" -genkeypair -v -keystore "%OUT%\desktopai.keystore" ^
    -alias desktopai -keyalg RSA -keysize 2048 -validity 10000 ^
    -storepass desktopai123 -keypass desktopai123 -dname "CN=DesktopAI"
)
"%BT%\zipalign.exe" -f 4 "%OUT%\base.apk" "%OUT%\aligned.apk"
"%BT%\apksigner.bat" sign --ks "%OUT%\desktopai.keystore" --ks-pass pass:desktopai123 --key-pass pass:desktopai123 ^
  --out "%OUT%\DesktopAI-v6.1.4.apk" "%OUT%\aligned.apk"
if errorlevel 1 exit /b 1

echo == 5. result ==
dir "%OUT%\DesktopAI-v6.1.4.apk"
echo BUILD_OK
