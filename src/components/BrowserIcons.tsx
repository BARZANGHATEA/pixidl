// Simplified, hand-drawn marks used to identify each supported browser.
type P = { size?: number };

export function ChromeIcon({ size = 56 }: P) {
  // Three 120° sectors around a blue core.
  const r = 46;
  const c = 50;
  const pt = (deg: number) => [c + r * Math.cos((deg * Math.PI) / 180), c + r * Math.sin((deg * Math.PI) / 180)];
  const sector = (a: number, b: number) => {
    const [x1, y1] = pt(a);
    const [x2, y2] = pt(b);
    return `M${c} ${c} L${x1.toFixed(2)} ${y1.toFixed(2)} A${r} ${r} 0 0 1 ${x2.toFixed(2)} ${y2.toFixed(2)} Z`;
  };
  return (
    <svg width={size} height={size} viewBox="0 0 100 100" aria-hidden="true">
      <path d={sector(-150, -30)} fill="#EA4335" />
      <path d={sector(-30, 90)} fill="#FBBC04" />
      <path d={sector(90, 210)} fill="#34A853" />
      <circle cx="50" cy="50" r="21" fill="#fff" />
      <circle cx="50" cy="50" r="16" fill="#4285F4" />
    </svg>
  );
}

export function EdgeIcon({ size = 56 }: P) {
  return (
    <svg width={size} height={size} viewBox="0 0 100 100" aria-hidden="true">
      <defs>
        <linearGradient id="edge-a" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#35C1F1" />
          <stop offset="0.55" stopColor="#1B8FD8" />
          <stop offset="1" stopColor="#0C59A4" />
        </linearGradient>
        <linearGradient id="edge-b" x1="0" y1="1" x2="1" y2="0">
          <stop offset="0" stopColor="#2CAC53" />
          <stop offset="1" stopColor="#66D16F" />
        </linearGradient>
      </defs>
      <circle cx="50" cy="50" r="46" fill="url(#edge-a)" />
      <path d="M14 62c6 20 26 33 48 30 14-2 25-10 30-21-9 6-21 8-31 5-12-3-18-12-17-22 1-9 9-15 18-15 10 0 18 6 20 15 5-18-9-35-30-36C30 17 9 38 14 62Z" fill="url(#edge-b)" />
      <path d="M44 54c-1 10 5 19 17 22 10 3 22 1 31-5-6 14-19 22-34 22C38 93 22 80 17 63c4 5 12 8 20 7-1-5 0-11 7-16Z" fill="#0C59A4" opacity="0.35" />
      <circle cx="61" cy="56" r="12" fill="#fff" opacity="0.9" />
    </svg>
  );
}

export function BraveIcon({ size = 56 }: P) {
  return (
    <svg width={size} height={size} viewBox="0 0 100 100" aria-hidden="true">
      <defs>
        <linearGradient id="brave-a" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#FF6A2B" />
          <stop offset="1" stopColor="#F23A1C" />
        </linearGradient>
      </defs>
      <path d="M50 6 66 10l8-2 12 13-3 8 4 13-17 44-20 8-20-8L13 42l4-13-3-8L26 8l8 2Z" fill="url(#brave-a)" />
      <path d="M50 30c6 0 9 2 13-1l6 6-3 9 4 4-9 14-5-2-6 6-6-6-5 2-9-14 4-4-3-9 6-6c4 3 7 1 13 1Z" fill="#fff" />
      <path d="M43 72h14l-7 6Z" fill="#fff" />
      <circle cx="42" cy="50" r="3" fill="#F23A1C" />
      <circle cx="58" cy="50" r="3" fill="#F23A1C" />
    </svg>
  );
}

export function FirefoxIcon({ size = 56 }: P) {
  return (
    <svg width={size} height={size} viewBox="0 0 100 100" aria-hidden="true">
      <defs>
        <radialGradient id="ff-a" cx="0.7" cy="0.15" r="1">
          <stop offset="0" stopColor="#FFE226" />
          <stop offset="0.35" stopColor="#FF9A1F" />
          <stop offset="0.7" stopColor="#FF3750" />
          <stop offset="1" stopColor="#E31587" />
        </radialGradient>
        <radialGradient id="ff-b" cx="0.4" cy="0.4" r="0.7">
          <stop offset="0" stopColor="#9059FF" />
          <stop offset="1" stopColor="#3A1F9C" />
        </radialGradient>
      </defs>
      <circle cx="50" cy="52" r="40" fill="url(#ff-b)" />
      <path d="M90 46c0 25-18 46-41 46S8 74 8 52c0-8 2-15 6-21 0 7 3 12 7 14-1-9 4-19 12-24-2 6 0 11 4 14 4-9 13-15 23-16-4 4-5 9-3 14 9-1 17 6 18 15 1 9-5 17-14 19-12 3-22-4-24-14 3 5 9 7 14 6-7-4-9-12-5-18 7 4 16 2 20-4C80 27 74 18 66 13c13 4 24 17 24 33Z" fill="url(#ff-a)" />
    </svg>
  );
}

export const BROWSER_ICONS = { chrome: ChromeIcon, edge: EdgeIcon, brave: BraveIcon, firefox: FirefoxIcon } as const;
