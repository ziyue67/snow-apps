# Linux Debian Build

This branch builds a Tauri Debian package.

## Prerequisites

Install the system build dependencies:

```bash
sudo apt install \
  build-essential pkg-config patchelf \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev libxdo-dev libpipewire-0.3-dev libspa-0.2-dev \
  libgbm-dev
```

The custom Excalidraw fork must be cloned next to this repository and built
before the frontend is installed:

```bash
git clone https://github.com/mg-chao/excalidraw.git ../excalidraw
cd ../excalidraw
yarn install
yarn build:packages
```

Create the ONNX Runtime linker name expected by `ort-sys`:

```bash
mkdir -p src-tauri/lib
ln -sf "$(readlink -f /usr/lib/x86_64-linux-gnu/libonnxruntime.so.1.23)" \
  src-tauri/lib/libonnxruntime.so
```

## Build

```bash
pnpm install
pnpm tauri build --bundles deb
```

The package is written to:

```text
src-tauri/target/release/bundle/deb/
```
