const tsParser = require("@typescript-eslint/parser");
const ts = require("@typescript-eslint/eslint-plugin");
const reactHooks = require("eslint-plugin-react-hooks");

module.exports = [
  {
    ignores: ["out/**", "src/generated/**"],
  },
  {
    files: ["src/**/*.ts", "src/**/*.tsx", "test/**/*.ts", "e2e/**/*.ts", "*.config.ts"],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        project: ["./tsconfig.node.json", "./tsconfig.web.json", "./tsconfig.e2e.json"],
        tsconfigRootDir: __dirname,
      },
    },
    plugins: { "@typescript-eslint": ts, "react-hooks": reactHooks },
    rules: {
      ...ts.configs.recommended.rules,
      ...ts.configs["recommended-type-checked"].rules,
      ...ts.configs["strict-type-checked"].rules,
      ...ts.configs["stylistic-type-checked"].rules,
      // The renderer must never originate a filesystem path or an executable.
      // These make an accidental `any` or a floating promise a build failure rather than a
      // runtime surprise in a process that spawns children.
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-floating-promises": "error",
      "@typescript-eslint/no-misused-promises": "error",
      "@typescript-eslint/explicit-function-return-type": [
        "error",
        { allowExpressions: true },
      ],
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "error",
      "no-console": ["error", { allow: ["error", "warn"] }],
      eqeqeq: ["error", "always"],
    },
  },
];
