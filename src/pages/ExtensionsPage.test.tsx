import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ExtensionsPage, connectionState } from "./ExtensionsPage";
import { api } from "../services/api";
import type { ExtensionBrowser } from "../types";

vi.mock("../services/api", () => ({
  api: { getExtensionBrowsers: vi.fn(), getExtension: vi.fn(), openBrowserExtensionsPage: vi.fn(() => Promise.resolve()), revealPath: vi.fn(() => Promise.resolve()) },
}));

const now = new Date().toISOString();
const browsers: ExtensionBrowser[] = [
  { browser: "chrome", label: "Google Chrome", installed: true, executable: "C:/chrome.exe", package: "chromium", extensionsPage: "chrome://extensions/", client: { browser: "chrome", version: "2.0.0", lastSeen: now } },
  { browser: "edge", label: "Microsoft Edge", installed: true, executable: "C:/msedge.exe", package: "chromium", extensionsPage: "edge://extensions/", client: null },
  { browser: "brave", label: "Brave", installed: false, executable: null, package: "chromium", extensionsPage: "brave://extensions/", client: null },
  { browser: "firefox", label: "Mozilla Firefox", installed: true, executable: "C:/firefox.exe", package: "firefox", extensionsPage: "about:debugging#/runtime/this-firefox", client: null },
];

describe("ExtensionsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.getExtensionBrowsers).mockResolvedValue(browsers);
  });

  it("computes connection state", () => {
    expect(connectionState(browsers[0])).toBe("connected");
    expect(connectionState({ installed: true, client: { browser: "x", version: "1", lastSeen: "2020-01-01T00:00:00Z" } })).toBe("seen");
    expect(connectionState(browsers[1])).toBe("installed");
    expect(connectionState(browsers[2])).toBe("missing");
  });

  it("shows a card per browser with its status", async () => {
    render(<ExtensionsPage />);
    const cards = await screen.findAllByRole("listitem");
    expect(cards).toHaveLength(4);
    expect(within(cards[0]).getByText("Google Chrome")).toBeInTheDocument();
    expect(within(cards[0]).getByText("Connected · v2.0.0")).toBeInTheDocument();
    expect(within(cards[1]).getByText("Browser found on this computer")).toBeInTheDocument();
    expect(within(cards[2]).getByText("Browser not found")).toBeInTheDocument();
  });

  it("gets the extension and shows install steps for Chromium browsers", async () => {
    vi.mocked(api.getExtension).mockResolvedValue({ folder: "/data/extension/chromium", archive: "/dl/pixidl-extension-edge.zip", extensionsPage: "edge://extensions/" });
    render(<ExtensionsPage />);
    const cards = await screen.findAllByRole("listitem");
    await userEvent.click(within(cards[1]).getByRole("button", { name: /Get extension/ }));
    expect(api.getExtension).toHaveBeenCalledWith("edge");
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("/dl/pixidl-extension-edge.zip")).toBeInTheDocument();
    expect(within(dialog).getByText(/Load unpacked/)).toBeInTheDocument();
    expect(within(dialog).getByText("/data/extension/chromium")).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: "Open in Microsoft Edge" }));
    expect(api.openBrowserExtensionsPage).toHaveBeenCalledWith("edge");
  });

  it("explains the Firefox limitation honestly", async () => {
    vi.mocked(api.getExtension).mockResolvedValue({ folder: "/data/extension/firefox", archive: "/dl/pixidl-extension-firefox.xpi", extensionsPage: "about:debugging#/runtime/this-firefox" });
    render(<ExtensionsPage />);
    const cards = await screen.findAllByRole("listitem");
    await userEvent.click(within(cards[3]).getByRole("button", { name: /Get extension/ }));
    const dialog = await screen.findByRole("dialog");
    await waitFor(() => expect(within(dialog).getByText(/Load Temporary Add-on/)).toBeInTheDocument());
    expect(within(dialog).getByText(/signed by Mozilla/)).toBeInTheDocument();
  });
});
