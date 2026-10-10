// Official browser logos (128 px), used only to identify each browser.
import chrome from "../assets/browsers/chrome.png";
import edge from "../assets/browsers/edge.png";
import brave from "../assets/browsers/brave.png";
import firefox from "../assets/browsers/firefox.png";

type P = { size?: number };

function logo(src: string) {
  return function BrowserLogo({ size = 56 }: P) {
    return <img src={src} width={size} height={size} alt="" aria-hidden="true" draggable={false} style={{ objectFit: "contain" }} />;
  };
}

export const ChromeIcon = logo(chrome);
export const EdgeIcon = logo(edge);
export const BraveIcon = logo(brave);
export const FirefoxIcon = logo(firefox);

export const BROWSER_ICONS = {
  chrome: ChromeIcon,
  edge: EdgeIcon,
  brave: BraveIcon,
  firefox: FirefoxIcon,
} as const;
