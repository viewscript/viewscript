import { defineConfig } from 'vite';
import viewScript from '@viewscript/vite-plugin';

export default defineConfig({
  plugins: [
    viewScript({
      // Entity map is auto-generated from .vs files in later versions.
      // For now, manually define entity name -> ID mapping if using triggers.
      entityMap: {},
    }),
  ],
});
