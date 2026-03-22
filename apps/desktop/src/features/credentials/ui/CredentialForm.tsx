import { type ChangeEvent, useState } from "react";
import { Button } from "../../../components/ui/Button";
import { Card } from "../../../components/ui/Card";
import { Input } from "../../../components/ui/Input";
import { Select } from "../../../components/ui/Select";
import {
  CREDENTIAL_SCHEMAS,
  getSchemaByKind,
  validateCredentialData,
} from "../application/schemas";
import type { CredentialFormData, CredentialKind } from "../domain/types";

export interface CredentialFormProps {
  /**
   * Initial form data for editing (undefined for new credential)
   */
  initialData?: Partial<CredentialFormData>;
  /**
   * Callback when form is submitted with valid data
   */
  onSubmit: (data: CredentialFormData) => void;
  /**
   * Callback when form is cancelled
   */
  onCancel: () => void;
  /**
   * Whether the form is in a submitting state
   */
  isSubmitting?: boolean;
}

/**
 * Credential creation and editing form component
 *
 * Features:
 * - Protocol type selector with visual cards
 * - Dynamic field generation based on selected protocol schema
 * - Field-level validation with real-time feedback
 * - Sensitive value masking for passwords and secrets
 * - Follows desktop UI styling patterns
 */
export function CredentialForm({
  initialData,
  onSubmit,
  onCancel,
  isSubmitting = false,
}: CredentialFormProps) {
  const [selectedKind, setSelectedKind] = useState<CredentialKind | null>(
    (initialData?.kind as CredentialKind) ?? null,
  );
  const [formData, setFormData] = useState<Record<string, unknown>>(initialData?.state ?? {});
  const [name, setName] = useState<string>(initialData?.name ?? "");
  const [validationErrors, setValidationErrors] = useState<string[]>([]);
  const [touchedFields, setTouchedFields] = useState<Set<string>>(new Set());

  const selectedSchema = selectedKind ? getSchemaByKind(selectedKind) : null;

  function handleKindSelect(kind: CredentialKind) {
    setSelectedKind(kind);
    setFormData({});
    setValidationErrors([]);
    setTouchedFields(new Set());
  }

  function handleFieldChange(fieldName: string, value: unknown) {
    const newFormData = { ...formData, [fieldName]: value };
    setFormData(newFormData);

    setTouchedFields((prev) => new Set([...prev, fieldName]));

    if (touchedFields.has(fieldName) && selectedKind) {
      const errors = validateCredentialData(selectedKind, newFormData);
      setValidationErrors(errors);
    }
  }

  function handleFieldBlur(fieldName: string) {
    setTouchedFields((prev) => new Set([...prev, fieldName]));

    if (selectedKind) {
      const errors = validateCredentialData(selectedKind, formData);
      setValidationErrors(errors);
    }
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();

    if (!selectedKind) {
      setValidationErrors(["Please select a credential type"]);
      return;
    }

    if (!name.trim()) {
      setValidationErrors(["Credential name is required"]);
      return;
    }

    const errors = validateCredentialData(selectedKind, formData);
    setValidationErrors(errors);

    if (errors.length === 0) {
      onSubmit({
        name: name.trim(),
        kind: selectedKind,
        state: formData,
        tags: initialData?.tags ?? {},
      });
    }
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="flex h-full w-full flex-col overflow-hidden rounded-lg border border-[var(--border-primary)] bg-[var(--surface-primary)]"
    >
      {/* Header */}
      <div className="border-b border-[var(--border-primary)] bg-[var(--surface-primary)]/50 px-5 py-4">
        <h2 className="m-0 text-lg font-semibold tracking-wide text-[var(--text-primary)]">
          {initialData ? "Edit Credential" : "New Credential"}
        </h2>
        <p className="mt-1 text-sm text-[var(--text-secondary)]">
          {selectedSchema
            ? `Configure ${selectedSchema.displayName} credential`
            : "Select a protocol type to get started"}
        </p>
      </div>

      {/* Form Content */}
      <div className="flex-1 overflow-auto px-6 py-5">
        {/* Protocol Type Selector */}
        <h3 className="mb-3 mt-0 text-xs font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">
          Protocol Type
        </h3>
        <div className="mb-6 grid grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-3">
          {CREDENTIAL_SCHEMAS.map((schema) => (
            <ProtocolCard
              key={schema.kind}
              schema={schema}
              isSelected={selectedKind === schema.kind}
              onClick={() => handleKindSelect(schema.kind)}
              disabled={isSubmitting}
            />
          ))}
        </div>

        {/* Credential Name Field */}
        {selectedSchema && (
          <>
            <h3 className="mb-3 mt-0 text-xs font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">
              Credential Details
            </h3>
            <FormField
              label="Credential Name"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onBlur={() => {}}
              placeholder="My API Key"
              helpText="A descriptive name to identify this credential"
              required
              disabled={isSubmitting}
            />

            {/* Dynamic Protocol Fields */}
            <h3 className="mb-3 mt-5 text-xs font-semibold uppercase tracking-wider text-[var(--text-tertiary)]">
              {selectedSchema.displayName} Configuration
            </h3>
            {selectedSchema.fields.map((field) => (
              <FormField
                key={field.name}
                label={field.label}
                type={field.type}
                value={formData[field.name] as string | number | boolean}
                onChange={(e) => {
                  const value =
                    field.type === "checkbox"
                      ? (e.target as HTMLInputElement).checked
                      : field.type === "number"
                        ? (e.target as HTMLInputElement).value
                        : (e.target as HTMLInputElement | HTMLSelectElement).value;
                  handleFieldChange(field.name, value);
                }}
                onBlur={() => handleFieldBlur(field.name)}
                placeholder={field.placeholder}
                helpText={field.helpText}
                required={field.required}
                options={field.options}
                disabled={isSubmitting}
              />
            ))}
          </>
        )}

        {/* Validation Errors */}
        {validationErrors.length > 0 && (
          <Card variant="default" padding="sm" className="mt-4 border-[var(--error)] bg-[var(--error)]/10">
            <ul className="m-0 pl-5 text-sm text-[var(--error)]">
              {validationErrors.map((error) => (
                <li key={error}>{error}</li>
              ))}
            </ul>
          </Card>
        )}
      </div>

      {/* Footer */}
      <div className="flex justify-end gap-3 border-t border-[var(--border-primary)] bg-[var(--surface-primary)]/50 px-5 py-4">
        <Button
          type="button"
          variant="secondary"
          size="md"
          onClick={onCancel}
          disabled={isSubmitting}
        >
          Cancel
        </Button>
        <Button
          type="submit"
          variant="primary"
          size="md"
          loading={isSubmitting}
          disabled={!selectedKind}
        >
          {initialData ? "Update Credential" : "Create Credential"}
        </Button>
      </div>
    </form>
  );
}

/**
 * Protocol selection card component
 */
interface ProtocolCardProps {
  schema: { kind: CredentialKind; displayName: string; description: string; icon?: string };
  isSelected: boolean;
  onClick: () => void;
  disabled?: boolean;
}

function ProtocolCard({ schema, isSelected, onClick, disabled }: ProtocolCardProps) {
  const iconMap: Record<string, string> = {
    key: "🔑",
    user: "👤",
    database: "🗄️",
    shield: "🔐",
    code: "⚙️",
  };

  const icon = schema.icon ? (iconMap[schema.icon] ?? "🔐") : "🔐";

  return (
    <button
      type="button"
      onClick={() => {
        if (!disabled) onClick();
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          if (!disabled) onClick();
        }
      }}
      disabled={disabled}
      className={`rounded-lg border p-3.5 text-left transition-colors disabled:pointer-events-none disabled:opacity-50 ${
        isSelected
          ? "border-[var(--accent)]/60 bg-[var(--accent)]/10"
          : "border-[var(--border-primary)] bg-[var(--surface-secondary)] hover:bg-[var(--surface-hover)]"
      }`}
    >
      <div className="mb-2 text-2xl">{icon}</div>
      <div className="mb-1 text-sm font-semibold text-[var(--text-primary)]">
        {schema.displayName}
      </div>
      <div className="text-xs leading-snug text-[var(--text-secondary)]">
        {schema.description}
      </div>
    </button>
  );
}

/**
 * Dynamic form field component
 */
interface FormFieldProps {
  label: string;
  type: string;
  value: string | number | boolean | undefined;
  onChange: (e: ChangeEvent<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>) => void;
  onBlur: () => void;
  placeholder?: string;
  helpText?: string;
  required?: boolean;
  options?: Array<{ value: string; label: string }>;
  disabled?: boolean;
}

function FormField({
  label,
  type,
  value,
  onChange,
  onBlur,
  placeholder,
  helpText,
  required,
  options,
  disabled,
}: FormFieldProps) {
  const fieldId = `field-${label.toLowerCase().replace(/\s+/g, "-")}`;

  if (type === "select" && options) {
    return (
      <div className="mb-4">
        <Select
          id={fieldId}
          label={`${label}${required ? " *" : ""}`}
          value={value as string}
          onChange={onChange}
          onBlur={onBlur}
          disabled={disabled}
          options={options}
          placeholder={placeholder}
        />
        {helpText && (
          <p className="mt-1 text-xs text-[var(--text-tertiary)]">{helpText}</p>
        )}
      </div>
    );
  }

  if (type === "textarea") {
    return (
      <div className="mb-4 flex flex-col gap-1.5">
        <label htmlFor={fieldId} className="text-sm font-medium text-[var(--text-primary)]">
          {label}
          {required && <span className="ml-1 text-[var(--error)]">*</span>}
        </label>
        <textarea
          id={fieldId}
          value={value as string}
          onChange={onChange}
          onBlur={onBlur}
          placeholder={placeholder}
          disabled={disabled}
          className="min-h-[80px] w-full resize-y rounded-md border border-[var(--border-primary)] bg-[var(--surface-primary)] px-3 py-2 font-mono text-sm text-[var(--text-primary)] placeholder:text-[var(--text-tertiary)] transition-colors focus:outline-none focus:ring-2 focus:ring-[var(--border-focus)] disabled:pointer-events-none disabled:opacity-50"
        />
        {helpText && (
          <p className="text-xs text-[var(--text-tertiary)]">{helpText}</p>
        )}
      </div>
    );
  }

  if (type === "checkbox") {
    return (
      <div className="mb-4 flex items-center gap-2">
        <input
          id={fieldId}
          type="checkbox"
          checked={value as boolean}
          onChange={onChange}
          onBlur={onBlur}
          disabled={disabled}
          className="h-4 w-4 cursor-pointer disabled:cursor-not-allowed"
        />
        <label
          htmlFor={fieldId}
          className="cursor-pointer text-sm text-[var(--text-primary)] disabled:cursor-not-allowed"
        >
          {label}
          {required && <span className="ml-1 text-[var(--error)]">*</span>}
        </label>
        {helpText && (
          <p className="text-xs text-[var(--text-tertiary)]">{helpText}</p>
        )}
      </div>
    );
  }

  return (
    <div className="mb-4">
      <Input
        id={fieldId}
        label={`${label}${required ? " *" : ""}`}
        type={type}
        value={value as string | number}
        onChange={onChange}
        onBlur={onBlur}
        placeholder={placeholder}
        disabled={disabled}
      />
      {helpText && (
        <p className="mt-1 text-xs text-[var(--text-tertiary)]">{helpText}</p>
      )}
    </div>
  );
}
