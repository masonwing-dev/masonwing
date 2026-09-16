import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    setupFiles: ['./tests/web/setup.ts'],
    include: ['tests/web/**/*.test.{ts,tsx}'],
    maxWorkers: 2,
    restoreMocks: true,
  },
});
