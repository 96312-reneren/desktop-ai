@echo off
setlocal

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
if exist "%OUT%\desktopai.keystore" copy /y "%OUT%\desktopai.keystore" "%TEMP%\desktopai.keystore" >nul
if exist "%OUT%" rmdir /s /q "%OUT%"
mkdir "%OUT%\classes"
mkdir "%OUT%\lib\arm64-v8a"
if exist "%TEMP%\desktopai.keystore" copy /y "%TEMP%\desktopai.keystore" "%OUT%\desktopai.keystore" >nul

echo == 2. kotlin compile ==
call "%KOTLINC%" "%SRC%\kotlin\com\desktopai\android\MainActivity.kt" -classpath "%PLATFORM%\android.jar" -d "%OUT%\classes"
if errorlevel 1 exit /b 1

echo == 3. dex ==
python "%~dp0gen-classlist.py" "%OUT%\classes" "%OUT%\classes.list"
call "%BT%\d8.bat" --lib "%PLATFORM%\android.jar" --release --output "%OUT%" --classpath "%OUT%\classes" @%OUT%\classes.list "E:\kotlinc\lib\kotlin-stdlib.jar"
if not exist "%OUT%\classes.dex" ( echo DEX_FAILED & exit /b 1 )

echo == 4. aapt2 ==
"%BT%\aapt2.exe" link -o "%OUT%\base.apk" --manifest "%SRC%\AndroidManifest.xml" -I "%PLATFORM%\android.jar" --min-sdk-version 24 --target-sdk-version 34
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
if not exist "%OUT%\desktopai.keystore" (
  "%JAVA%\keytool.exe" -genkeypair -v -keystore "%OUT%\desktopai.keystore" -alias desktopai -keyalg RSA -keysize 2048 -validity 10000 -storepass desktopai123 -keypass desktopai123 -dname "CN=DesktopAI" >nul
)
"%BT%\zipalign.exe" -f 4 "%OUT%\base.apk" "%OUT%\aligned.apk"
call "%BT%\apksigner.bat" sign --ks "%OUT%\desktopai.keystore" --ks-pass pass:desktopai123 --key-pass pass:desktopai123 --out "%OUT%\DesktopAI-v6.1.4.apk" "%OUT%\aligned.apk"
if errorlevel 1 exit /b 1

echo == 7. result ==
dir "%OUT%\DesktopAI-v6.1.4.apk"
echo BUILD_OK
