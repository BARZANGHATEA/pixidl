import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DownloadsPage } from "./DownloadsPage";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { useDownloads } from "../stores/downloads";
import { useQueues } from "../stores/queues";
import { useSelection } from "../stores/selection";
import { useUi } from "../stores/ui";
import { makeDownload } from "../test/fixtures";
import { api } from "../services/api";
import type { Download, Queue } from "../types";

vi.mock("../services/api", () => {
  const ok = () => vi.fn(() => Promise.resolve());
  return {
    api: {
      pauseAll: ok(), resumeAll: ok(), resumeMany: ok(), pauseMany: ok(), removeMany: ok(), moveToQueue: ok(),
      clearCompleted: vi.fn(() => Promise.resolve(1)), startQueue: ok(), stopQueue: ok(),
    },
  };
});

const queue = (id: string, name: string, running = true): Queue => ({ id, name, maxConcurrent: 2, running, sortOrder: 0, createdAt: "" });

let alpha: Download, beta: Download, gamma: Download, delta: Download;

function seed() {
  alpha = makeDownload({ filename: "alpha.zip", status: "downloading" });
  beta = makeDownload({ filename: "beta.mp4", status: "completed", engine: "video", category: "Videos" });
  gamma = makeDownload({ filename: "gamma.exe", status: "paused" });
  delta = makeDownload({ filename: "delta.iso", status: "completed", queueId: "night" });
  const list = [alpha, beta, gamma, delta];
  useDownloads.setState({ byId: Object.fromEntries(list.map((d) => [d.id, d])), loaded: true });
  useQueues.setState({ queues: [queue("main", ""), queue("night", "Night")] });
}

const row = (name: string) => screen.getByRole("listitem", { name: new RegExp(`^${name.replace(".", "\\.")}`) });
const toolbar = () => within(screen.getByRole("toolbar", { name: "Selected downloads" }));

function renderPage() {
  return render(
    <>
      <DownloadsPage />
      <ConfirmDialog />
    </>,
  );
}

describe("DownloadsPage selection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUi.setState({ view: "downloads", scope: { kind: "all" }, status: "all", query: "", sort: "name", confirm: null });
    useSelection.getState().clear();
    seed();
  });

  it("selects with checkboxes, Ctrl+click and Shift+click, then starts and pauses the selection", async () => {
    const user = userEvent.setup();
    renderPage();
    // Order by name: alpha, beta, delta, gamma.
    await user.click(screen.getByRole("checkbox", { name: "Select alpha.zip" }));
    expect(toolbar().getByText("1 selected")).toBeInTheDocument();
    expect(row("alpha.zip")).toHaveClass("selected");

    await user.keyboard("{Control>}");
    await user.click(row("gamma.exe"));
    await user.keyboard("{/Control}");
    expect(toolbar().getByText("2 selected")).toBeInTheDocument();

    await user.click(toolbar().getByRole("button", { name: "Start" }));
    expect(api.resumeMany).toHaveBeenCalledWith([alpha.id, gamma.id]);

    // Shift+click from the anchor (gamma) up to beta selects the range beta..gamma.
    await user.keyboard("{Shift>}");
    await user.click(row("beta.mp4"));
    await user.keyboard("{/Shift}");
    expect(toolbar().getByText("4 selected")).toBeInTheDocument();
    await user.click(toolbar().getByRole("button", { name: "Pause" }));
    expect(api.pauseMany).toHaveBeenCalledWith([alpha.id, beta.id, delta.id, gamma.id]);

    await user.click(toolbar().getByRole("button", { name: "Clear selection" }));
    expect(screen.queryByText(/selected$/)).not.toBeInTheDocument();
  });

  it("a plain click toggles rows once something is selected", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(row("alpha.zip"));
    expect(useSelection.getState().ids.size).toBe(0);
    await user.click(screen.getByRole("checkbox", { name: "Select alpha.zip" }));
    await user.click(row("beta.mp4"));
    expect([...useSelection.getState().ids].sort()).toEqual([alpha.id, beta.id].sort());
  });

  it("Ctrl+A selects all visible, Escape clears, the header checkbox toggles all", async () => {
    const user = userEvent.setup();
    useUi.setState({ status: "completed" });
    renderPage();
    await user.keyboard("{Control>}a{/Control}");
    expect([...useSelection.getState().ids].sort()).toEqual([beta.id, delta.id].sort());
    await user.keyboard("{Escape}");
    expect(useSelection.getState().ids.size).toBe(0);
    await user.click(screen.getByRole("checkbox", { name: "Select all" }));
    expect(useSelection.getState().ids.size).toBe(2);
    await user.click(screen.getByRole("checkbox", { name: "Select all" }));
    expect(useSelection.getState().ids.size).toBe(0);
  });

  it("moves the selection to another queue", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(screen.getByRole("checkbox", { name: "Select alpha.zip" }));
    await user.click(screen.getByRole("checkbox", { name: "Select gamma.exe" }));
    await user.click(toolbar().getByRole("button", { name: /Move to queue/ }));
    const menu = within(screen.getByRole("menu"));
    expect(menu.getByRole("menuitem", { name: "Main queue" })).toBeDisabled();
    await user.click(menu.getByRole("menuitem", { name: "Night" }));
    expect(api.moveToQueue).toHaveBeenCalledWith([alpha.id, gamma.id], "night");
  });

  it("Delete asks before removing, with an option to delete files", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(screen.getByRole("checkbox", { name: "Select beta.mp4" }));
    await user.click(screen.getByRole("checkbox", { name: "Select alpha.zip" }));
    await user.keyboard("{Delete}");
    const dialog = within(screen.getByRole("dialog"));
    expect(dialog.getByText(/2 downloads will be removed/)).toBeInTheDocument();
    expect(api.removeMany).not.toHaveBeenCalled();
    await user.click(dialog.getByRole("checkbox", { name: "Also delete the downloaded files from disk" }));
    await user.click(dialog.getByRole("button", { name: "Remove" }));
    expect(api.removeMany).toHaveBeenCalledWith([alpha.id, beta.id], true);
  });

  it("drops ids that disappear from the list", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(screen.getByRole("checkbox", { name: "Select alpha.zip" }));
    await user.click(screen.getByRole("checkbox", { name: "Select gamma.exe" }));
    act(() => useDownloads.getState().applyEvent({ type: "download_removed", id: gamma.id }));
    expect([...useSelection.getState().ids]).toEqual([alpha.id]);
    expect(toolbar().getByText("1 selected")).toBeInTheDocument();
  });
});

describe("Clear completed", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUi.setState({ view: "downloads", scope: { kind: "all" }, status: "all", query: "", sort: "name", confirm: null });
    useSelection.getState().clear();
    seed();
  });

  it("asks, then clears every completed download", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(screen.getByRole("button", { name: "Clear completed" }));
    const dialog = within(screen.getByRole("dialog"));
    expect(dialog.getByText(/2 completed downloads will be removed/)).toBeInTheDocument();
    await user.click(dialog.getByRole("button", { name: "Clear" }));
    expect(api.clearCompleted).toHaveBeenCalled();
  });

  it("in a queue only clears that queue's completed downloads", async () => {
    const user = userEvent.setup();
    useUi.setState({ scope: { kind: "queue", id: "night" }, status: "completed" });
    renderPage();
    expect(screen.getByRole("heading", { name: "Night" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Clear completed" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Clear" }));
    expect(api.removeMany).toHaveBeenCalledWith([delta.id], false);
    expect(api.clearCompleted).not.toHaveBeenCalled();
  });

  it("is disabled when nothing is completed", () => {
    const d = makeDownload({ status: "downloading" });
    useDownloads.setState({ byId: { [d.id]: d } });
    renderPage();
    expect(screen.getByRole("button", { name: "Clear completed" })).toBeDisabled();
  });
});
