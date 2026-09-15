# JSON Schema Export Extensions

`ValidSchema::json_schema()` emits Draft 2020-12 plus Nebula extensions for
contracts that JSON Schema cannot enforce. These keys are versioned as part of
the schema export contract. They are projections from the admitted schema; they
do not replace `ValidSchema::validate`, expression resolution, secret policy,
loader admission, file scanning, tenant checks, or runtime capability checks.

`SCHEMA_WIRE_VERSION` changes only when the durable schema-definition wire bytes
or their admission semantics change. Adding, removing, or changing an
`x-nebula-*` JSON Schema export key requires this document and the export
snapshot tests to change in the same PR. If a consumer persists exported JSON
Schema as authority, that consumer owns a separate envelope/version and must
reject unsupported extension sets fail-closed.

| Extension | Emitted on | Meaning |
|---|---|---|
| `x-nebula-field-kind` | every declared property | Nebula field family used for runtime admission. Unknown kinds remain opaque and must not be downgraded to string/any. |
| `x-nebula-expression-mode` | every declared property | Whether authored expressions are allowed, forbidden, or required at this exact property. |
| `x-nebula-resolved-value-schema` | expression-capable properties | Literal resolved-data schema after expression evaluation; rendered JSON alone is not proof. |
| `x-nebula-required-mode` | every declared property | Static or conditional requiredness annotation. Conditional modes are runtime policy, not JSON Schema authority. |
| `x-nebula-visibility-mode` | every declared property | Presentation visibility annotation only. Hidden or disabled UI never waives validation. |
| `x-nebula-root-rules` | schema root | Serialized root rules retained as runtime obligations. |
| `x-nebula-read-aliases` | aliased properties | Extra input keys consumed and canonicalized before validation. |
| `x-nebula-emit-as` | projected properties | Output projection key for `to_wire_json`; it is not an accepted input alias. |
| `x-nebula-file-accept` | file properties | MIME accept constraint/hint for file references. Extension hints are never security authority. |
| `x-nebula-file-max-size` | file properties | Maximum admitted file size in bytes. File properties represent handles/references by default, not inline raw bytes. |
| `x-nebula-select-dynamic` | choice properties | Options may come from a loader. Loader output is suggestion data unless separately admitted as authority. |
| `x-nebula-select-multiple` | choice properties | The submitted value is an array of selected items. |
| `x-nebula-select-allow-custom` | choice properties | Values outside the static option set are accepted by runtime validation. |
| `x-nebula-disabled` | static choice option entries | The option should not be offered as a selectable UI value. This is presentation/authoring metadata only; admission still follows the select field contract. |
| `x-nebula-mode-default-variant` | mode/union properties | Default selector used when a mode selector is omitted. |

Presentation annotations such as labels, placeholders, grouping, widgets, and
visibility exist to generate forms, CLIs, SDK docs, and visual editor panels
from the same contract. They never authorize access, suppress requiredness,
turn a password widget into a secret, validate MIME sniffing, grant slot
bindings, or make client-side validation trustworthy.
