# Validator Compatibility Fixtures

This directory stores versioned contract fixtures used by `tests/contract`.

## File Contract

- `minor_contract_v1.json`: Baseline fixtures for minor-release compatibility.

Each fixture entry defines:

- `id`: Stable fixture identifier
- `scenario`: Human-readable scenario name
- `input`: Serialized validation input
- `expected.pass`: Whether validation should pass
- `expected.error_code`: Stable error code when failure is expected
- `expected.field_path`: Stable field path when applicable

These fixtures are used to detect behavioral drift in:

- validator outcomes (`pass`/`fail`)
- error codes
- field-path formatting
