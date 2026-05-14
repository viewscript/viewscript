import { defineConfig } from 'vite';
import viewScript from '@viewscript/vite-plugin';

export default defineConfig({
  plugins: [
    viewScript({
      // Entity map is auto-generated from .vs files when parsing is implemented.
      // For now, this demonstrates the plugin configuration.
      entityMap: {},
    }),
  ],
  // Enable top-level await for async mount()
  esbuild: {
    target: 'es2022',
  },
  build: {
    target: 'es2022',
  },
});
