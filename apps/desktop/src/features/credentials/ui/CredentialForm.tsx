import { type CSSProperties, type ChangeEvent, useState } from "react";
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

  /**
   * Handle protocol type selection
   */
  function handleKindSelect(kind: CredentialKind) {
    setSelectedKind(kind);
    setFormData({});
    setValidationErrors([]);
    setTouchedFields(new Set());
  }

  /**
   * Handle field value change
   */
  function handleFieldChange(fieldName: string, value: unknown) {
    const newFormData = { ...formData, [fieldName]: value };
    setFormData(newFormData);

    // Mark field as touched
    setTouchedFields((prev) => new Set([...prev, fieldName]));

    // Validate on change if field was already touched
    if (touchedFields.has(fieldName) && selectedKind) {
      const errors = validateCredentialData(selectedKind, newFormData);
      setValidationErrors(errors);
    }
  }

  /**
   * Handle field blur (mark as touched)
   */
  function handleFieldBlur(fieldName: string) {
    setTouchedFields((prev) => new Set([...prev, fieldName]));

    // Validate on blur
    if (selectedKind) {
      const errors = validateCredentialData(selectedKind, formData);
      setValidationErrors(errors);
    }
  }

  /**
   * Handle form submission
   */
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

  // Styles
  const containerStyle: CSSProperties = {
    width: "100%",
    height: "100%",
    display: "flex",
    flexDirection: "column",
    background: "rgba(14, 20, 38, 0.82)",
    border: "1px solid rgba(151, 165, 198, 0.2)",
    borderRadius: 14,
    overflow: "hidden",
  };

  const headerStyle: CSSProperties = {
    padding: "16px 20px",
    borderBottom: "1px solid rgba(151, 165, 198, 0.15)",
    background: "rgba(14, 20, 38, 0.5)",
  };

  const titleStyle: CSSProperties = {
    margin: 0,
    fontSize: 18,
    fontWeight: 600,
    color: "#edf2ff",
    letterSpacing: 0.3,
  };

  const subtitleStyle: CSSProperties = {
    marginTop: 4,
    fontSize: 13,
    color: "#b8c5e6",
  };

  const formContentStyle: CSSProperties = {
    flex: 1,
    overflow: "auto",
    padding: "20px 24px",
  };

  const sectionTitleStyle: CSSProperties = {
    fontSize: 13,
    fontWeight: 600,
    color: "#8ea0cf",
    textTransform: "uppercase",
    letterSpacing: 0.5,
    marginBottom: 12,
    marginTop: 0,
  };

  const protocolGridStyle: CSSProperties = {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fill, minmax(160px, 1fr))",
    gap: 12,
    marginBottom: 24,
  };

  const footerStyle: CSSProperties = {
    padding: "16px 20px",
    borderTop: "1px solid rgba(151, 165, 198, 0.15)",
    background: "rgba(14, 20, 38, 0.5)",
    display: "flex",
    justifyContent: "flex-end",
    gap: 12,
  };

  const buttonStyle: CSSProperties = {
    padding: "9px 18px",
    borderRadius: 8,
    border: "1px solid rgba(184, 197, 230, 0.35)",
    background: "transparent",
    color: "#edf2ff",
    fontWeight: 600,
    fontSize: 13,
    cursor: "pointer",
    transition: "all 0.15s ease",
  };

  const primaryButtonStyle: CSSProperties = {
    ...buttonStyle,
    background: "rgba(99, 128, 255, 0.25)",
    border: "1px solid rgba(99, 128, 255, 0.5)",
  };

  const errorContainerStyle: CSSProperties = {
    marginTop: 16,
    padding: "12px 16px",
    borderRadius: 8,
    background: "rgba(255, 92, 92, 0.15)",
    border: "1px solid rgba(255, 92, 92, 0.35)",
  };

  const errorListStyle: CSSProperties = {
    margin: 0,
    paddingLeft: 20,
    color: "#ffb7b7",
    fontSize: 13,
  };

  return (
    <form onSubmit={handleSubmit} style={containerStyle}>
      <div style={headerStyle}>
        <h2 style={titleStyle}>{initialData ? "Edit Credential" : "New Credential"}</h2>
        <p style={subtitleStyle}>
          {selectedSchema
            ? `Configure ${selectedSchema.displayName} credential`
            : "Select a protocol type to get started"}
        </p>
      </div>

      <div style={formContentStyle}>
        {/* Protocol Type Selector */}
        <h3 style={sectionTitleStyle}>Protocol Type</h3>
        <div style={protocolGridStyle}>
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
            <h3 style={sectionTitleStyle}>Credential Details</h3>
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
            <h3 style={{ ...sectionTitleStyle, marginTop: 20 }}>
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
          <div style={errorContainerStyle}>
            <ul style={errorListStyle}>
              {validationErrors.map((error) => (
                <li key={error}>{error}</li>
              ))}
            </ul>
          </div>
        )}
      </div>

      <div style={footerStyle}>
        <button type="button" onClick={onCancel} disabled={isSubmitting} style={buttonStyle}>
          Cancel
        </button>
        <button type="submit" disabled={isSubmitting || !selectedKind} style={primaryButtonStyle}>
          {isSubmitting ? "Saving..." : initialData ? "Update Credential" : "Create Credential"}
        </button>
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

  const cardStyle: CSSProperties = {
    padding: "14px",
    borderRadius: 10,
    border: isSelected
      ? "1.5px solid rgba(99, 128, 255, 0.6)"
      : "1px solid rgba(151, 165, 198, 0.2)",
    background: isSelected ? "rgba(99, 128, 255, 0.12)" : "rgba(14, 20, 38, 0.5)",
    cursor: disabled ? "not-allowed" : "pointer",
    transition: "all 0.15s ease",
    opacity: disabled ? 0.5 : 1,
  };

  const iconStyle: CSSProperties = {
    fontSize: 24,
    marginBottom: 8,
  };

  const nameStyle: CSSProperties = {
    fontSize: 14,
    fontWeight: 600,
    color: "#edf2ff",
    marginBottom: 4,
  };

  const descStyle: CSSProperties = {
    fontSize: 11,
    color: "#b8c5e6",
    lineHeight: 1.4,
  };

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
      style={cardStyle}
      onMouseEnter={(e) => {
        if (!disabled && !isSelected) {
          e.currentTarget.style.background = "rgba(151, 165, 198, 0.08)";
        }
      }}
      onMouseLeave={(e) => {
        if (!isSelected) {
          e.currentTarget.style.background = "rgba(14, 20, 38, 0.5)";
        }
      }}
    >
      <div style={iconStyle}>{icon}</div>
      <div style={nameStyle}>{schema.displayName}</div>
      <div style={descStyle}>{schema.description}</div>
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
  const fieldContainerStyle: CSSProperties = {
    marginBottom: 16,
  };

  const labelStyle: CSSProperties = {
    display: "block",
    fontSize: 13,
    fontWeight: 600,
    color: "#edf2ff",
    marginBottom: 6,
  };

  const requiredStyle: CSSProperties = {
    color: "#ffb7b7",
    marginLeft: 4,
  };

  const inputStyle: CSSProperties = {
    width: "100%",
    padding: "10px 12px",
    borderRadius: 8,
    border: "1px solid rgba(151, 165, 198, 0.3)",
    background: "rgba(14, 20, 38, 0.6)",
    color: "#edf2ff",
    fontSize: 13,
    fontFamily: "inherit",
    outline: "none",
    transition: "border-color 0.15s ease",
    boxSizing: "border-box",
  };

  const textareaStyle: CSSProperties = {
    ...inputStyle,
    minHeight: 80,
    resize: "vertical",
    fontFamily: "monospace",
  };

  const helpTextStyle: CSSProperties = {
    fontSize: 11,
    color: "#8ea0cf",
    marginTop: 4,
  };

  const checkboxContainerStyle: CSSProperties = {
    display: "flex",
    alignItems: "center",
    gap: 8,
  };

  const checkboxStyle: CSSProperties = {
    width: 16,
    height: 16,
    cursor: disabled ? "not-allowed" : "pointer",
  };

  const checkboxLabelStyle: CSSProperties = {
    fontSize: 13,
    color: "#edf2ff",
    cursor: disabled ? "not-allowed" : "pointer",
  };

  return (
    <div style={fieldContainerStyle}>
      {type !== "checkbox" && (
        <label htmlFor={`field-${label}`} style={labelStyle}>
          {label}
          {required && <span style={requiredStyle}>*</span>}
        </label>
      )}

      {type === "select" && options ? (
        <select
          id={`field-${label}`}
          value={value as string}
          onChange={onChange}
          onBlur={onBlur}
          disabled={disabled}
          style={inputStyle}
        >
          {placeholder && (
            <option value="" disabled>
              {placeholder}
            </option>
          )}
          {options.map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
      ) : type === "textarea" ? (
        <textarea
          id={`field-${label}`}
          value={value as string}
          onChange={onChange}
          onBlur={onBlur}
          placeholder={placeholder}
          disabled={disabled}
          style={textareaStyle}
        />
      ) : type === "checkbox" ? (
        <div style={checkboxContainerStyle}>
          <input
            id={`field-${label}`}
            type="checkbox"
            checked={value as boolean}
            onChange={onChange}
            onBlur={onBlur}
            disabled={disabled}
            style={checkboxStyle}
          />
          <label htmlFor={`field-${label}`} style={checkboxLabelStyle}>
            {label}
            {required && <span style={requiredStyle}>*</span>}
          </label>
        </div>
      ) : (
        <input
          id={`field-${label}`}
          type={type}
          value={value as string | number}
          onChange={onChange}
          onBlur={onBlur}
          placeholder={placeholder}
          disabled={disabled}
          style={inputStyle}
        />
      )}

      {helpText && <div style={helpTextStyle}>{helpText}</div>}
    </div>
  );
}
