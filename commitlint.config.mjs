export default {
  extends: ["@commitlint/config-conventional"],
  rules: {
    "type-enum": [
      2,
      "always",
      [
        "feat",
        "fix",
        "docs",
        "style",
        "refactor",
        "perf",
        "test",
        "chore",
        "ci",
        "build",
      ],
    ],
    // Rust convention uses PascalCase type names in commit subjects (e.g. ValidSchema, FieldKey).
    "subject-case": [0],
    "body-max-line-length": [0],
  },
};
