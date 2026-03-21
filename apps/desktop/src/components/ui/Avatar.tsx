import { type ImgHTMLAttributes, forwardRef, useState } from "react";

const sizeClasses = {
  sm: "h-8 w-8 text-xs",
  md: "h-10 w-10 text-sm",
  lg: "h-12 w-12 text-base",
} as const;

type AvatarSize = keyof typeof sizeClasses;

interface AvatarProps extends Omit<ImgHTMLAttributes<HTMLImageElement>, "size"> {
  name: string;
  size?: AvatarSize;
}

function getInitials(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0].toUpperCase())
    .join("");
}

const Avatar = forwardRef<HTMLDivElement, AvatarProps>(
  ({ name, size = "md", src, alt, className = "", ...props }, ref) => {
    const [imgError, setImgError] = useState(false);
    const showImage = src && !imgError;

    return (
      <div
        ref={ref}
        className={`inline-flex shrink-0 items-center justify-center rounded-full bg-[var(--surface-secondary)] text-[var(--text-secondary)] font-medium overflow-hidden ${sizeClasses[size]} ${className}`}
        aria-label={alt ?? name}
      >
        {showImage ? (
          <img
            src={src}
            alt={alt ?? name}
            className="h-full w-full object-cover"
            onError={() => setImgError(true)}
            {...props}
          />
        ) : (
          <span aria-hidden="true">{getInitials(name)}</span>
        )}
      </div>
    );
  },
);

Avatar.displayName = "Avatar";

export { Avatar, type AvatarProps, type AvatarSize };
