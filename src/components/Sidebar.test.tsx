import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Sidebar } from "./Sidebar";
import { ConfirmDialog } from "./ConfirmDialog";
import { QueueDialog } from "./QueueDialog";
import { useDownloads } from "../stores/downloads";
import { useQueues } from "../stores/queues";
import { useUi } from "../stores/ui";
import { makeDownload } from "../test/fixtures";
import { api } from "../services/api";
import type { Queue } from "../types";

vi.mock("../services/api", () => {
  const ok = () => vi.fn(() => Promise.resolve());
  return {
    api: {
      createQueue: vi.fn(() => Promise.resolve()), renameQueue: ok(), setQueueMaxConcurrent: ok(), deleteQueue: ok(), startQueue: ok(), stopQueue: ok(),
    },
  };
});

const queue = (id: string, name: string, running = true, maxConcurrent = 2): Queue => ({ id, name, maxConcurrent, running, sortOrder: 0, createdAt: "" });

function renderAll() {
  return render(
    <>
      <Sidebar />
      <QueueDialog />
      <ConfirmDialog />
    </>,
  );
}

describe("Sidebar queues", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUi.setState({ view: "settings", scope: { kind: "all" }, status: "all", confirm: null, queueDialog: null });
    useQueues.setState({ queues: [queue("main", ""), queue("night", "Night", false)] });
    const list = [makeDownload({ queueId: "main" }), makeDownload({ queueId: "night", status: "paused" }), makeDownload({ queueId: "night", status: "queued" })];
    useDownloads.setState({ byId: Object.fromEntries(list.map((d) => [d.id, d])), loaded: true });
  });

  it("lists queues with counts and selects one as the scope", async () => {
    const user = userEvent.setup();
    renderAll();
    const group = within(screen.getByRole("group", { name: "Queues" }));
    const main = group.getByRole("button", { name: /^Main queue/ });
    const night = group.getByRole("button", { name: /^Night/ });
    expect(within(main).getByText("1")).toBeInTheDocument();
    expect(within(night).getByText("2")).toBeInTheDocument();
    expect(within(night).getByText("Stopped")).toBeInTheDocument();
    await user.click(night);
    expect(useUi.getState().scope).toEqual({ kind: "queue", id: "night" });
    expect(useUi.getState().view).toBe("downloads");
    expect(night).toHaveAttribute("aria-current", "true");
  });

  it("creates a queue, rejecting a duplicate name", async () => {
    const user = userEvent.setup();
    renderAll();
    await user.click(screen.getByRole("button", { name: "New queue" }));
    const dialog = within(screen.getByRole("dialog", { name: "New queue" }));
    await user.type(dialog.getByLabelText("Name"), "night");
    await user.click(dialog.getByRole("button", { name: "Create" }));
    expect(dialog.getByText("A queue with this name already exists.")).toBeInTheDocument();
    expect(api.createQueue).not.toHaveBeenCalled();
    await user.clear(dialog.getByLabelText("Name"));
    await user.type(dialog.getByLabelText("Name"), "Weekend");
    await user.clear(dialog.getByLabelText("Simultaneous downloads"));
    await user.type(dialog.getByLabelText("Simultaneous downloads"), "3");
    await user.click(dialog.getByRole("button", { name: "Create" }));
    expect(api.createQueue).toHaveBeenCalledWith("Weekend", 3);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("queue menu: start, change the limit, delete with confirmation", async () => {
    const user = userEvent.setup();
    useUi.setState({ scope: { kind: "queue", id: "night" } });
    renderAll();
    const more = screen.getByRole("button", { name: "Queue actions: Night" });

    await user.click(more);
    await user.click(screen.getByRole("menuitem", { name: "Start queue" }));
    expect(api.startQueue).toHaveBeenCalledWith("night");

    await user.click(more);
    await user.click(screen.getByRole("menuitem", { name: "Simultaneous downloads…" }));
    const dialog = within(screen.getByRole("dialog", { name: "Queue settings" }));
    expect(dialog.getByLabelText("Name")).toHaveValue("Night");
    await user.clear(dialog.getByLabelText("Simultaneous downloads"));
    await user.type(dialog.getByLabelText("Simultaneous downloads"), "40");
    await user.click(dialog.getByRole("button", { name: "Save" }));
    expect(api.setQueueMaxConcurrent).not.toHaveBeenCalled(); // out of range (1–20): the form does not submit
    await user.clear(dialog.getByLabelText("Simultaneous downloads"));
    await user.type(dialog.getByLabelText("Simultaneous downloads"), "5");
    await user.click(dialog.getByRole("button", { name: "Save" }));
    expect(api.setQueueMaxConcurrent).toHaveBeenCalledWith("night", 5);
    expect(api.renameQueue).not.toHaveBeenCalled();

    await user.click(more);
    await user.click(screen.getByRole("menuitem", { name: "Delete queue" }));
    const confirm = within(screen.getByRole("dialog"));
    expect(confirm.getByText(/Its 2 downloads move to the main queue/)).toBeInTheDocument();
    await user.click(confirm.getByRole("button", { name: "Delete queue" }));
    expect(api.deleteQueue).toHaveBeenCalledWith("night");
    await vi.waitFor(() => expect(useUi.getState().scope).toEqual({ kind: "all" }));
  });

  it("the main queue cannot be deleted and keeps its translated name", async () => {
    const user = userEvent.setup();
    renderAll();
    await user.click(screen.getByRole("button", { name: "Queue actions: Main queue" }));
    const menu = within(screen.getByRole("menu"));
    expect(menu.queryByRole("menuitem", { name: "Delete queue" })).not.toBeInTheDocument();
    await user.click(menu.getByRole("menuitem", { name: "Rename…" }));
    const dialog = within(screen.getByRole("dialog", { name: "Queue settings" }));
    expect(dialog.getByLabelText("Name")).toHaveValue("Main queue");
    await user.click(dialog.getByRole("button", { name: "Save" }));
    expect(api.renameQueue).not.toHaveBeenCalled();
  });
});
