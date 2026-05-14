### 1. Existing npm packages
--- packages/browser-defaults/package.json ---
  "name": "@viewscript/browser-defaults",
  "version": "0.1.0",
  "main": "dist/index.js",
  "types": "dist/index.d.ts",

--- packages/renderer/package.json ---
  "name": "@viewscript/renderer",
  "version": "0.1.0",
  "main": "dist/index.js",
  "types": "dist/index.d.ts",

--- crates/vsc-wasm/pkg/package.json ---
  "name": "vsc-wasm",
  "version": "0.1.0",
  "files": [
  "main": "vsc_wasm.js",
  "types": "vsc_wasm.d.ts",

### 2. wasm-pack generated package
{
  "name": "vsc-wasm",
  "type": "module",
  "version": "0.1.0",
  "license": "MIT OR Apache-2.0",
  "files": [
    "vsc_wasm_bg.wasm",
    "vsc_wasm.js",
    "vsc_wasm.d.ts"
  ],
  "main": "vsc_wasm.js",
  "types": "vsc_wasm.d.ts",
  "sideEffects": [
    "./snippets/*"
  ]
}
### 3. pnpm workspace
packages:
  - 'packages/*'

### 4. Root package.json
{
  "name": "viewscript",
  "private": true,
  "packageManager": "pnpm@9.1.0",
  "scripts": {
    "build": "just build",
    "test": "just test",
    "lint": "just lint"
  }
}
