# Windows Build Setup

If you encounter the error `dlltool.exe: program not found` when building on Windows, follow these steps to install the required build tools.

## Option 1: Install MinGW-w64 (Recommended)

1. Download MSYS2 from https://www.msys2.org/
2. Run the installer and follow the instructions
3. Open MSYS2 terminal and run:
   ```bash
   pacman -S mingw-w64-x86_64-toolchain
   ```
4. Add to your PATH (usually `C:\msys64\mingw64\bin`)

## Option 2: Install via Rust's MSVC toolchain

Alternatively, you can use the MSVC toolchain instead:

1. Install Visual Studio Build Tools:
   - Download from: https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022
   - Select "Desktop development with C++"

2. Set Rust to use MSVC toolchain:
   ```bash
   rustup default stable-msvc
   ```

## Option 3: Install via Chocolatey

If you have Chocolatey installed:

```bash
choco install mingw
```

## Verify Installation

After installing, verify dlltool is available:

```bash
dlltool --version
```

## Then Run the App

Once the build tools are installed:

```bash
cd file-finder
npm run tauri dev
```

## Additional Notes

- The error occurs because some Rust crates need to link with Windows system libraries
- This is a one-time setup required for Tauri development on Windows
- On Linux and macOS, these tools are typically pre-installed or easily available via package managers
