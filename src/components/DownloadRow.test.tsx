import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DownloadRow } from "./DownloadRow";
import { ConfirmDialog } from "./ConfirmDialog";
import { makeDownload } from "../test/fixtures";
import { api } from "../services/api";

vi.mock("../services/api", () => ({
  api: { pause: vi.fn(() => Promise.resolve()), resume: vi.fn(() => Promise.resolve()), retry: vi.fn(() => Promise.resolve()), openFolder: vi.fn(() => Promise.resolve()), openFile: vi.fn(() => Promise.resolve()), remove: vi.fn(() => Promise.resolve()), cancel: vi.fn(() => Promise.resolve()) },
}));

describe("DownloadRow", () => {
  beforeEach(() => vi.clearAllMocks());

  it("downloading: shows real progress, speed, ETA and a pause action", async () => {
    const d = makeDownload({ filename: "ubuntu.iso", downloadedBytes: 2.4 * 1024 ** 3, totalBytes: 3.8 * 1024 ** 3, speedBps: 8.4 * 1024 * 1024, etaSeconds: 134 });
    render(<DownloadRow d={d} />);
    expect(screen.getByText("ubuntu.iso")).toBeInTheDocument();
    expect(screen.getByText("2.4 GB / 3.8 GB")).toBeInTheDocument();
    expect(screen.getByText("63%")).toBeInTheDocument();
    expect(screen.getByText("8.4 MB/s")).toBeInTheDocument();
    expect(screen.getByText("2m 14s")).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "63");
    await userEvent.click(screen.getByRole("button", { name: "Pause: ubuntu.iso" }));
    expect(api.pause).toHaveBeenCalledWith(d.id);
  });

  it("paused: resume action", async () => {
    const d = makeDownload({ status: "paused", speedBps: 0 });
    render(<DownloadRow d={d} />);
    expect(screen.getByText("Paused")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: `Resume: ${d.filename}` }));
    expect(api.resume).toHaveBeenCalledWith(d.id);
  });

  it("failed: reason and retry", async () => {
    const d = makeDownload({ status: "failed", errorKind: "network_unavailable", errorMessage: "Network unavailable", errorDetail: "connect error" });
    render(<DownloadRow d={d} />);
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("Network unavailable")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: `Retry: ${d.filename}` }));
    expect(api.retry).toHaveBeenCalledWith(d.id);
  });

  it("completed: 100%, open folder, and open file in the menu", async () => {
    const d = makeDownload({ status: "completed", downloadedBytes: 1000, totalBytes: 1000, completedAt: new Date().toISOString() });
    render(<DownloadRow d={d} />);
    expect(screen.getByText("100%")).toBeInTheDocument();
    expect(screen.getByText("Completed")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: `Open folder: ${d.filename}` }));
    expect(api.openFolder).toHaveBeenCalledWith(d.id);
    await userEvent.click(screen.getByRole("button", { name: `More actions: ${d.filename}` }));
    await userEvent.click(within(screen.getByRole("menu")).getByRole("menuitem", { name: "Open file" }));
    expect(api.openFile).toHaveBeenCalledWith(d.id);
  });

  it("preparing with unknown size shows no percentage", () => {
    const d = makeDownload({ status: "preparing", totalBytes: null, downloadedBytes: 0, engine: "torrent" });
    render(<DownloadRow d={d} />);
    expect(screen.getByText(/Fetching metadata/)).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).not.toHaveAttribute("aria-valuenow");
  });

  it("cancelled: restart", async () => {
    const d = makeDownload({ status: "cancelled" });
    render(<DownloadRow d={d} />);
    expect(screen.getByText("Cancelled")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: `Restart: ${d.filename}` }));
    expect(api.retry).toHaveBeenCalledWith(d.id);
  });

  it("remove asks for confirmation and only deletes the file when ticked", async () => {
    const d = makeDownload({ status: "completed", totalBytes: 1000, downloadedBytes: 1000 });
    render(<><DownloadRow d={d} /><ConfirmDialog /></>);
    await userEvent.click(screen.getByRole("button", { name: `More actions: ${d.filename}` }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Remove" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText(/will be removed from the list/)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("checkbox"));
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove" }));
    expect(api.remove).toHaveBeenCalledWith(d.id, true);
  });
});
