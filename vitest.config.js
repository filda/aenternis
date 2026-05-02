import { defineConfig } from 'vitest/config';

// Vitest configuration for Aenternis production code.
//
// Scope:
//   - tests/**/*.test.js  -> unit tests for src/**/*.js (production-track code)
//   - prototypes/**       -> excluded; the prototypes are throwaway lab experiments
//                            and are explicitly not part of the test gate.
//
// Coverage is collected with v8 over src/ only. Targets are kept high because
// the production codebase is small and intended to stay tightly tested.

export default defineConfig({
  test: {
    include: ['tests/**/*.test.js'],
    environment: 'node',
    coverage: {
      provider: 'v8',
      include: ['src/**/*.js'],
      exclude: [
        'prototypes/**',
        'tests/**',
        'node_modules/**',
        'dist/**',
        '.stryker-tmp/**',
      ],
      reporter: ['text', 'html'],
      reportsDirectory: 'reports/coverage',
      thresholds: {
        lines: 95,
        functions: 95,
        branches: 90,
        statements: 95,
      },
    },
  },
});
