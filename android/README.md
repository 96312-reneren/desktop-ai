# Android 构建说明(零 Gradle)

APK 由 `build-apk.bat` 手工组装:`kotlinc` 编译 Kotlin → `d8` 转 dex →
`aapt2` 链接资源 → `zipalign` + `apksigner` 签名。不需要 Gradle。

## 前置条件

| 组件 | 版本 | 说明 |
|------|------|------|
| JDK | 17 | `JAVA_HOME`,供 kotlinc / keytool / apksigner 使用 |
| Kotlin 编译器 | 2.1.x | `KOTLIN_HOME`(含 `bin/kotlinc.bat` 与 `lib/kotlin-stdlib.jar`) |
| Android SDK | build-tools 34.0.0 + platform android-34 | `ANDROID_HOME` |
| Android NDK | 25.2.9519653 | 交叉编译 Rust 与运行时库 |

所有路径都可用环境变量覆盖(脚本内默认值只是作者机器的设置):
`JAVA_HOME`、`KOTLIN_HOME`、`KOTLINC`、`ANDROID_HOME`、`NDKOUT`、`LLAMA_LIB_DIR`。

## 构建输入(两个目录,环境变量可覆盖)

1. **`NDKOUT`** — `cargo ndk` 产出的 `libdesktop_ai.so`
   ```
   cd desktop-ai
   cargo ndk -t arm64-v8a -o <任意输出目录>/jni build --release
   # 然后设置 NDKOUT=<输出目录>/jni/arm64-v8a
   ```

2. **`LLAMA_LIB_DIR`** — llama.cpp 的 Android 运行时库:
   `libllama.so`、`libggml.so`、`libggml-base.so`、`libggml-cpu.so`、`libomp.so`
   - 前 4 个可用 NDK 工具链交叉编译 llama.cpp b7700(`-DCMAKE_TOOLCHAIN_FILE=$NDK/build/cmake/android.toolchain.cmake -DANDROID_ABI=arm64-v8a`)
   - `libomp.so` 直接取 NDK 自带的:
     `<NDK>/toolchains/llvm/prebuilt/windows-x86_64/lib64/clang/14.0.7/lib/linux/aarch64/libomp.so`
   - 历史版本可从 git 历史中恢复:
     `git restore --source=f0d00ee --worktree -- android/build/lib/arm64-v8a`

## 签名

- keystore 与随机口令存放在仓库外(`%USERPROFILE%\.desktopai-android\`,由
  `keystore-mgr.py` 管理),**绝不要**把签名口令写进脚本或提交进仓库。
- keystore 不要删除:签名证书变化后,已安装用户无法覆盖升级。

## 运行

```bat
set NDKOUT=D:\build\jni\arm64-v8a
set LLAMA_LIB_DIR=D:\build\llama-android
build-apk.bat
adb install -r build\DesktopAI-v6.1.6.apk
```

模型放置(无需存储权限):

```
adb push <模型>.gguf /sdcard/Android/data/com.desktopai.android/files/DesktopAI/models/
```

APK 版本号自动读取 `../desktop-ai/Cargo.toml` 的 `version` 字段。
