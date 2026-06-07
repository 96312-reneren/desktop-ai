@echo off
chcp 65001 >nul
setlocal enabledelayedexpansion

echo ============================================
echo   桌面AI 打包脚本
echo ============================================
echo.

:: ─── 定位 Python ─────────────────────────────
set PYTHON=
if exist "venv\Scripts\python.exe" (
    set PYTHON=venv\Scripts\python.exe
    echo [1/5] 使用虚拟环境: venv\
) else (
    echo [错误] 未找到 venv\Scripts\python.exe
    echo   请先运行:
    echo     python -m venv venv
    echo     venv\Scripts\python -m pip install llama-cpp-python --extra-index-url https://abetlen.github.io/llama-cpp-python/whl/cpu
    echo     venv\Scripts\python -m pip install -r requirements.txt
    pause
    exit /b 1
)
echo.

:: ─── 检查/安装依赖 ────────────────────────────
echo [2/5] 检查依赖...
%PYTHON% -c "import customtkinter, llama_cpp, requests, markdown2" 2>nul
if %errorlevel% neq 0 (
    echo   依赖缺失，正在安装...
    %PYTHON% -m pip install llama-cpp-python --extra-index-url https://abetlen.github.io/llama-cpp-python/whl/cpu
    %PYTHON% -m pip install -r requirements.txt
)

echo   安装 PyInstaller...
%PYTHON% -m pip install pyinstaller -q
echo.

:: ─── 检查图标 ────────────────────────────────
set ICON_ARG=
if exist "assets\icon.ico" (
    set ICON_ARG=--icon=assets\icon.ico
    echo [3/5] 图标: assets\icon.ico
) else (
    echo [3/5] 未找到图标，使用默认
)
echo.

:: ─── 清理 ────────────────────────────────────
echo [4/5] 清理旧文件...
if exist "build" rmdir /s /q "build"
if exist "dist"  rmdir /s /q "dist"
echo.

:: ─── 打包 ────────────────────────────────────
echo [5/5] 开始打包（约 2-5 分钟）...
echo.

%PYTHON% -m PyInstaller ^
    --windowed ^
    --name 桌面AI ^
    %ICON_ARG% ^
    --add-data "config.json;." ^
    --add-data "assets;assets" ^
    --collect-all llama_cpp ^
    --collect-all customtkinter ^
    --hidden-import=requests ^
    --hidden-import=markdown2 ^
    --hidden-import=llama_cpp ^
    --hidden-import=customtkinter ^
    --noconfirm ^
    --clean ^
    main.py

if %errorlevel% neq 0 (
    echo.
    echo [错误] 打包失败！
    pause
    exit /b 1
)

echo.
echo ============================================
echo   ^^✓ 打包成功！
echo.
echo   输出: dist\桌面AI\
echo   主程序: dist\桌面AI\桌面AI.exe
echo.
echo   EXE 文件仅 ~20MB，模型由用户首次启动时选择下载。
echo   分发方式: 压缩 dist\桌面AI 文件夹即可。
echo ============================================

echo.
choice /c yn /m "是否打开输出目录"
if %errorlevel% equ 2 (
    start "" "dist\桌面AI"
)

pause
endlocal
