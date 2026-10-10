import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { UpdatesPage } from "./UpdatesPage";
import { api } from "../services/api";
import { useUpdates } from "../stores/updates";
import { useSettings } from "../stores/settings";
import type { ReleaseInfo, Settings, UpdateInfo, UpdateStatus } from "../types";

vi.mock("../services/api", async (orig) => {
  const real = await orig<typeof import("../services/api")>();
  return {
    ...real,
    api: {
      getUpdateStatus: vi.fn(),
      checkForUpdates: vi.fn(),
      downloadUpdate: vi.fn(),
      cancelUpdateDownload: vi.fn(() => Promise.resolve(true)),
      installUpdate: vi.fn(() => Promise.resolve()),
      openProjectPage: vi.fn(() => Promise.resolve()),
      updateSettings: vi.fn(() => Promise.resolve([])),
      getSettings: vi.fn(),
    },
  };
});

const REPO = "https://github.com/BARZANGHATEA/pixidl";

const release: ReleaseInfo = {
  version: "1.2.0",
  tag: "v1.2.0",
  name: "pixidl v1.2.0",
  notes: "## What's new\n- Faster **downloads**\n- <script>alert(1)</script> fixed\n\nSee [the docs](https://example.com).",
  publishedAt: "2026-10-01T10:00:00Z",
  htmlUrl: `${REPO}/releases/tag/v1.2.0`,
  prerelease: false,
  installer: { name: "pixidl_1.2.0_x64-setup.exe", url: `${REPO}/releases/download/v1.2.0/pixidl_1.2.0_x64-setup.exe`, size: 12 * 1024 * 1024 },
  sha256: "a".repeat(64),
};

function status(info: UpdateInfo | null, over: Partial<UpdateStatus> = {}): UpdateStatus {
  return { currentVersion: "1.1.0", info, lastChecked: info ? new Date().toISOString() : null, downloading: false, readyVersion: null, canInstall: true, repoUrl: REPO, releasesUrl: `${REPO}/releases`, ...over };
}

const available: UpdateInfo = { current: "1.1.0", latest: release, updateAvailable: true, installBlocker: null };

function setup(s: UpdateStatus) {
  vi.mocked(api.getUpdateStatus).mockResolvedValue(s);
  useUpdates.setState({ status: s, phase: "idle", progress: null, error: null });
}

describe("UpdatesPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useSettings.setState({ settings: { autoCheckUpdates: true, includePrereleases: false } as Settings });
  });

  it("shows the current version and checks on request", async () => {
    setup(status(null));
    vi.mocked(api.checkForUpdates).mockResolvedValue(status({ current: "1.1.0", latest: { ...release, version: "1.1.0" }, updateAvailable: false, installBlocker: null }));
    render(<UpdatesPage />);
    expect(await screen.findByText("Installed version 1.1.0")).toBeInTheDocument();
    expect(screen.getByText("Not checked yet")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(api.checkForUpdates).toHaveBeenCalled();
    expect(await screen.findByText("pixidl is up to date")).toBeInTheDocument();
    expect(screen.getByText("The newest published version is 1.1.0.")).toBeInTheDocument();
  });

  it("shows a checking state", async () => {
    setup(status(null));
    useUpdates.setState({ phase: "checking" });
    render(<UpdatesPage />);
    expect(screen.getByText("Checking for updates…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check for updates" })).toBeDisabled();
  });

  it("explains that nothing has been released yet", async () => {
    setup(status({ current: "1.1.0", latest: null, updateAvailable: false, installBlocker: null }));
    render(<UpdatesPage />);
    expect(await screen.findByText("No releases have been published yet")).toBeInTheDocument();
    await userEvent.click(screen.getAllByRole("button", { name: /Open releases page/ })[0]);
    expect(api.openProjectPage).toHaveBeenCalledWith(`${REPO}/releases`);
  });

  it("shows an available update with safe release notes", async () => {
    setup(status(available));
    const { container } = render(<UpdatesPage />);
    expect(await screen.findByText("pixidl 1.2.0 is available")).toBeInTheDocument();
    expect(screen.getByText(/Installer 12.0 MB/)).toBeInTheDocument();
    expect(screen.getByText("What's new")).toBeInTheDocument();
    expect(screen.getByText("Faster downloads")).toBeInTheDocument();
    expect(screen.getByText("See the docs.")).toBeInTheDocument();
    // Raw HTML from the release is shown as text, never injected.
    expect(screen.getByText("<script>alert(1)</script> fixed")).toBeInTheDocument();
    expect(container.querySelector("script")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: /View on GitHub/ }));
    expect(api.openProjectPage).toHaveBeenCalledWith(release.htmlUrl);
  });

  it("downloads with progress, can cancel, then installs", async () => {
    setup(status(available));
    let finish: () => void = () => {};
    vi.mocked(api.downloadUpdate).mockReturnValue(new Promise<void>((r) => (finish = r)));
    render(<UpdatesPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Download and install/ }));
    expect(api.downloadUpdate).toHaveBeenCalled();
    act(() => useUpdates.getState().applyEvent({ type: "progress", downloaded: 6 * 1024 * 1024, total: 12 * 1024 * 1024, speedBps: 1024 * 1024 }));
    expect(screen.getByText("Downloading pixidl 1.2.0…")).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "50");
    expect(screen.getByText("6.0 MB of 12.0 MB · 1.0 MB/s")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(api.cancelUpdateDownload).toHaveBeenCalled();

    act(() => useUpdates.getState().applyEvent({ type: "verifying" }));
    expect(screen.getByText("Verifying the download…")).toBeInTheDocument();

    vi.mocked(api.getUpdateStatus).mockResolvedValue(status(available, { readyVersion: "1.2.0" }));
    await act(async () => finish());
    const installBtn = await screen.findByRole("button", { name: /Install and restart/ });
    expect(screen.getByText("pixidl 1.2.0 is ready to install")).toBeInTheDocument();
    await userEvent.click(installBtn);
    expect(api.installUpdate).toHaveBeenCalled();
  });

  it("returns to the available state after a cancelled download", async () => {
    setup(status(available));
    vi.mocked(api.downloadUpdate).mockRejectedValue({ kind: "cancelled", message: "Download cancelled", detail: null });
    render(<UpdatesPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Download and install/ }));
    expect(await screen.findByRole("button", { name: /Download and install/ })).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows errors with retry and the releases page", async () => {
    setup(status(available));
    vi.mocked(api.downloadUpdate).mockRejectedValueOnce({ kind: "checksum_mismatch", message: "The downloaded update failed checksum verification and was deleted", detail: "expected a, got b" });
    render(<UpdatesPage />);
    await userEvent.click(await screen.findByRole("button", { name: /Download and install/ }));
    expect(await screen.findByText("Could not download the update")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("failed checksum verification");
    vi.mocked(api.downloadUpdate).mockResolvedValueOnce();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(api.downloadUpdate).toHaveBeenCalledTimes(2);
  });

  it("shows a check error", async () => {
    setup(status(null));
    vi.mocked(api.checkForUpdates).mockRejectedValue({ kind: "server_rejected", message: "GitHub's request limit was reached. Try again in a while.", detail: null });
    render(<UpdatesPage />);
    await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(await screen.findByText("Could not check for updates")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("request limit");
    expect(screen.getAllByRole("button", { name: /Open releases page/ }).length).toBeGreaterThan(0);
  });

  it("links to the release page when the app can't install it (other platforms)", async () => {
    setup(status({ ...available, installBlocker: "unsupported_platform" }, { canInstall: false }));
    render(<UpdatesPage />);
    expect(await screen.findByText(/available on Windows only/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Download and install/ })).toBeNull();
    expect(screen.getByRole("button", { name: /View on GitHub/ })).toBeInTheDocument();
  });

  it("refuses to offer an unverifiable installer", async () => {
    setup(status({ ...available, latest: { ...release, sha256: null }, installBlocker: "no_checksum" }));
    render(<UpdatesPage />);
    expect(await screen.findByText(/no published checksum/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Download and install/ })).toBeNull();
  });

  it("binds the preferences to settings", async () => {
    setup(status(null));
    vi.mocked(api.getSettings).mockResolvedValue({ autoCheckUpdates: false, includePrereleases: false } as Settings);
    render(<UpdatesPage />);
    const auto = screen.getByRole("switch", { name: "Check for updates automatically" });
    expect(auto).toBeChecked();
    await userEvent.click(auto);
    await waitFor(() => expect(api.updateSettings).toHaveBeenCalledWith(expect.objectContaining({ autoCheckUpdates: false })));
    await userEvent.click(screen.getByRole("button", { name: /GitHub repository/ }));
    expect(api.openProjectPage).toHaveBeenCalledWith(REPO);
  });

  it("marks the sidebar entry when an update is available", async () => {
    const { Sidebar } = await import("../components/Sidebar");
    setup(status(available));
    render(<Sidebar />);
    expect(screen.getByRole("status", { name: "Update available" })).toBeInTheDocument();
    act(() => useUpdates.setState({ status: status({ ...available, updateAvailable: false }) }));
    expect(screen.queryByRole("status", { name: "Update available" })).toBeNull();
  });
});
