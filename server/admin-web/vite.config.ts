import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  base: '/static/admin-app/',
  plugins: [react()],
  build: {
    outDir: '../src/testflying_api/static/admin-app',
    emptyOutDir: true
  },
  test: {
    globals: true
  }
});
