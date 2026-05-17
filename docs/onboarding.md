# Developer Onboarding

## Prerequisites

- Node.js >= 20
- pnpm >= 9
- Rust >= 1.77 (stable)
- Python >= 3.10 (for the Headroom compression subprocess in Week 3)

### Tauri system dependencies

**macOS:** Xcode Command Line Tools (`xcode-select --install`).

**Windows:** Microsoft Visual Studio C++ Build Tools and WebView2 runtime
(WebView2 ships with Windows 11 and modern Windows 10).

**Linux (Ubuntu/Debian):**

```bash
sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

### Tauri CLI (one-time, optional)

The `@tauri-apps/cli` npm package is installed as a dev dependency, so
`pnpm tauri:dev` works out of the box. If you prefer the global Rust CLI:

```bash
cargo install tauri-cli --version "^2.0"
```

## Setup

```bash
git clone https://github.com/sessiongraph/sessiongraph-core
cd sessiongraph-core
pnpm install
pnpm tauri:dev
```

## End-user onboarding

For the in-app 4-step setup wizard the desktop app presents to end users,
see spec section 6.6.
