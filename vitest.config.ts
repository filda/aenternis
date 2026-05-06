import { defineConfig } from 'vitest/config';

// Vitest configuration for Aenternis production code.
//
// Scope:
//   - tests/**/*.test.ts  -> unit tests for src/**/*.ts (production-track code)
//   - prototypes/**       -> excluded; the prototypes are throwaway lab experiments
//                            and are explicitly not part of the test gate.
//
// Coverage is collected with v8 over src/ only. Targets are kept high because
// the production codebase is small and intended to stay tightly tested.

export default defineConfig({
  test: {
    include: ['tests/**/*.test.ts'],
    environment: 'node',
    coverage: {
      provider: 'v8',
      include: ['src/**/*.ts'],
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
