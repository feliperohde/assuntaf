import React from "react";

/**
 * Assunta brand mark: an "A" whose crossbar is an audio waveform, on an
 * indigo→violet rounded square. Same artwork as the app icon
 * (src-tauri/icons/source/assunta-icon.svg), cropped to the tile.
 */
export function AssuntaMark({ size = 32, className = "" }: { size?: number; className?: string }) {
  const id = React.useId();
  return (
    <svg
      width={size}
      height={size}
      viewBox="100 100 824 824"
      className={className}
      role="img"
      aria-label="Assunta"
    >
      <defs>
        <linearGradient id={`${id}-bg`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#4F46E5" />
          <stop offset="1" stopColor="#8B5CF6" />
        </linearGradient>
      </defs>
      <rect x="100" y="100" width="824" height="824" rx="190" fill={`url(#${id}-bg)`} />
      <path
        d="M322 774 L512 262 L702 774"
        fill="none"
        stroke="#FFFFFF"
        strokeWidth="92"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <g fill="#FFFFFF">
        <rect x="442" y="640" width="32" height="60" rx="16" />
        <rect x="496" y="600" width="32" height="140" rx="16" />
        <rect x="550" y="640" width="32" height="60" rx="16" />
      </g>
    </svg>
  );
}

/** Mark + "Assunta" wordmark. */
export function AssuntaWordmark({ size = 28, className = "" }: { size?: number; className?: string }) {
  return (
    <span className={`inline-flex items-center gap-2 ${className}`}>
      <AssuntaMark size={size} />
      <span
        className="font-semibold tracking-tight bg-gradient-to-r from-indigo-600 to-violet-500 bg-clip-text text-transparent"
        style={{ fontSize: size * 0.72 }}
      >
        Assunta
      </span>
    </span>
  );
}
