import type { CSSProperties } from "react";

export type RotationStatus = "healthy" | "due-for-rotation" | "expired" | "failed";

export interface RotationIndicatorProps {
  status: RotationStatus;
  size?: "sm" | "md";
}

interface StatusConfig {
  label: string;
  color: string;
  backgroundColor: string;
  borderColor: string;
}

const STATUS_CONFIGS: Record<RotationStatus, StatusConfig> = {
  healthy: {
    label: "Healthy",
    color: "#9de7ca",
    backgroundColor: "rgba(61, 167, 127, 0.15)",
    borderColor: "rgba(61, 167, 127, 0.35)",
  },
  "due-for-rotation": {
    label: "Due for Rotation",
    color: "#ffd89c",
    backgroundColor: "rgba(255, 169, 77, 0.15)",
    borderColor: "rgba(255, 169, 77, 0.35)",
  },
  expired: {
    label: "Expired",
    color: "#ffb7b7",
    backgroundColor: "rgba(255, 92, 92, 0.15)",
    borderColor: "rgba(255, 92, 92, 0.35)",
  },
  failed: {
    label: "Failed",
    color: "#ff9999",
    backgroundColor: "rgba(220, 38, 38, 0.15)",
    borderColor: "rgba(220, 38, 38, 0.35)",
  },
};

export function RotationIndicator({ status, size = "md" }: RotationIndicatorProps) {
  const config = STATUS_CONFIGS[status];
  const isSmall = size === "sm";

  const containerStyle: CSSProperties = {
    display: "inline-flex",
    alignItems: "center",
    gap: isSmall ? 4 : 6,
    padding: isSmall ? "3px 8px" : "5px 10px",
    borderRadius: isSmall ? 6 : 8,
    border: `1px solid ${config.borderColor}`,
    backgroundColor: config.backgroundColor,
    fontSize: isSmall ? 11 : 12,
    fontWeight: 600,
    color: config.color,
    letterSpacing: 0.2,
  };

  const dotStyle: CSSProperties = {
    width: isSmall ? 5 : 6,
    height: isSmall ? 5 : 6,
    borderRadius: "50%",
    backgroundColor: config.color,
    flexShrink: 0,
  };

  return (
    <span style={containerStyle}>
      <span style={dotStyle} />
      <span>{config.label}</span>
    </span>
  );
}
