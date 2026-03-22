import { Badge, type BadgeVariant } from "@/components/ui/Badge";

export type RotationStatus = "healthy" | "due-for-rotation" | "expired" | "failed";

export interface RotationIndicatorProps {
  status: RotationStatus;
  size?: "sm" | "md";
}

interface StatusConfig {
  label: string;
  variant: BadgeVariant;
}

const STATUS_CONFIGS: Record<RotationStatus, StatusConfig> = {
  healthy: {
    label: "Healthy",
    variant: "success",
  },
  "due-for-rotation": {
    label: "Due for Rotation",
    variant: "warning",
  },
  expired: {
    label: "Expired",
    variant: "error",
  },
  failed: {
    label: "Failed",
    variant: "error",
  },
};

export function RotationIndicator({ status, size = "md" }: RotationIndicatorProps) {
  const config = STATUS_CONFIGS[status];
  const dotSize = size === "sm" ? "size-1.5" : "size-2";

  return (
    <Badge variant={config.variant} size={size} className="gap-1.5 font-semibold tracking-wide">
      <span className={`${dotSize} rounded-full bg-current shrink-0`} />
      <span>{config.label}</span>
    </Badge>
  );
}
